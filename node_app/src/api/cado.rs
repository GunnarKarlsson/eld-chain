//! CADO lookup REST (`GET /cado/{cado_path}`).

use super::api_rate_limiting::{check_rate_limit, ApiEndpointType, RateLimitState};
use super::error::ApiError;
use super::state::ApiState;
use crate::app_state::AppState;
use crate::storage::rocksdb::RocksDBStorage;
use crate::storage::traits::CADOStorage;
use axum::{
    extract::{self, State},
    http::HeaderMap,
    routing::get,
    Json, Router,
};
use eld_common::cado::{CadoBody, CadoPath};
use eld_common::error::EldError;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;
use tracing::{debug, error};

pub(crate) fn init_cado_routes() -> Router<ApiState> {
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

pub(crate) fn lookup_cado_by_path(
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
