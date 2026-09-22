//! Slot allocation file REST (`GET /slotallocation`).

use super::api_rate_limiting::{check_rate_limit, ApiEndpointType, RateLimitState};
use super::error::ApiError;
use axum::{extract::State, http::HeaderMap, Json};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::sync::RwLock;
use tracing::{debug, error};

/// Handler for GET /slotallocation - Get slot allocation data from capacity files
pub(crate) async fn handle_get_slot_allocation(
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
