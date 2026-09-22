//! Epoch record REST.

use super::api_rate_limiting::{check_rate_limit, ApiEndpointType, RateLimitState};
use super::cado::lookup_cado_by_path;
use super::error::ApiError;
use crate::app_state::AppState;
use crate::storage::rocksdb::RocksDBStorage;
use crate::storage::traits::EpochRecordListOrder;
use axum::{
    extract::{self, State},
    http::HeaderMap,
    Json,
};
use eld_common::cado::{epoch_record_path_name, CadoPath, CadoPathKey, CadoType};
use eld_common::constants::{cado, protocol::BLOCKS_PER_EPOCH};
use eld_common::validator::EpochRecord;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

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
pub(crate) async fn handle_get_epochs(
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
pub(crate) async fn handle_get_epoch_current(
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
pub(crate) struct EpochPathParams {
    epoch: String,
}

/// Handler for GET /epoch/{epoch}
pub(crate) async fn handle_get_epoch_by_number(
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
pub(crate) struct EpochHeightQueryParams {
    height: String,
}

/// Handler for GET /epoch/height?height={h}
pub(crate) async fn handle_get_epoch_by_height(
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
