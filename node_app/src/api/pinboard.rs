//! Pinboard REST (submit + list + get-by-path).

use super::api_rate_limiting::{
    check_rate_limit, extract_client_id, ApiEndpointType, RateLimitState,
};
use super::error::ApiError;
use super::namespace;
use super::pagination::{PaginationParams, PrefixQueryOptions};
use crate::app_state::AppState;
use crate::capacity::capacity_manager::CapacityManager;
use crate::config::ConsensusConfig;
use crate::storage::rocksdb::RocksDBStorage;
use crate::storage::traits::PinboardGlobalFeedOrder;
use axum::{
    extract::{self, State},
    http::HeaderMap,
    Json,
};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use eld_client::api::rest::{PostMessageSubmitRequest, PostMessageSubmitResponse};
use eld_client::facade::ChainClient;
use eld_common::account::Account;
use eld_common::cado::{CadoBody, CadoPath, CadoPathKey, CadoType};
use eld_common::capacity::slot_allocator::CONTENT_ID_NOT_IN_SLOT_MAP;
use eld_common::coin::Coin;
use eld_common::constants::{abci_query, cado};
use eld_common::error::EldError;
use eld_common::fee::calculate_dynamic_fee;
use eld_common::namespace::{resolve_optional_namespace, validate_namespace_upload_authorization};
use eld_common::pinboard::{
    parse_pinboard_namespace_content_path, path_uses_eld_pinboard_lookup,
    pinboard_namespace_content_path, pinboard_response_cado_path, PinboardMessageMetadata,
};
use eld_common::tx::{validate_post_message_user_signature, Payload, PostMessageTx, Tx, TxSig};
use eld_common::Address;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// Submit transaction to Tendermint RPC
async fn submit_transaction_to_tendermint(
    tendermint_rpc_url: &str,
    tx: &Tx,
) -> Result<(), ApiError> {
    // Serialize transaction to JSON
    let tx_json = serde_json::to_string(tx).map_err(|e| ApiError::InternalServerError {
        message: "Failed to serialize transaction".to_string(),
        details: Some(e.to_string()),
    })?;

    // Hex encode the JSON
    let tx_hex = hex::encode(tx_json.as_bytes());

    // Base64 encode the hex
    let tx_base64 = BASE64_STANDARD.encode(tx_hex.as_bytes());

    // Create JSON-RPC request body
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "broadcast_tx_sync",
        "params": [tx_base64]
    });

    // Send POST request to Tendermint RPC
    let client = reqwest::Client::new();
    let response = client
        .post(tendermint_rpc_url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| ApiError::InternalServerError {
            message: "Failed to send transaction to Tendermint".to_string(),
            details: Some(e.to_string()),
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        error!(
            "Tendermint RPC request failed with status {}: {}",
            status, error_text
        );
        return Err(ApiError::InternalServerError {
            message: format!("Tendermint RPC request failed with status: {status}"),
            details: Some(error_text),
        });
    }

    let response_text = response
        .text()
        .await
        .map_err(|e| ApiError::InternalServerError {
            message: "Failed to read Tendermint RPC response".to_string(),
            details: Some(e.to_string()),
        })?;

    // Parse response to check for errors
    let rpc_response: serde_json::Value =
        serde_json::from_str(&response_text).map_err(|e| ApiError::InternalServerError {
            message: "Failed to parse Tendermint RPC response".to_string(),
            details: Some(e.to_string()),
        })?;

    // Check for RPC error
    if let Some(error) = rpc_response.get("error") {
        let error_msg = error
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        error!("Tendermint RPC error: {}", error_msg);
        return Err(ApiError::InternalServerError {
            message: "Transaction submission failed".to_string(),
            details: Some(error_msg.to_string()),
        });
    }

    // Check result for CheckTx errors
    if let Some(result) = rpc_response.get("result") {
        if let Some(check_tx) = result.get("check_tx") {
            if let Some(code) = check_tx.get("code") {
                if let Some(code_value) = code.as_u64() {
                    if code_value != 0 {
                        let log = check_tx
                            .get("log")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown error");
                        error!(
                            "Transaction CheckTx failed with code {}: {}",
                            code_value, log
                        );
                        return Err(ApiError::InternalServerError {
                            message: "Transaction validation failed".to_string(),
                            details: Some(format!("CheckTx code: {code_value}, log: {log}")),
                        });
                    }
                }
            }
        }
    }

    info!("Transaction submitted successfully to Tendermint mempool");
    Ok(())
}

/// Pinboard: verify user-signed message, then broadcast validator-signed `PostMessage` tx.
pub(crate) async fn handle_pinboard_submit(
    State(storage): State<Arc<RocksDBStorage>>,
    State(consensus_config): State<Arc<Mutex<ConsensusConfig>>>,
    State(rate_limit_state): State<Arc<RwLock<RateLimitState>>>,
    State(cli): State<Arc<ChainClient>>,
    State(capacity_manager): State<Arc<CapacityManager>>,
    headers: HeaderMap,
    Json(body): Json<PostMessageSubmitRequest>,
) -> Result<Json<PostMessageSubmitResponse>, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::Upload, &headers).await?;

    const MAX_PINBOARD_MESSAGE_BYTES: usize = 1024 * 1024;
    let message_bytes = BASE64_STANDARD
        .decode(body.message_b64.as_bytes())
        .map_err(|e| ApiError::BadRequest {
            message: "invalid_base64_message".to_string(),
            details: Some(e.to_string()),
        })?;
    if message_bytes.is_empty() {
        return Err(ApiError::BadRequest {
            message: "empty_message".to_string(),
            details: Some("message_b64 decodes to empty bytes".to_string()),
        });
    }
    if message_bytes.len() > MAX_PINBOARD_MESSAGE_BYTES {
        return Err(ApiError::PayloadTooLarge {
            max_size: MAX_PINBOARD_MESSAGE_BYTES,
            actual_size: message_bytes.len(),
            details: Some("Pinboard message exceeds maximum size".to_string()),
        });
    }

    validate_post_message_user_signature(&message_bytes, &body.user).map_err(|e| {
        ApiError::BadRequest {
            message: "invalid_user_signature".to_string(),
            details: Some(e),
        }
    })?;

    let original_signer_addr =
        Address::parse_hex_str(&body.user.original_signer).map_err(|e| ApiError::BadRequest {
            message: "invalid_original_signer".to_string(),
            details: Some(e.to_string()),
        })?;

    validate_namespace_upload_authorization(
        body.user.namespace.clone(),
        original_signer_addr,
        |slug| namespace::lookup_namespace_record_from_storage(storage.as_ref(), &slug),
    )
    .map_err(|e| ApiError::BadRequest {
        message: "invalid_namespace".to_string(),
        details: Some(e.to_string()),
    })?;

    let wallet_name = std::env::var("ELD_POST_MESSAGE_VALIDATOR_WALLET")
        .unwrap_or_else(|_| "wallet1".to_string());
    let validator_wallet = cli
        .get_wallet_by_name(wallet_name.clone())
        .await
        .map_err(|e| ApiError::ServiceUnavailable {
            message: "validator_wallet_missing".to_string(),
            details: Some(e.to_string()),
        })?
        .ok_or_else(|| ApiError::ServiceUnavailable {
            message: "validator_wallet_missing".to_string(),
            details: Some(format!(
                "No wallet {wallet_name:?} in wallets.json (ELD_POST_MESSAGE_VALIDATOR_WALLET for Eld)"
            )),
        })?;

    let (chain_id, fee_config) = {
        let cfg = consensus_config
            .lock()
            .map_err(|e| ApiError::InternalServerError {
                message: "consensus_config_lock".to_string(),
                details: Some(e.to_string()),
            })?;
        (cfg.chain_id.clone(), cfg.fee_config.clone())
    };

    let received_timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| ApiError::InternalServerError {
            message: "system_time".to_string(),
            details: Some(e.to_string()),
        })?
        .as_secs();

    const PINBOARD_TEMP_BLOB_TTL_SECS: u64 = 15 * 60;
    let confirmed_blob_exists =
        load_pinboard_blob_from_capacity_slots(capacity_manager.as_ref(), &body.user.content_key)
            .await?
            .is_some();
    if !confirmed_blob_exists {
        storage
            .put_pinboard_temp_blob(
                &body.user.content_key,
                &message_bytes,
                received_timestamp.saturating_add(PINBOARD_TEMP_BLOB_TTL_SECS),
            )
            .map_err(|e| ApiError::InternalServerError {
                message: "pinboard_temp_blob_store_failed".to_string(),
                details: Some(e.to_string()),
            })?;
    }

    let validator_address = validator_wallet.address;
    let sender_path = CadoPath::new(CadoType::Account, CadoPathKey::Address(validator_address))
        .map_err(|e| ApiError::InternalServerError {
            message: "account_path".to_string(),
            details: Some(e.to_string()),
        })?;

    let sender_account: Account = match storage.get_cado_by_path(sender_path.clone()) {
        Ok(Some(CadoBody::Mutable(cado_mut))) => Account::deserialize_bin(cado_mut.data())
            .map_err(|e| ApiError::InternalServerError {
                message: "account_deserialize".to_string(),
                details: Some(e.to_string()),
            })?,
        Ok(Some(_)) => {
            return Err(ApiError::InternalServerError {
                message: "invalid_account_cado".to_string(),
                details: None,
            });
        }
        Ok(None) => {
            return Err(ApiError::BadRequest {
                message: "validator_account_not_found".to_string(),
                details: Some(format!(
                    "Validator account {validator_address} is not on chain; fund it before posting"
                )),
            });
        }
        Err(e) => {
            return Err(ApiError::InternalServerError {
                message: "account_lookup_failed".to_string(),
                details: Some(e.to_string()),
            });
        }
    };

    let validator_tx_nonce = sender_account
        .nonce()
        .next()
        .ok_or_else(|| ApiError::InternalServerError {
            message: "nonce_overflow".to_string(),
            details: Some("Validator account nonce overflow".to_string()),
        })?
        .value();

    let post_inner = PostMessageTx::new(
        validator_address,
        original_signer_addr,
        body.user.original_signer_pubkey.clone(),
        body.user.content_key.clone(),
        body.user.message_id.clone(),
        body.user.expires_height,
        body.user.visibility.clone(),
        body.user.topic.clone(),
        body.user.tags.clone(),
        body.user.content_type.clone(),
        body.user.fee_amount,
        received_timestamp,
        body.user.user_signature.clone(),
        body.user.namespace.clone(),
    )
    .map_err(|e| ApiError::BadRequest {
        message: "invalid_post_message_payload".to_string(),
        details: Some(e.to_string()),
    })?;

    let mut tx_for_fee = Tx::new(
        validator_tx_nonce,
        Payload::new(post_inner),
        validator_wallet.verifying_key(),
    );
    tx_for_fee.sig = TxSig::new("0".repeat(128)).map_err(|e| ApiError::InternalServerError {
        message: "invalid_dummy_signature".to_string(),
        details: Some(e.to_string()),
    })?;

    let dynamic_fee = calculate_dynamic_fee(&tx_for_fee, &fee_config).map_err(|e| {
        ApiError::InternalServerError {
            message: "fee_calculation_failed".to_string(),
            details: Some(e.to_string()),
        }
    })?;
    let fee_buffer = Coin::new(200).map_err(|e| ApiError::InternalServerError {
        message: "fee_buffer".to_string(),
        details: Some(e.to_string()),
    })?;
    let fee_with_buffer = dynamic_fee.checked_add(fee_buffer).unwrap_or(dynamic_fee);

    if sender_account.balance() < fee_with_buffer {
        return Err(ApiError::BadRequest {
            message: "validator_insufficient_balance_for_fee".to_string(),
            details: Some(format!(
                "balance {} required_at_least {}",
                sender_account.balance(),
                fee_with_buffer
            )),
        });
    }

    let signed_tx = validator_wallet
        .sign_post_message_chain_tx(
            &message_bytes,
            &body.user,
            validator_tx_nonce,
            received_timestamp,
            chain_id.as_str(),
            fee_with_buffer.amount(),
        )
        .map_err(|e| ApiError::BadRequest {
            message: "build_post_message_tx_failed".to_string(),
            details: Some(e),
        })?;

    let tx_json = serde_json::to_string(&signed_tx).map_err(|e| ApiError::InternalServerError {
        message: "tx_serialize".to_string(),
        details: Some(e.to_string()),
    })?;
    let mut hasher = Sha256::new();
    hasher.update(tx_json.as_bytes());
    let tx_hash = hex::encode(hasher.finalize());

    let tendermint_rpc_url = std::env::var("TENDERMINT_RPC_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:26657".to_string());
    submit_transaction_to_tendermint(&tendermint_rpc_url, &signed_tx).await?;

    let content_path = match resolve_optional_namespace(body.user.namespace.clone()) {
        Ok(Some(ns)) => Some(pinboard_namespace_content_path(&ns, &body.user.message_id)),
        Ok(None) => None,
        Err(e) => {
            return Err(ApiError::BadRequest {
                message: "invalid_namespace".to_string(),
                details: Some(e.to_string()),
            });
        }
    };

    Ok(Json(PostMessageSubmitResponse::submitted(
        body.user.message_id.clone(),
        body.user.content_key.clone(),
        tx_hash,
        validator_address.hex_with_prefix(),
        received_timestamp,
        content_path,
    )))
}

#[derive(Debug, Deserialize)]
pub(crate) struct PinboardPostsQueryParams {
    order: Option<PinboardGlobalFeedOrder>,
    page: Option<usize>,
    page_size: Option<usize>,
    wallet: Option<String>,
    include_expired: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PinboardPostByPathQueryParams {
    path: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct PinboardRestPagination {
    page: usize,
    page_size: usize,
    has_more: bool,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PinboardRestPostItem {
    cado_path: String,
    meta: PinboardMessageMetadata,
    message_b64: Option<String>,
    blob_status: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct PinboardPostsResponse {
    items: Vec<PinboardRestPostItem>,
    pagination: PinboardRestPagination,
}

fn parse_pinboard_post_cado_path(path: &str) -> Result<(String, String), ApiError> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() != 5
        || parts[0] != cado::SCOPE_ELD_ROOT
        || parts[1] != abci_query::PINBOARD
        || parts[2] != abci_query::PINBOARD_SEGMENT_POST
    {
        return Err(ApiError::BadRequest {
            message: "Invalid pinboard post path".to_string(),
            details: Some(format!(
                "Expected {}/{}/{{wallet}}/{{message_id}}",
                cado::PATH_PREFIX_PINBOARD,
                abci_query::PINBOARD_SEGMENT_POST
            )),
        });
    }

    let wallet = Address::parse_hex_str(parts[3]).map_err(|e| ApiError::BadRequest {
        message: "Invalid wallet in path".to_string(),
        details: Some(e.to_string()),
    })?;
    if parts[4].is_empty() {
        return Err(ApiError::BadRequest {
            message: "Invalid pinboard post path".to_string(),
            details: Some("message_id cannot be empty".to_string()),
        });
    }

    Ok((wallet.hex_with_prefix(), parts[4].to_string()))
}

/// Last committed block height from in-memory app state (no RocksDB read).
fn current_committed_height(committed_state: &AppState) -> u64 {
    committed_state.envelope.block_height.max(0) as u64
}

async fn load_pinboard_blob_from_capacity_slots(
    capacity_manager: &CapacityManager,
    content_key: &str,
) -> Result<Option<Vec<u8>>, ApiError> {
    match capacity_manager.get_content_from_slots(content_key).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(EldError::NotFoundError { resource_type, .. })
            if resource_type == CONTENT_ID_NOT_IN_SLOT_MAP =>
        {
            Ok(None)
        }
        Err(e) => Err(ApiError::InternalServerError {
            message: e.to_string(),
            details: None,
        }),
    }
}

fn pinboard_blob_fields(
    meta: &PinboardMessageMetadata,
    current_height: u64,
    blob_bytes: Option<Vec<u8>>,
) -> (Option<String>, String) {
    if current_height >= meta.expires_height {
        return (None, "expired".to_string());
    }

    match blob_bytes {
        Some(bytes) => (Some(BASE64_STANDARD.encode(bytes)), "available".to_string()),
        None => (None, "missing".to_string()),
    }
}

#[allow(dead_code)]
fn pinboard_post_cado_path(wallet: &str, message_id: &str) -> String {
    format!(
        "{}{}/{}/{}",
        cado::PATH_PREFIX_PINBOARD,
        abci_query::PINBOARD_SEGMENT_POST,
        wallet,
        message_id
    )
}

pub(crate) async fn handle_get_pinboard_posts(
    State(storage): State<Arc<RocksDBStorage>>,
    State(committed_state): State<Arc<Mutex<AppState>>>,
    State(rate_limit_state): State<Arc<RwLock<RateLimitState>>>,
    State(capacity_manager): State<Arc<CapacityManager>>,
    headers: HeaderMap,
    extract::Query(params): extract::Query<PinboardPostsQueryParams>,
) -> Result<Json<PinboardPostsResponse>, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::General, &headers).await?;

    let order = params.order.unwrap_or(PinboardGlobalFeedOrder::Desc);
    let page = params.page.unwrap_or(0);
    let page_size = params.page_size.unwrap_or(100);
    let include_expired = params.include_expired.unwrap_or(true);
    if page_size == 0 || page_size > 100 {
        return Err(ApiError::BadRequest {
            message: "Invalid page_size".to_string(),
            details: Some("page_size must be in range 1..=100".to_string()),
        });
    }

    let client_id = extract_client_id(&headers);
    let options = PrefixQueryOptions::default().with_pagination(PaginationParams {
        page,
        page_size,
        cursor: None,
    });
    let options = PrefixQueryOptions {
        client_id,
        ..options
    };

    let ids_page = if let Some(wallet_raw) = params.wallet.as_deref() {
        let wallet = Address::parse_hex_str(wallet_raw).map_err(|e| ApiError::BadRequest {
            message: "Invalid wallet".to_string(),
            details: Some(e.to_string()),
        })?;
        storage
            .get_pinboard_message_ids_by_wallet_secure(&wallet.hex_with_prefix(), options)
            .map_err(|e| ApiError::InternalServerError {
                message: "Failed to list wallet pinboard posts".to_string(),
                details: Some(e.to_string()),
            })?
    } else {
        storage
            .get_pinboard_message_ids_global_secure(order, options)
            .map_err(|e| ApiError::InternalServerError {
                message: "Failed to list pinboard posts".to_string(),
                details: Some(e.to_string()),
            })?
    };

    let current_height = {
        let committed = committed_state
            .lock()
            .map_err(|_| ApiError::InternalServerError {
                message: "Failed to read committed application state".to_string(),
                details: None,
            })?;
        current_committed_height(&committed)
    };
    let mut items = Vec::new();
    for message_id in ids_page.items {
        let Some(meta) = storage.get_pinboard_metadata(&message_id).map_err(|e| {
            ApiError::InternalServerError {
                message: "Failed to load pinboard metadata".to_string(),
                details: Some(e.to_string()),
            }
        })?
        else {
            warn!(
                message_id = %message_id,
                "pinboard list index pointed to missing metadata"
            );
            continue;
        };

        let is_expired = current_height >= meta.expires_height;
        if !include_expired && is_expired {
            continue;
        }

        let blob_bytes = if is_expired {
            None
        } else {
            load_pinboard_blob_from_capacity_slots(capacity_manager.as_ref(), &meta.content_key)
                .await?
        };
        let (message_b64, blob_status) = pinboard_blob_fields(&meta, current_height, blob_bytes);
        items.push(PinboardRestPostItem {
            cado_path: pinboard_response_cado_path(&meta),
            meta,
            message_b64,
            blob_status,
        });
    }

    Ok(Json(PinboardPostsResponse {
        items,
        pagination: PinboardRestPagination {
            page: ids_page.page,
            page_size: ids_page.page_size,
            has_more: ids_page.has_more,
            next_cursor: ids_page.next_cursor,
        },
    }))
}

pub(crate) async fn handle_get_pinboard_post_by_path(
    State(storage): State<Arc<RocksDBStorage>>,
    State(committed_state): State<Arc<Mutex<AppState>>>,
    State(rate_limit_state): State<Arc<RwLock<RateLimitState>>>,
    State(capacity_manager): State<Arc<CapacityManager>>,
    headers: HeaderMap,
    extract::Query(params): extract::Query<PinboardPostByPathQueryParams>,
) -> Result<Json<PinboardRestPostItem>, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::General, &headers).await?;

    let meta =
        if path_uses_eld_pinboard_lookup(&params.path) {
            let (wallet, message_id) = parse_pinboard_post_cado_path(&params.path)?;
            let meta = storage
                .get_pinboard_metadata(&message_id)
                .map_err(|e| ApiError::InternalServerError {
                    message: "Failed to load pinboard metadata".to_string(),
                    details: Some(e.to_string()),
                })?
                .ok_or_else(|| ApiError::NotFound {
                    resource_type: "pinboard_message".to_string(),
                    identifier: message_id.clone(),
                    details: Some("Metadata not found".to_string()),
                })?;
            if meta.original_signer.hex_with_prefix() != wallet {
                return Err(ApiError::NotFound {
                    resource_type: "pinboard_message".to_string(),
                    identifier: format!("{wallet}/{message_id}"),
                    details: Some("Path wallet does not match message owner".to_string()),
                });
            }
            meta
        } else {
            let (expected_namespace, _) = parse_pinboard_namespace_content_path(&params.path)
                .map_err(|e| ApiError::BadRequest {
                    message: "Invalid pinboard namespace path".to_string(),
                    details: Some(e.to_string()),
                })?;
            let meta = storage
                .get_pinboard_metadata_by_path_key(&params.path)
                .map_err(|e| ApiError::InternalServerError {
                    message: "Failed to load pinboard metadata".to_string(),
                    details: Some(e.to_string()),
                })?
                .ok_or_else(|| ApiError::NotFound {
                    resource_type: "pinboard_message".to_string(),
                    identifier: params.path.clone(),
                    details: Some("Metadata not found for namespace path".to_string()),
                })?;
            match meta.namespace.as_deref() {
                Some(ns) if ns == expected_namespace => meta,
                _ => {
                    return Err(ApiError::NotFound {
                        resource_type: "pinboard_message".to_string(),
                        identifier: params.path.clone(),
                        details: Some("Namespace path does not match stored metadata".to_string()),
                    });
                }
            }
        };

    let current_height = {
        let committed = committed_state
            .lock()
            .map_err(|_| ApiError::InternalServerError {
                message: "Failed to read committed application state".to_string(),
                details: None,
            })?;
        current_committed_height(&committed)
    };
    let blob_bytes = if current_height >= meta.expires_height {
        None
    } else {
        load_pinboard_blob_from_capacity_slots(capacity_manager.as_ref(), &meta.content_key).await?
    };
    let (message_b64, blob_status) = pinboard_blob_fields(&meta, current_height, blob_bytes);

    Ok(Json(PinboardRestPostItem {
        cado_path: pinboard_response_cado_path(&meta),
        meta,
        message_b64,
        blob_status,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capacity::capacity_manager::CapacityManager;
    use crate::storage::traits::PinboardStorage;
    use eld_common::capacity::CapacityConfig;
    use eld_common::constants::{abci_query, cado};
    use eld_common::pinboard::{pinboard_eld_post_cado_path, PinboardMessageMetadata};
    use eld_common::Address;

    #[test]
    fn test_current_committed_height_from_app_state() {
        let mut state = AppState::default();
        assert_eq!(current_committed_height(&state), 0);

        state.envelope.block_height = 42;
        assert_eq!(current_committed_height(&state), 42);

        state.envelope.block_height = -1;
        assert_eq!(current_committed_height(&state), 0);
    }

    #[test]
    fn test_parse_pinboard_post_cado_path() {
        let wallet = "0x1111111111111111111111111111111111111111";
        let message_id = "msg-123";
        let path = format!(
            "{}{}/{}/{}",
            cado::PATH_PREFIX_PINBOARD,
            abci_query::PINBOARD_SEGMENT_POST,
            wallet,
            message_id
        );

        let (parsed_wallet, parsed_message_id) =
            parse_pinboard_post_cado_path(&path).expect("path should parse");
        assert_eq!(parsed_wallet, wallet);
        assert_eq!(parsed_message_id, message_id);
    }

    #[test]
    fn test_parse_pinboard_post_cado_path_invalid() {
        let invalid = format!("{}wallet/0xabc/123", cado::PATH_PREFIX_PINBOARD);
        let result = parse_pinboard_post_cado_path(&invalid);
        assert!(result.is_err());
    }

    #[test]
    fn test_path_uses_eld_pinboard_lookup_branches() {
        use eld_common::pinboard::path_uses_eld_pinboard_lookup;

        assert!(path_uses_eld_pinboard_lookup(
            "/@eld/pinboard/post/0x1111111111111111111111111111111111111111/msg1"
        ));
        assert!(!path_uses_eld_pinboard_lookup("/@captainhook/msg1"));
    }

    #[tokio::test]
    async fn get_pinboard_post_by_path_returns_metadata_when_blob_is_absent() {
        let wallet_dir = tempfile::TempDir::new().expect("wallet dir");
        let db_dir = tempfile::TempDir::new().expect("db dir");
        let capacity_dir = tempfile::TempDir::new().expect("capacity dir");

        let wallet_path = wallet_dir.path().join("wallets.json");
        std::fs::write(&wallet_path, "[]").expect("empty wallets file");
        let cli = Arc::new(
            ChainClient::with_wallets(
                eld_client::config::ClientConfig {
                    node_host: "127.0.0.1".to_string(),
                    node_port: "26657".to_string(),
                    chain_id: "test-chain".to_string(),
                    faucet_host: "127.0.0.1".to_string(),
                    faucet_port: "8080".to_string(),
                    faucet_end_point: "/faucet/request".to_string(),
                    faucet_url: None,
                    app_port: "9001".to_string(),
                    node_url: None,
                    app_url: None,
                },
                eld_common::fee::FeeConfig::default(),
                wallet_path,
            )
            .expect("test ChainClient"),
        );
        let consensus_config = Arc::new(Mutex::new(ConsensusConfig {
            chain_id: "test-chain".to_string(),
            app_host: "127.0.0.1".to_string(),
            app_port: "26658".to_string(),
            accounts: std::collections::HashMap::new(),
            max_tx_bytes: 10 * 1024 * 1024,
            fee_config: eld_common::fee::FeeConfig::default(),
            storage_limits: crate::config::StorageLimits::default(),
        }));
        let provider =
            Address::parse_hex_str("0x0000000000000000000000000000000000000001").expect("provider");
        let capacity_manager = Arc::new(CapacityManager::new(
            CapacityConfig {
                capacity_dir: capacity_dir.path().to_path_buf(),
                max_capacity_gb: 1,
                provider_id: provider,
                auto_register: false,
                registration_retry_interval_secs: 60,
                tendermint_rpc_url: "http://127.0.0.1:26657".to_string(),
            },
            "capacity".to_string(),
            cli,
            consensus_config,
        ));

        let wallet =
            Address::parse_hex_str("0x1111111111111111111111111111111111111111").expect("wallet");
        let meta = PinboardMessageMetadata {
            message_id: "msg1".to_string(),
            original_signer: wallet,
            content_key: "deadbeef".to_string(),
            content_type: "text/plain".to_string(),
            expires_height: 50,
            visibility: "public".to_string(),
            topic: None,
            tags: vec![],
            committed_height: 1,
            received_timestamp: 1_710_000_000,
            namespace: None,
        };
        let storage = Arc::new(RocksDBStorage::new(db_dir.path()).expect("rocksdb"));
        let tx = storage.begin_transaction();
        storage
            .put_pinboard_metadata_with_tx(&meta.message_id, &meta, &tx)
            .expect("put metadata");
        tx.commit().expect("commit metadata");

        let path = pinboard_eld_post_cado_path(&wallet.hex_with_prefix(), &meta.message_id);
        let rate_limit_state = Arc::new(RwLock::new(RateLimitState::new(
            crate::api::api_rate_limiting::ApiRateLimitConfig {
                enabled: false,
                ..crate::api::api_rate_limiting::ApiRateLimitConfig::default()
            },
        )));
        let committed_state = Arc::new(Mutex::new(AppState::default()));

        let Json(item) = handle_get_pinboard_post_by_path(
            State(storage),
            State(committed_state),
            State(rate_limit_state),
            State(capacity_manager),
            HeaderMap::new(),
            extract::Query(PinboardPostByPathQueryParams { path }),
        )
        .await
        .expect("pinboard get");

        assert_eq!(item.meta, meta);
        assert_eq!(item.cado_path, pinboard_response_cado_path(&meta));
        assert!(item.message_b64.is_none());
        assert_eq!(item.blob_status, "missing");
    }

    #[test]
    fn test_pinboard_blob_fields_by_expiry_and_blob_presence() {
        let wallet = Address::parse_hex_str("0x1111111111111111111111111111111111111111")
            .expect("valid address");
        let meta = PinboardMessageMetadata {
            message_id: "m1".to_string(),
            original_signer: wallet,
            content_key: "ck".to_string(),
            content_type: "text/plain".to_string(),
            expires_height: 10,
            visibility: "public".to_string(),
            topic: None,
            tags: vec![],
            committed_height: 5,
            received_timestamp: 123,
            namespace: None,
        };

        let (msg, status) = pinboard_blob_fields(&meta, 12, Some(b"abc".to_vec()));
        assert!(msg.is_none());
        assert_eq!(status, "expired");

        let (msg, status) = pinboard_blob_fields(&meta, 8, Some(b"abc".to_vec()));
        assert!(msg.is_some());
        assert_eq!(status, "available");

        let (msg, status) = pinboard_blob_fields(&meta, 8, None);
        assert!(msg.is_none());
        assert_eq!(status, "missing");
    }
}
