//! Transaction indexer REST (txs, events, verified-proof rewards).

use super::api_rate_limiting::{check_rate_limit, ApiEndpointType, RateLimitState};
use super::error::ApiError;
use crate::indexer::{IndexedEvent, IndexedTransaction, TransactionIndexer};
use axum::{
    extract::{self, State},
    http::HeaderMap,
    Json,
};
use eld_common::Address;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error};

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
pub(crate) async fn handle_get_transactions(
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
pub(crate) async fn handle_get_transaction(
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
pub(crate) async fn handle_get_verified_proof_rewards_sum(
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
pub(crate) async fn handle_get_verified_proof_rewards(
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
pub(crate) async fn handle_get_events(
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
