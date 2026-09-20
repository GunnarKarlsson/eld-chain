//! HTTP API (Axum routers and handlers).

pub mod admin;
pub mod api_rate_limiting;
pub mod namespace;
pub mod pagination;
pub mod state;

pub use state::ApiState;

use crate::app_state::AppState;
use crate::capacity::capacity_manager::CapacityManager;
use crate::config::ConsensusConfig;
use crate::indexer::{IndexedEvent, IndexedTransaction, TransactionIndexer};
use axum::{
    extract::{self, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use eld_client::api::rest::{PostMessageSubmitRequest, PostMessageSubmitResponse};
use eld_client::facade::cli::Cli;
use eld_common::account::Account;
use eld_common::coin::Coin;
use eld_common::constants::{abci_query, cado, protocol::BLOCKS_PER_EPOCH};
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
use std::collections::HashMap;
use std::path::PathBuf;
use std::str;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, error, info, warn};

use eld_common::cado::{epoch_record_path_name, CadoBody, CadoPath, CadoPathKey, CadoType};
use eld_common::capacity::slot_allocator::CONTENT_ID_NOT_IN_SLOT_MAP;
use eld_common::error::EldError;
use eld_common::validator::EpochRecord;

use self::api_rate_limiting::{ApiEndpointType, ApiRateLimitConfig};
use self::pagination::{PaginationParams, PrefixQueryOptions};
use crate::node_identity::{LocalNodeIdentity, NodeIdentityResponse};
use crate::storage::rocksdb::RocksDBStorage;
use crate::storage::traits::{CADOStorage, EpochRecordListOrder, PinboardGlobalFeedOrder};

// ============================================================================
// STANDARDIZED HTTP API ERROR HANDLING
// ============================================================================

/// Standardized API error response structure
#[derive(Debug, Serialize, Deserialize)]
pub struct ApiErrorResponse {
    /// Error code for programmatic handling
    pub code: String,
    /// Human-readable error message
    pub message: String,
    /// Detailed error information
    pub details: Option<String>,
    /// Request ID for tracking (if available)
    pub request_id: Option<String>,
    /// Timestamp of the error
    pub timestamp: String,
}

/// API error types with corresponding HTTP status codes
#[derive(Debug)]
pub enum ApiError {
    /// 400 Bad Request - Invalid input parameters
    BadRequest {
        message: String,
        details: Option<String>,
    },
    /// 404 Not Found - Resource not found
    NotFound {
        resource_type: String,
        identifier: String,
        details: Option<String>,
    },
    /// 413 Payload Too Large - Request too large
    PayloadTooLarge {
        max_size: usize,
        actual_size: usize,
        details: Option<String>,
    },
    /// 500 Internal Server Error - Server error
    InternalServerError {
        message: String,
        details: Option<String>,
    },
    /// 503 Service Unavailable - Service temporarily unavailable
    ServiceUnavailable {
        message: String,
        details: Option<String>,
    },
}

impl ApiError {
    /// Get the HTTP status code for this error
    pub fn status_code(&self) -> StatusCode {
        match self {
            ApiError::BadRequest { .. } => StatusCode::BAD_REQUEST,
            ApiError::NotFound { .. } => StatusCode::NOT_FOUND,
            ApiError::PayloadTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            ApiError::InternalServerError { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            ApiError::ServiceUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    /// Get the error code for programmatic handling
    pub fn error_code(&self) -> &'static str {
        match self {
            ApiError::BadRequest { .. } => "BAD_REQUEST",
            ApiError::NotFound { .. } => "NOT_FOUND",
            ApiError::PayloadTooLarge { .. } => "PAYLOAD_TOO_LARGE",
            ApiError::InternalServerError { .. } => "INTERNAL_SERVER_ERROR",
            ApiError::ServiceUnavailable { .. } => "SERVICE_UNAVAILABLE",
        }
    }

    /// Get the error message
    pub fn message(&self) -> String {
        match self {
            ApiError::BadRequest { message, .. } => message.clone(),
            ApiError::NotFound {
                resource_type,
                identifier,
                ..
            } => {
                format!("{resource_type} not found: {identifier}")
            }
            ApiError::PayloadTooLarge {
                max_size,
                actual_size,
                ..
            } => {
                format!(
                    "Request too large. Maximum size: {max_size} bytes, actual size: {actual_size} bytes"
                )
            }
            ApiError::InternalServerError { message, .. } => message.clone(),
            ApiError::ServiceUnavailable { message, .. } => message.clone(),
        }
    }

    /// Get additional error details
    pub fn details(&self) -> Option<String> {
        match self {
            ApiError::BadRequest { details, .. } => details.clone(),
            ApiError::NotFound { details, .. } => details.clone(),
            ApiError::PayloadTooLarge { details, .. } => details.clone(),
            ApiError::InternalServerError { details, .. } => details.clone(),
            ApiError::ServiceUnavailable { details, .. } => details.clone(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status_code = self.status_code();
        let error_response = ApiErrorResponse {
            code: self.error_code().to_string(),
            message: self.message(),
            details: self.details(),
            request_id: None, // TODO: Implement request ID tracking
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        let mut headers = HeaderMap::new();
        headers.insert(
            "Content-Type",
            "application/json"
                .parse()
                .expect("Hardcoded Content-Type should always parse"),
        );

        (status_code, headers, Json(error_response)).into_response()
    }
}

// ============================================================================
// RATE LIMITING STATE
// ============================================================================

/// Simple rate limiting state for tracking requests per client
#[derive(Debug)]
pub struct RateLimitState {
    clients: HashMap<String, Vec<Instant>>,
    config: ApiRateLimitConfig,
}

impl RateLimitState {
    pub fn new(config: ApiRateLimitConfig) -> Self {
        Self {
            clients: HashMap::new(),
            config,
        }
    }

    /// Check if a request is allowed for the given client and endpoint type
    pub fn is_allowed(&mut self, client_id: &str, endpoint_type: ApiEndpointType) -> bool {
        if !self.config.enabled {
            return true;
        }

        let max_requests = match endpoint_type {
            ApiEndpointType::General => self.config.general_requests_per_minute,
            ApiEndpointType::Upload => self.config.upload_requests_per_minute,
            ApiEndpointType::Cado => self.config.cado_requests_per_minute,
            ApiEndpointType::Health => self.config.health_requests_per_minute,
        };

        let now = Instant::now();
        let window = Duration::from_secs(60);

        let client_requests = self.clients.entry(client_id.to_string()).or_default();

        // Remove old requests outside the time window
        client_requests.retain(|&time| now.duration_since(time) <= window);

        // Check if we're under the limit
        if client_requests.len() < max_requests as usize {
            client_requests.push(now);
            true
        } else {
            warn!(
                "Rate limit exceeded for client {} on endpoint type {:?}: {} requests in the last minute",
                client_id, endpoint_type, client_requests.len()
            );
            false
        }
    }
}

/// Extract client identifier from request headers
fn extract_client_id(headers: &HeaderMap) -> String {
    // Try to get X-Forwarded-For header first (for proxied requests)
    if let Some(forwarded_for) = headers.get("X-Forwarded-For") {
        if let Ok(forwarded_for_str) = forwarded_for.to_str() {
            // Take the first IP in the chain
            if let Some(first_ip) = forwarded_for_str.split(',').next() {
                return first_ip.trim().to_string();
            }
        }
    }

    // Fall back to X-Real-IP header
    if let Some(real_ip) = headers.get("X-Real-IP") {
        if let Ok(real_ip_str) = real_ip.to_str() {
            return real_ip_str.to_string();
        }
    }

    // Default to a placeholder (in production, you'd want to extract the actual IP)
    "unknown".to_string()
}

/// Check rate limit for a request
async fn check_rate_limit(
    rate_limit_state: &Arc<RwLock<RateLimitState>>,
    endpoint_type: ApiEndpointType,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    let client_id = extract_client_id(headers);

    let rate_limit_exceeded = {
        let mut state = rate_limit_state.write().await;
        !state.is_allowed(&client_id, endpoint_type)
    };

    if rate_limit_exceeded {
        return Err(ApiError::ServiceUnavailable {
            message: "Rate limit exceeded. Please try again later.".to_string(),
            details: Some("Too many requests from this client".to_string()),
        });
    }

    Ok(())
}

// ============================================================================
// API ROUTES
// ============================================================================

fn init_cado_routes() -> Router<ApiState> {
    Router::new().route("/cado/{cado_path}", get(handle_get_cado))
}

/// Resolves a CADO from last-committed in-memory cache, then RocksDB.
///
/// Does not read `current_state` block staging — for mempool / committed-head reads.
pub(crate) fn lookup_cado_by_path_excluding_current_cache(
    committed_state: &Arc<Mutex<AppState>>,
    storage: &impl CADOStorage,
    path: &CadoPath,
) -> Result<Option<CadoBody>, EldError> {
    let committed_snapshot = committed_state
        .lock()
        .map_err(|_| EldError::InitializationError {
            component: "committed state".to_string(),
            details: "lock poisoned".to_string(),
        })?;

    if let Some(cado) = committed_snapshot
        .envelope
        .committed_cado_cache
        .get(path.as_str().as_bytes())
    {
        return Ok(Some(cado.clone()));
    }

    storage.get_cado_by_path(path.clone())
}

fn lookup_cado_by_path(
    current_state: &Arc<Mutex<Option<AppState>>>,
    committed_state: &Arc<Mutex<AppState>>,
    storage: &RocksDBStorage,
    path: &CadoPath,
) -> Result<Option<CadoBody>, ApiError> {
    // extract current snapshot
    let current_snapshot = current_state
        .lock()
        .map_err(|_| ApiError::InternalServerError {
            message: "Failed to read current application state".to_string(),
            details: None,
        })?
        .clone();

    // if the cado is in the current snapshot, return it
    if let Some(current) = current_snapshot.as_ref() {
        if let Some(cado) = current.envelope.cado_cache.get(path.as_str()) {
            return Ok(Some(cado.clone()));
        }
    }

    lookup_cado_by_path_excluding_current_cache(committed_state, storage, path).map_err(|e| {
        error!("Failed to retrieve CADO: {}", e);
        ApiError::InternalServerError {
            message: "Error while retrieving CADO from storage".to_string(),
            details: Some(e.to_string()),
        }
    })
}

async fn handle_get_cado(
    State(storage): State<Arc<RocksDBStorage>>,
    State(committed_state): State<Arc<Mutex<AppState>>>,
    State(current_state): State<Arc<Mutex<Option<AppState>>>>,
    State(rate_limit_state): State<Arc<RwLock<RateLimitState>>>,
    headers: HeaderMap,
    extract::Path(cado_path_string): extract::Path<String>,
) -> Result<Json<Option<CadoBody>>, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::Cado, &headers).await?;
    debug!("handle_get_cado: cado_path: {}", cado_path_string);

    let cado_path = match CadoPath::parse(&cado_path_string) {
        Ok(path) => path,
        Err(e) => {
            return Err(ApiError::BadRequest {
                message: "Invalid CADO path format".to_string(),
                details: Some(format!("Path '{cado_path_string}' is invalid: {e}")),
            });
        }
    };

    let cado_option = lookup_cado_by_path(
        &current_state,
        &committed_state,
        storage.as_ref(),
        &cado_path,
    )?;

    Ok(Json(cado_option))
}

/// Inputs for [`init_router_with_storage`].
pub struct ApiRouterInitContext {
    pub storage: Arc<RocksDBStorage>,
    pub consensus_config: Arc<Mutex<ConsensusConfig>>,
    pub rate_limit_state: Arc<RwLock<RateLimitState>>,
    pub indexer: Option<Arc<TransactionIndexer>>,
    pub cli: Arc<Cli>,
    pub capacity_manager: Arc<CapacityManager>,
    pub local_identity: Arc<std::sync::RwLock<LocalNodeIdentity>>,
    pub admin_token: Option<String>,
    pub committed_state: Arc<Mutex<crate::app_state::AppState>>,
    pub current_state: Arc<Mutex<Option<crate::app_state::AppState>>>,
}

/// Initialize router with storage access for content browser API.
///
/// Pinboard **message bytes** in GET/submit flows are read from [`CapacityManager::get_content_from_slots`]
/// only (not from RocksDB `pinboard_blob`).
pub fn init_router_with_storage(ctx: ApiRouterInitContext) -> Router {
    let ApiRouterInitContext {
        storage,
        consensus_config,
        rate_limit_state,
        indexer,
        cli,
        capacity_manager,
        local_identity,
        admin_token,
        committed_state,
        current_state,
    } = ctx;

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app_state = ApiState {
        storage: storage.clone(),
        consensus_config,
        rate_limit_state: rate_limit_state.clone(),
        cli,
        capacity_manager,
        local_identity,
        admin_token,
        indexer,
        committed_state,
        current_state,
    };

    // Base router: pinboard + common endpoints + admin (same listener as upload/content port)
    let base_router = Router::new()
        .route("/health", get(health))
        .route("/node_identity", get(handle_get_node_identity))
        .route("/v1/pinboard/messages:submit", post(handle_pinboard_submit))
        .route("/v1/pinboard/posts", get(handle_get_pinboard_posts))
        .route("/v1/pinboard/post", get(handle_get_pinboard_post_by_path))
        .route("/v1/namespaces", get(namespace::handle_list_namespaces))
        .route(
            "/v1/namespace/{namespace_slug}",
            get(namespace::handle_get_namespace),
        )
        .route("/admin/v1/status", get(admin::handle_admin_status));

    let transaction_router = Router::new()
        .route("/transactions", get(handle_get_transactions))
        .route("/transaction", get(handle_get_transaction))
        .route("/events", get(handle_get_events))
        .route(
            "/v1/capacity/verified-proof-rewards/sum",
            get(handle_get_verified_proof_rewards_sum),
        )
        .route(
            "/v1/capacity/verified-proof-rewards/{address}",
            get(handle_get_verified_proof_rewards),
        )
        .route("/epochs", get(handle_get_epochs))
        .route("/epoch/current", get(handle_get_epoch_current))
        .route("/epoch/height", get(handle_get_epoch_by_height))
        .route("/epoch/{epoch}", get(handle_get_epoch_by_number))
        .route("/slotallocation", get(handle_get_slot_allocation));

    base_router
        .merge(transaction_router)
        .merge(init_cado_routes())
        .with_state(app_state)
        .layer(cors)
}

pub async fn health(
    State(rate_limit_state): State<Arc<RwLock<RateLimitState>>>,
    headers: HeaderMap,
) -> Result<&'static str, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::Health, &headers).await?;
    Ok("OK")
}

/// `GET /node_identity` — local TM consensus validator identity (not part of app state).
pub async fn handle_get_node_identity(
    State(rate_limit_state): State<Arc<RwLock<RateLimitState>>>,
    State(capacity_manager): State<Arc<CapacityManager>>,
    State(local_identity): State<Arc<std::sync::RwLock<LocalNodeIdentity>>>,
    headers: HeaderMap,
) -> Result<Json<NodeIdentityResponse>, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::General, &headers).await?;

    let identity = local_identity
        .read()
        .map_err(|e| ApiError::InternalServerError {
            message: format!("Failed to read local node identity: {e}"),
            details: None,
        })?;

    let capacity_provider_address = capacity_manager.config().provider_id;
    Ok(Json(NodeIdentityResponse::from_identity(
        &identity,
        &capacity_provider_address,
    )))
}

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
    // TODO: Can we do this via abci call - ?
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
async fn handle_pinboard_submit(
    State(storage): State<Arc<RocksDBStorage>>,
    State(consensus_config): State<Arc<Mutex<ConsensusConfig>>>,
    State(rate_limit_state): State<Arc<RwLock<RateLimitState>>>,
    State(cli): State<Arc<Cli>>,
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
    // TODO: Also check it doesn't exist in temp blob storage. if it does, should extend ttl?
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
struct PinboardPostsQueryParams {
    order: Option<PinboardGlobalFeedOrder>,
    page: Option<usize>,
    page_size: Option<usize>,
    wallet: Option<String>,
    include_expired: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct PinboardPostByPathQueryParams {
    path: String,
}

#[derive(Debug, Serialize)]
struct PinboardRestPagination {
    page: usize,
    page_size: usize,
    has_more: bool,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
struct PinboardRestPostItem {
    cado_path: String,
    meta: PinboardMessageMetadata,
    message_b64: Option<String>,
    blob_status: String,
}

#[derive(Debug, Serialize)]
struct PinboardPostsResponse {
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
        // TODO: get-content_from_slots should return None if not found but no error
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

async fn handle_get_pinboard_posts(
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

async fn handle_get_pinboard_post_by_path(
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

/// Query parameters for transaction listing
#[derive(Debug, Deserialize)]
pub struct TransactionListParams {
    /// With `after_index`, return txs strictly older than `(after_height, after_index)` in chain order.
    pub after_height: Option<u64>,
    /// Must be paired with `after_height` when continuing a listing.
    pub after_index: Option<u32>,
    pub limit: Option<u32>,
    pub block_height: Option<u64>,
    pub sender: Option<String>,
    #[serde(rename = "type")]
    pub payload_type: Option<String>,
}

/// Query parameters for single transaction lookup
#[derive(Debug, Deserialize)]
pub struct TransactionQueryParams {
    pub id: String,
}

/// Query parameters for event listing
#[derive(Debug, Deserialize)]
pub struct EventListParams {
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub txid: Option<String>,
}

/// Pagination information for transaction list response
#[derive(Debug, Serialize, Deserialize)]
pub struct TransactionPagination {
    pub limit: u32,
    /// True when additional rows exist after this page (`limit + 1` probe).
    pub has_next: bool,
    /// Primary index total when unfiltered; filtered total on the first page (no `after_*`);
    /// omitted on filtered continuation requests (`after_*` set) to avoid repeated full scans.
    pub total: Option<u64>,
}

/// Response for transaction list endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct TransactionListResponse {
    pub transactions: Vec<IndexedTransaction>,
    pub pagination: TransactionPagination,
}

/// Pagination information for event list response
#[derive(Debug, Serialize, Deserialize)]
pub struct EventPagination {
    pub page: u32,
    pub limit: u32,
    pub total: u64,
    pub total_pages: u32,
}

/// Response for event list endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct EventListResponse {
    pub events: Vec<IndexedEvent>,
    pub pagination: EventPagination,
}

/// Query parameters for verified proof reward aggregation by block height (inclusive).
#[derive(Debug, Deserialize)]
pub struct VerifiedProofRewardsQuery {
    /// First block height to include.
    pub from_height: u64,
    /// Last block height to include.
    pub to_height: u64,
}

/// Response for `GET /v1/capacity/verified-proof-rewards/:address`.
#[derive(Debug, Serialize, Deserialize)]
pub struct VerifiedProofRewardsResponse {
    /// Canonical `0x`-prefixed capacity provider address.
    pub capacity_provider: String,
    /// First block height included in the aggregation.
    pub from_height: u64,
    /// Last block height included in the aggregation.
    pub to_height: u64,
    /// Number of successful `VerifiedProof` transactions in the height range.
    pub successful_proofs: u64,
    /// Total native units (`u128` as decimal string).
    pub total_rewards: String,
}

/// Response for `GET /v1/capacity/verified-proof-rewards/sum` (network-wide lifetime rollup).
#[derive(Debug, Serialize, Deserialize)]
pub struct VerifiedProofRewardsSumResponse {
    /// Number of successful indexed `VerifiedProof` transactions since the rollup existed.
    pub successful_proofs: u64,
    /// Total native units (`u128` as decimal string).
    pub total_rewards: String,
}

/// Maximum `(to_height - from_height)` accepted by [`handle_get_verified_proof_rewards`].
const MAX_VERIFIED_PROOF_REWARD_HEIGHT_SPAN: u64 = 1_000_000;

// ============================================================================
// TRANSACTION INDEXER API ENDPOINTS
// ============================================================================

/// Handler for GET /transactions — newest-first chronological list (`after_height` / `after_index`).
async fn handle_get_transactions(
    State(indexer): State<Option<Arc<TransactionIndexer>>>,
    State(rate_limit_state): State<Arc<RwLock<RateLimitState>>>,
    headers: HeaderMap,
    extract::Query(params): extract::Query<TransactionListParams>,
) -> Result<Json<TransactionListResponse>, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::General, &headers).await?;

    // Check if indexer is enabled
    let indexer = indexer.ok_or_else(|| ApiError::ServiceUnavailable {
        message: "Transaction indexer is not enabled".to_string(),
        details: Some("Enable the indexer in config.json by setting 'indexer': true".to_string()),
    })?;

    let limit = params.limit.unwrap_or(50).min(100); // Max 100 per page

    if limit == 0 {
        return Err(ApiError::BadRequest {
            message: "Invalid limit".to_string(),
            details: Some("Limit must be > 0".to_string()),
        });
    }

    let continuation = match (params.after_height, params.after_index) {
        (None, None) => None,
        (Some(h), Some(i)) => Some((h, i)),
        _ => {
            return Err(ApiError::BadRequest {
                message: "Invalid transaction list continuation".to_string(),
                details: Some("Provide both after_height and after_index, or neither.".to_string()),
            });
        }
    };

    debug!(
        "Getting transactions: continuation={:?}, limit={}, block_height={:?}, sender={:?}, type={:?}",
        continuation,
        limit,
        params.block_height,
        params.sender,
        params.payload_type
    );

    let sender_addr = match &params.sender {
        None => None,
        Some(s) => match Address::parse_hex_str(s) {
            Ok(addr) => Some(addr),
            Err(_) => {
                return Ok(Json(TransactionListResponse {
                    transactions: vec![],
                    pagination: TransactionPagination {
                        limit,
                        has_next: false,
                        total: Some(0),
                    },
                }));
            }
        },
    };

    let filtered_query =
        params.block_height.is_some() || params.sender.is_some() || params.payload_type.is_some();

    let total: Option<u64> = if !filtered_query {
        match indexer.indexed_transactions_primary_total() {
            Ok(n) => Some(n),
            Err(e) => {
                error!("Failed to read indexed transaction total: {}", e);
                return Err(ApiError::InternalServerError {
                    message: "Failed to retrieve transactions".to_string(),
                    details: Some(e.to_string()),
                });
            }
        }
    } else if continuation.is_none() {
        match indexer.count_transactions_matching_chron_filters(
            params.block_height,
            sender_addr,
            params.payload_type.as_deref(),
        ) {
            Ok(n) => Some(n),
            Err(e) => {
                error!("Failed to count filtered transactions: {}", e);
                return Err(ApiError::InternalServerError {
                    message: "Failed to retrieve transactions".to_string(),
                    details: Some(e.to_string()),
                });
            }
        }
    } else {
        None
    };

    let mut rows = match indexer.list_transactions_chron_desc(
        continuation,
        limit,
        true,
        params.block_height,
        sender_addr,
        params.payload_type.as_deref(),
    ) {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to list transactions chronologically: {}", e);
            return Err(ApiError::InternalServerError {
                message: "Failed to retrieve transactions".to_string(),
                details: Some(e.to_string()),
            });
        }
    };

    let has_next = rows.len() > limit as usize;
    if has_next {
        rows.truncate(limit as usize);
    }

    let pagination = TransactionPagination {
        limit,
        has_next,
        total,
    };

    Ok(Json(TransactionListResponse {
        transactions: rows,
        pagination,
    }))
}

/// Handler for GET /transaction?id={tx_id} - Get specific transaction
async fn handle_get_transaction(
    State(indexer): State<Option<Arc<TransactionIndexer>>>,
    State(rate_limit_state): State<Arc<RwLock<RateLimitState>>>,
    headers: HeaderMap,
    extract::Query(params): extract::Query<TransactionQueryParams>,
) -> Result<Json<IndexedTransaction>, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::General, &headers).await?;

    // Check if indexer is enabled
    let indexer = indexer.ok_or_else(|| ApiError::ServiceUnavailable {
        message: "Transaction indexer is not enabled".to_string(),
        details: Some("Enable the indexer in config.json by setting 'indexer': true".to_string()),
    })?;

    debug!("Getting transaction by ID: {}", params.id);

    match indexer.get_transaction(&params.id) {
        Ok(Some(transaction)) => Ok(Json(transaction)),
        Ok(None) => Err(ApiError::NotFound {
            resource_type: "Transaction".to_string(),
            identifier: params.id,
            details: Some("Transaction not found in index".to_string()),
        }),
        Err(e) => {
            error!("Failed to get transaction: {}", e);
            Err(ApiError::InternalServerError {
                message: "Failed to retrieve transaction".to_string(),
                details: Some(e.to_string()),
            })
        }
    }
}

/// Handler for `GET /v1/capacity/verified-proof-rewards/sum` — O(1) lifetime network-wide rollup.
async fn handle_get_verified_proof_rewards_sum(
    State(indexer): State<Option<Arc<TransactionIndexer>>>,
    State(rate_limit_state): State<Arc<RwLock<RateLimitState>>>,
    headers: HeaderMap,
) -> Result<Json<VerifiedProofRewardsSumResponse>, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::General, &headers).await?;

    let indexer = indexer.ok_or_else(|| ApiError::ServiceUnavailable {
        message: "Transaction indexer is not enabled".to_string(),
        details: Some("Enable the indexer in config.json by setting 'indexer': true".to_string()),
    })?;

    let (successful_proofs, total_rewards) =
        indexer.global_verified_proof_rewards().map_err(|e| {
            error!("global_verified_proof_rewards: {}", e);
            ApiError::InternalServerError {
                message: "Failed to read verified proof rewards sum".to_string(),
                details: Some(e.to_string()),
            }
        })?;

    Ok(Json(VerifiedProofRewardsSumResponse {
        successful_proofs,
        total_rewards: total_rewards.to_string(),
    }))
}

/// Handler for `GET /v1/capacity/verified-proof-rewards/{address}`.
async fn handle_get_verified_proof_rewards(
    State(indexer): State<Option<Arc<TransactionIndexer>>>,
    State(rate_limit_state): State<Arc<RwLock<RateLimitState>>>,
    headers: HeaderMap,
    extract::Path(address): extract::Path<String>,
    extract::Query(query): extract::Query<VerifiedProofRewardsQuery>,
) -> Result<Json<VerifiedProofRewardsResponse>, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::General, &headers).await?;

    let indexer = indexer.ok_or_else(|| ApiError::ServiceUnavailable {
        message: "Transaction indexer is not enabled".to_string(),
        details: Some("Enable the indexer in config.json by setting 'indexer': true".to_string()),
    })?;

    if query.from_height > query.to_height {
        return Err(ApiError::BadRequest {
            message: "Invalid block height range".to_string(),
            details: Some("from_height must be <= to_height".to_string()),
        });
    }

    let span = query.to_height - query.from_height;
    if span > MAX_VERIFIED_PROOF_REWARD_HEIGHT_SPAN {
        return Err(ApiError::BadRequest {
            message: "Block height range too large".to_string(),
            details: Some(format!(
                "Maximum allowed (to_height - from_height) is {MAX_VERIFIED_PROOF_REWARD_HEIGHT_SPAN}"
            )),
        });
    }

    let addr = Address::parse_hex_str(address.trim()).map_err(|e| ApiError::BadRequest {
        message: "Invalid capacity_provider address".to_string(),
        details: Some(e.to_string()),
    })?;
    let capacity_provider = addr.hex_with_prefix();

    let (successful_proofs, total_rewards) = indexer
        .aggregate_verified_proof_rewards(&capacity_provider, query.from_height, query.to_height)
        .map_err(|e| {
            error!("aggregate_verified_proof_rewards: {}", e);
            ApiError::InternalServerError {
                message: "Failed to aggregate verified proof rewards".to_string(),
                details: Some(e.to_string()),
            }
        })?;

    Ok(Json(VerifiedProofRewardsResponse {
        capacity_provider,
        from_height: query.from_height,
        to_height: query.to_height,
        successful_proofs,
        total_rewards: total_rewards.to_string(),
    }))
}

/// Handler for GET /events - Get events (all events or filtered by txid)
async fn handle_get_events(
    State(indexer): State<Option<Arc<TransactionIndexer>>>,
    State(rate_limit_state): State<Arc<RwLock<RateLimitState>>>,
    headers: HeaderMap,
    extract::Query(params): extract::Query<EventListParams>,
) -> Result<Json<EventListResponse>, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::General, &headers).await?;

    // Check if indexer is enabled
    let indexer = indexer.ok_or_else(|| ApiError::ServiceUnavailable {
        message: "Transaction indexer is not enabled".to_string(),
        details: Some("Enable the indexer in config.json by setting 'indexer': true".to_string()),
    })?;

    // If txid is provided, get events for that transaction
    if let Some(txid) = &params.txid {
        debug!("Getting events for transaction ID: {}", txid);

        match indexer.get_events_by_tx_id(txid) {
            Ok(events) => {
                let total = events.len() as u64;
                let pagination = EventPagination {
                    page: 1,
                    limit: total as u32,
                    total,
                    total_pages: 1,
                };

                return Ok(Json(EventListResponse { events, pagination }));
            }
            Err(e) => {
                error!("Failed to get events by txid: {}", e);
                return Err(ApiError::InternalServerError {
                    message: "Failed to retrieve events".to_string(),
                    details: Some(e.to_string()),
                });
            }
        }
    }

    // Otherwise, get all events with pagination
    let page = params.page.unwrap_or(1);
    let limit = params.limit.unwrap_or(50).min(100); // Max 100 per page

    if page == 0 {
        return Err(ApiError::BadRequest {
            message: "Invalid page number".to_string(),
            details: Some("Page must be >= 1".to_string()),
        });
    }

    if limit == 0 {
        return Err(ApiError::BadRequest {
            message: "Invalid limit".to_string(),
            details: Some("Limit must be > 0".to_string()),
        });
    }

    debug!("Getting all events: page={}, limit={}", page, limit);

    // Get all events from indexer
    let (events, total) = match indexer.get_all_events(page, limit) {
        Ok(result) => result,
        Err(e) => {
            error!("Failed to get events: {}", e);
            return Err(ApiError::InternalServerError {
                message: "Failed to retrieve events".to_string(),
                details: Some(e.to_string()),
            });
        }
    };

    // Calculate pagination info
    let total_pages = ((total as f64) / (limit as f64)).ceil() as u32;

    let pagination = EventPagination {
        page,
        limit,
        total,
        total_pages: total_pages.max(1),
    };

    Ok(Json(EventListResponse { events, pagination }))
}

/// Query parameters for GET /epochs
#[derive(Debug, Deserialize)]
pub struct EpochListParams {
    /// `desc` (default) or `asc`
    pub order: Option<String>,
    /// Continuation: last `epoch` from the previous page (exclusive boundary).
    pub after_epoch: Option<i64>,
    pub limit: Option<u32>,
}

/// Pagination for epoch list responses
#[derive(Debug, Serialize, Deserialize)]
pub struct EpochListPagination {
    pub limit: u32,
    pub has_next: bool,
    /// Full count on the first page (no `after_epoch`); omitted on continuation pages.
    pub total: Option<u64>,
}

/// Response for GET /epochs
#[derive(Debug, Serialize, Deserialize)]
pub struct EpochListResponse {
    pub epochs: Vec<EpochRecord>,
    pub pagination: EpochListPagination,
}

/// Handler for GET /epochs — chronological epoch record list (`after_epoch` cursor).
async fn handle_get_epochs(
    State(storage): State<Arc<RocksDBStorage>>,
    State(committed_state): State<Arc<Mutex<AppState>>>,
    State(current_state): State<Arc<Mutex<Option<AppState>>>>,
    State(rate_limit_state): State<Arc<RwLock<RateLimitState>>>,
    headers: HeaderMap,
    extract::Query(params): extract::Query<EpochListParams>,
) -> Result<Json<EpochListResponse>, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::General, &headers).await?;

    let limit = params.limit.unwrap_or(50).min(100);
    if limit == 0 {
        return Err(ApiError::BadRequest {
            message: "Invalid limit".to_string(),
            details: Some("Limit must be > 0".to_string()),
        });
    }

    let order = match params.order.as_deref() {
        None | Some("desc") => EpochRecordListOrder::Desc,
        Some("asc") => EpochRecordListOrder::Asc,
        Some(other) => {
            return Err(ApiError::BadRequest {
                message: "Invalid order".to_string(),
                details: Some(format!("order must be 'desc' or 'asc', got '{other}'")),
            });
        }
    };

    let fetch_limit = limit as usize + 1;
    let (mut epochs, total) = list_epoch_records(
        &current_state,
        &committed_state,
        storage.as_ref(),
        order,
        params.after_epoch,
        fetch_limit,
    )?;

    let has_next = epochs.len() > limit as usize;
    if has_next {
        epochs.truncate(limit as usize);
    }

    Ok(Json(EpochListResponse {
        epochs,
        pagination: EpochListPagination {
            limit,
            has_next,
            total,
        },
    }))
}

fn list_epoch_records(
    current_state: &Arc<Mutex<Option<AppState>>>,
    committed_state: &Arc<Mutex<AppState>>,
    storage: &RocksDBStorage,
    order: EpochRecordListOrder,
    after_epoch: Option<i64>,
    fetch_limit: usize,
) -> Result<(Vec<EpochRecord>, Option<u64>), ApiError> {
    let current_snapshot = current_state
        .lock()
        .map_err(|_| ApiError::InternalServerError {
            message: "Failed to read current application state".to_string(),
            details: None,
        })?
        .clone();

    let committed_snapshot = committed_state
        .lock()
        .map_err(|_| ApiError::InternalServerError {
            message: "Failed to read committed application state".to_string(),
            details: None,
        })?
        .clone();

    let epoch_numbers = committed_snapshot
        .envelope
        .collect_epoch_numbers(current_snapshot.as_ref().map(|s| &s.envelope));

    let total = if after_epoch.is_none() {
        Some(epoch_numbers.len() as u64)
    } else {
        None
    };

    let selected: Vec<i64> = match order {
        EpochRecordListOrder::Desc => epoch_numbers
            .iter()
            .rev()
            .filter(|&&epoch| match after_epoch {
                Some(ae) => epoch < ae,
                None => true,
            })
            .take(fetch_limit)
            .copied()
            .collect(),
        EpochRecordListOrder::Asc => epoch_numbers
            .iter()
            .filter(|&&epoch| match after_epoch {
                Some(ae) => epoch > ae,
                None => true,
            })
            .take(fetch_limit)
            .copied()
            .collect(),
    };

    let mut epochs = Vec::with_capacity(selected.len());
    for epoch in selected {
        epochs.push(get_epoch_record(
            current_state,
            committed_state,
            storage,
            epoch,
        )?);
    }

    Ok((epochs, total))
}

fn epoch_record_cado_path(epoch: i64) -> Result<CadoPath, ApiError> {
    let key = epoch_record_path_name(epoch).map_err(|e| ApiError::InternalServerError {
        message: "Failed to build epoch record path key".to_string(),
        details: Some(e.to_string()),
    })?;
    CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(&key)).map_err(|e| {
        ApiError::InternalServerError {
            message: "Failed to build epoch record path".to_string(),
            details: Some(e.to_string()),
        }
    })
}

fn get_epoch_record(
    current_state: &Arc<Mutex<Option<AppState>>>,
    committed_state: &Arc<Mutex<AppState>>,
    storage: &RocksDBStorage,
    epoch: i64,
) -> Result<EpochRecord, ApiError> {
    let path = epoch_record_cado_path(epoch)?;
    match lookup_cado_by_path(current_state, committed_state, storage, &path)? {
        Some(cado) => {
            EpochRecord::deserialize_bin(cado.data()).map_err(|e| ApiError::InternalServerError {
                message: "Failed to deserialize epoch record".to_string(),
                details: Some(e.to_string()),
            })
        }
        None => Err(ApiError::NotFound {
            resource_type: "EpochRecord".to_string(),
            identifier: epoch.to_string(),
            details: Some(format!("No epoch record stored for epoch {epoch}")),
        }),
    }
}

fn get_latest_epoch_record(
    current_state: &Arc<Mutex<Option<AppState>>>,
    committed_state: &Arc<Mutex<AppState>>,
    storage: &RocksDBStorage,
) -> Result<EpochRecord, ApiError> {
    let path =
        CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(cado::LATEST)).map_err(|e| {
            ApiError::InternalServerError {
                message: "Failed to build latest epoch record path".to_string(),
                details: Some(e.to_string()),
            }
        })?;
    match lookup_cado_by_path(current_state, committed_state, storage, &path)? {
        Some(cado) => {
            EpochRecord::deserialize_bin(cado.data()).map_err(|e| ApiError::InternalServerError {
                message: "Failed to deserialize latest epoch record".to_string(),
                details: Some(e.to_string()),
            })
        }
        None => Err(ApiError::NotFound {
            resource_type: "EpochRecord".to_string(),
            identifier: "latest".to_string(),
            details: Some("No epoch record has been persisted yet".to_string()),
        }),
    }
}

/// Handler for GET /epoch/current
async fn handle_get_epoch_current(
    State(storage): State<Arc<RocksDBStorage>>,
    State(committed_state): State<Arc<Mutex<AppState>>>,
    State(current_state): State<Arc<Mutex<Option<AppState>>>>,
    State(rate_limit_state): State<Arc<RwLock<RateLimitState>>>,
    headers: HeaderMap,
) -> Result<Json<EpochRecord>, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::General, &headers).await?;
    let record = get_latest_epoch_record(&current_state, &committed_state, storage.as_ref())?;
    Ok(Json(record))
}

/// Path parameter for GET /epoch/{epoch}
#[derive(Debug, Deserialize)]
struct EpochPathParams {
    epoch: String,
}

/// Handler for GET /epoch/{epoch}
async fn handle_get_epoch_by_number(
    State(storage): State<Arc<RocksDBStorage>>,
    State(committed_state): State<Arc<Mutex<AppState>>>,
    State(current_state): State<Arc<Mutex<Option<AppState>>>>,
    State(rate_limit_state): State<Arc<RwLock<RateLimitState>>>,
    headers: HeaderMap,
    extract::Path(params): extract::Path<EpochPathParams>,
) -> Result<Json<EpochRecord>, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::General, &headers).await?;

    let epoch: i64 = params
        .epoch
        .trim()
        .parse()
        .map_err(|e| ApiError::BadRequest {
            message: "Invalid epoch number".to_string(),
            details: Some(format!("{}: {}", params.epoch, e)),
        })?;

    let record = get_epoch_record(&current_state, &committed_state, storage.as_ref(), epoch)?;
    Ok(Json(record))
}

/// Query parameters for GET /epoch/height
#[derive(Debug, Deserialize)]
struct EpochHeightQueryParams {
    height: String,
}

/// Handler for GET /epoch/height?height={h}
async fn handle_get_epoch_by_height(
    State(storage): State<Arc<RocksDBStorage>>,
    State(committed_state): State<Arc<Mutex<AppState>>>,
    State(current_state): State<Arc<Mutex<Option<AppState>>>>,
    State(rate_limit_state): State<Arc<RwLock<RateLimitState>>>,
    headers: HeaderMap,
    extract::Query(params): extract::Query<EpochHeightQueryParams>,
) -> Result<Json<EpochRecord>, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::General, &headers).await?;

    let height: i64 = params
        .height
        .trim()
        .parse()
        .map_err(|e| ApiError::BadRequest {
            message: "Invalid block height".to_string(),
            details: Some(format!("{}: {}", params.height, e)),
        })?;

    let epoch = height / BLOCKS_PER_EPOCH;
    let record = get_epoch_record(&current_state, &committed_state, storage.as_ref(), epoch)?;
    Ok(Json(record))
}

/// Handler for GET /slotallocation - Get slot allocation data from capacity files
async fn handle_get_slot_allocation(
    State(rate_limit_state): State<Arc<RwLock<RateLimitState>>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::General, &headers).await?;

    // Path to the capacity directory
    let capacity_dir = PathBuf::from("./data/capacity");

    // Check if directory exists
    if !capacity_dir.exists() {
        return Err(ApiError::NotFound {
            resource_type: "Capacity directory".to_string(),
            identifier: capacity_dir.to_string_lossy().to_string(),
            details: Some("Capacity directory does not exist".to_string()),
        });
    }

    // Read directory entries
    let mut entries = fs::read_dir(&capacity_dir).await.map_err(|e| {
        error!("Failed to read capacity directory: {}", e);
        ApiError::InternalServerError {
            message: "Failed to read capacity directory".to_string(),
            details: Some(e.to_string()),
        }
    })?;

    // Find all files matching *.slots.json pattern
    let mut matching_files = Vec::new();

    while let Some(entry) = entries.next_entry().await.map_err(|e| {
        error!("Failed to read directory entry: {}", e);
        ApiError::InternalServerError {
            message: "Failed to read directory entry".to_string(),
            details: Some(e.to_string()),
        }
    })? {
        let path = entry.path();

        // Check if file matches the pattern: *.slots.json
        if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
            if file_name.ends_with("slots.json") {
                matching_files.push(path);
            }
        }
    }

    // If there's not exactly one file, return empty JSON
    if matching_files.len() != 1 {
        debug!(
            "Found {} slot allocation file(s), expected exactly 1. Returning empty JSON.",
            matching_files.len()
        );
        return Ok(Json(serde_json::json!({})));
    }

    // Read and return the single matching file
    let file_path = &matching_files[0];
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    debug!("Reading slot allocation file: {}", file_name);

    let file_contents = fs::read_to_string(file_path).await.map_err(|e| {
        error!("Failed to read file {}: {}", file_name, e);
        ApiError::InternalServerError {
            message: format!("Failed to read file: {file_name}"),
            details: Some(e.to_string()),
        }
    })?;

    let file_data: serde_json::Value = serde_json::from_str(&file_contents).map_err(|e| {
        error!("Failed to parse JSON from file {}: {}", file_name, e);
        ApiError::InternalServerError {
            message: format!("Failed to parse JSON from file: {file_name}"),
            details: Some(e.to_string()),
        }
    })?;

    Ok(Json(file_data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::api_rate_limiting::ApiRateLimitConfig;
    use axum::http::StatusCode;
    use eld_common::constants::{abci_query, cado};
    use eld_common::pinboard::PinboardMessageMetadata;
    use eld_common::Address;

    #[test]
    fn test_api_error_status_codes() {
        assert_eq!(
            ApiError::BadRequest {
                message: "test".to_string(),
                details: None,
            }
            .status_code(),
            StatusCode::BAD_REQUEST
        );

        assert_eq!(
            ApiError::NotFound {
                resource_type: "test".to_string(),
                identifier: "id".to_string(),
                details: None,
            }
            .status_code(),
            StatusCode::NOT_FOUND
        );

        assert_eq!(
            ApiError::InternalServerError {
                message: "test".to_string(),
                details: None,
            }
            .status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );

        assert_eq!(
            ApiError::ServiceUnavailable {
                message: "test".to_string(),
                details: None,
            }
            .status_code(),
            StatusCode::SERVICE_UNAVAILABLE
        );

        assert_eq!(
            ApiError::PayloadTooLarge {
                max_size: 1000,
                actual_size: 2000,
                details: None,
            }
            .status_code(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }

    #[test]
    fn test_api_error_codes() {
        assert_eq!(
            ApiError::BadRequest {
                message: "test".to_string(),
                details: None,
            }
            .error_code(),
            "BAD_REQUEST"
        );

        assert_eq!(
            ApiError::NotFound {
                resource_type: "test".to_string(),
                identifier: "id".to_string(),
                details: None,
            }
            .error_code(),
            "NOT_FOUND"
        );

        assert_eq!(
            ApiError::InternalServerError {
                message: "test".to_string(),
                details: None,
            }
            .error_code(),
            "INTERNAL_SERVER_ERROR"
        );

        assert_eq!(
            ApiError::ServiceUnavailable {
                message: "test".to_string(),
                details: None,
            }
            .error_code(),
            "SERVICE_UNAVAILABLE"
        );

        assert_eq!(
            ApiError::PayloadTooLarge {
                max_size: 1000,
                actual_size: 2000,
                details: None,
            }
            .error_code(),
            "PAYLOAD_TOO_LARGE"
        );
    }

    #[test]
    fn test_api_error_messages() {
        let bad_request = ApiError::BadRequest {
            message: "Invalid input".to_string(),
            details: None,
        };
        assert_eq!(bad_request.message(), "Invalid input");

        let not_found = ApiError::NotFound {
            resource_type: "User".to_string(),
            identifier: "123".to_string(),
            details: None,
        };
        assert_eq!(not_found.message(), "User not found: 123");

        let payload_too_large = ApiError::PayloadTooLarge {
            max_size: 1000,
            actual_size: 2000,
            details: None,
        };
        assert!(payload_too_large.message().contains("Request too large"));
        assert!(payload_too_large.message().contains("1000"));
        assert!(payload_too_large.message().contains("2000"));

        let internal_error = ApiError::InternalServerError {
            message: "Database error".to_string(),
            details: Some("Connection timeout".to_string()),
        };
        assert_eq!(internal_error.message(), "Database error");
        assert_eq!(
            internal_error.details(),
            Some("Connection timeout".to_string())
        );
    }

    #[test]
    fn test_api_error_serialization() {
        let error_response = ApiErrorResponse {
            code: "NOT_FOUND".to_string(),
            message: "Resource not found".to_string(),
            details: Some("No such resource".to_string()),
            request_id: Some("req-456".to_string()),
            timestamp: "2024-01-01T12:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&error_response).unwrap();
        assert!(json.contains("NOT_FOUND"));
        assert!(json.contains("Resource not found"));
        assert!(json.contains("No such resource"));
        assert!(json.contains("req-456"));
        assert!(json.contains("2024-01-01T12:00:00Z"));
    }

    #[test]
    fn test_api_error_deserialization() {
        let json = r#"{
        "code": "BAD_REQUEST",
        "message": "Invalid input",
        "details": "Missing required field",
        "request_id": "req-789",
        "timestamp": "2024-01-01T12:00:00Z"
    }"#;

        let error_response: ApiErrorResponse = serde_json::from_str(json).unwrap();

        assert_eq!(error_response.code, "BAD_REQUEST");
        assert_eq!(error_response.message, "Invalid input");
        assert_eq!(
            error_response.details,
            Some("Missing required field".to_string())
        );
        assert_eq!(error_response.request_id, Some("req-789".to_string()));
        assert_eq!(error_response.timestamp, "2024-01-01T12:00:00Z");
    }

    #[test]
    fn test_rate_limit_config_default() {
        let config = ApiRateLimitConfig::default();
        assert_eq!(config.general_requests_per_minute, 1000);
        assert_eq!(config.upload_requests_per_minute, 100);
        assert_eq!(config.cado_requests_per_minute, 2000);
        assert_eq!(config.health_requests_per_minute, 5000);
        assert!(config.enabled);
    }

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
