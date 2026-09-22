//! Health and local node identity REST.

use super::api_rate_limiting::{check_rate_limit, ApiEndpointType, RateLimitState};
use super::error::ApiError;
use crate::capacity::capacity_manager::CapacityManager;
use crate::node_identity::{LocalNodeIdentity, NodeIdentityResponse};
use axum::{extract::State, http::HeaderMap, Json};
use std::sync::Arc;
use tokio::sync::RwLock;

pub(crate) async fn health(
    State(rate_limit_state): State<Arc<RwLock<RateLimitState>>>,
    headers: HeaderMap,
) -> Result<&'static str, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::Health, &headers).await?;
    Ok("OK")
}

/// `GET /node_identity` — local TM consensus validator identity (not part of app state).
pub(crate) async fn handle_get_node_identity(
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
