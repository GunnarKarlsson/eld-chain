//! Admin HTTP routes served on the same listener as the app REST API (`app_port`).

use crate::capacity::capacity_manager::CapacityManager;
use crate::config::ConsensusConfig;
use crate::storage::rocksdb::RocksDBStorage;
use crate::sys_disk::{directory_tree_size_bytes, statvfs_for_path};
use axum::{
    extract::State,
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use eld_common::address::Address;
use eld_common::capacity::slot_allocator::SlotAllocator;
use eld_common::capacity_proof::SlotState;
use eld_common::error::EldError;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::warn;

/// When set, `GET /admin/v1/status` requires `Authorization: Bearer <this value>`.
pub const ELD_ADMIN_TOKEN_ENV: &str = "ELD_ADMIN_TOKEN";

const BLOCKCHAIN_MAX_NOTE: &str = "max_storable_bytes_on_same_volume = bytes_used + volume_available_bytes on the mount containing directory_path (naive ceiling if RocksDB were the only consumer of unprivileged free space).";

#[derive(Debug, Clone, Serialize)]
pub struct AdminNodeSection {
    pub chain_id: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminSignersSection {
    pub capacity_wallet_name: String,
    pub capacity_wallet_address: String,
}

/// ABCI application chain state on disk (RocksDB under `ELD_DB_PATH`).
#[derive(Debug, Clone, Serialize)]
pub struct AdminBlockchainStateSection {
    pub directory_path: String,
    pub bytes_used: u64,
    pub max_storable_bytes_on_same_volume: u64,
    pub volume_available_bytes: u64,
    pub note: String,
    /// Sum of `rocksdb.total-sst-files-size` for pinboard content column families (when available).
    pub rocksdb_pinboard_blob_cf_sst_bytes: Option<u64>,
    /// Sum of the same property for all other column families.
    pub rocksdb_other_cf_sst_bytes: Option<u64>,
    /// When per-CF SST breakdown is unavailable.
    pub rocksdb_cf_sst_breakdown_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminSlotKindStats {
    pub count: u64,
    pub bytes: u64,
}

/// Capacity provider slot map and directory footprint.
#[derive(Debug, Clone, Serialize)]
pub struct AdminCapacitySection {
    pub capacity_directory_path: String,
    pub capacity_directory_bytes: u64,
    pub slot_map_loaded: bool,
    pub reserved_capacity_bytes: u64,
    pub total_slot_count: u64,
    pub proof_slots: AdminSlotKindStats,
    pub open_slots: AdminSlotKindStats,
    pub content_slots: AdminSlotKindStats,
}

/// Host filesystem containing the app data paths (same mount as `statvfs_path`).
#[derive(Debug, Clone, Serialize)]
pub struct AdminHostSection {
    pub statvfs_path: String,
    pub volume_total_bytes: u64,
    pub volume_used_bytes: u64,
    pub volume_available_bytes: u64,
    pub eld_app_bytes_used: u64,
    pub eld_app_used_percent_of_volume: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminStatusResponse {
    pub node: AdminNodeSection,
    pub signers: AdminSignersSection,
    pub blockchain_state: AdminBlockchainStateSection,
    pub capacity: AdminCapacitySection,
    pub host: Option<AdminHostSection>,
    pub host_error: Option<String>,
    pub generated_at: String,
}

/// Slot counts and byte sums from the loaded slot map (passed into `spawn_blocking`).
#[derive(Debug, Clone)]
struct SlotsSnapshot {
    slot_map_loaded: bool,
    reserved_capacity_bytes: u64,
    total_slot_count: u64,
    proof_slots: AdminSlotKindStats,
    open_slots: AdminSlotKindStats,
    content_slots: AdminSlotKindStats,
}

impl SlotsSnapshot {
    fn empty() -> Self {
        Self {
            slot_map_loaded: false,
            reserved_capacity_bytes: 0,
            total_slot_count: 0,
            proof_slots: AdminSlotKindStats { count: 0, bytes: 0 },
            open_slots: AdminSlotKindStats { count: 0, bytes: 0 },
            content_slots: AdminSlotKindStats { count: 0, bytes: 0 },
        }
    }
}

fn snapshot_slots(allocator: &SlotAllocator) -> SlotsSnapshot {
    let Some(sm) = allocator.get_slot_map() else {
        return SlotsSnapshot::empty();
    };

    let mut proof_c = 0u64;
    let mut open_c = 0u64;
    let mut content_c = 0u64;
    let mut proof_b = 0u64;
    let mut open_b = 0u64;
    let mut content_b = 0u64;

    for slot in &sm.slots {
        let b = slot.size as u64;
        match &slot.state {
            SlotState::Proof => {
                proof_c = proof_c.saturating_add(1);
                proof_b = proof_b.saturating_add(b);
            }
            SlotState::Open => {
                open_c = open_c.saturating_add(1);
                open_b = open_b.saturating_add(b);
            }
            SlotState::Content { .. } => {
                content_c = content_c.saturating_add(1);
                content_b = content_b.saturating_add(b);
            }
        }
    }

    SlotsSnapshot {
        slot_map_loaded: true,
        reserved_capacity_bytes: sm.capacity_bytes,
        total_slot_count: sm.slots.len() as u64,
        proof_slots: AdminSlotKindStats {
            count: proof_c,
            bytes: proof_b,
        },
        open_slots: AdminSlotKindStats {
            count: open_c,
            bytes: open_b,
        },
        content_slots: AdminSlotKindStats {
            count: content_c,
            bytes: content_b,
        },
    }
}

pub(crate) enum AdminError {
    Unauthorized,
    Internal(String),
}

impl IntoResponse for AdminError {
    fn into_response(self) -> Response {
        match self {
            AdminError::Unauthorized => StatusCode::UNAUTHORIZED.into_response(),
            AdminError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        }
    }
}

fn check_bearer(headers: &HeaderMap, expected: &Option<String>) -> Result<(), AdminError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let auth = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(AdminError::Unauthorized)?;
    let prefix = "Bearer ";
    if !auth.starts_with(prefix) {
        return Err(AdminError::Unauthorized);
    }
    let token = auth[prefix.len()..].trim();
    if token != expected.as_str() {
        return Err(AdminError::Unauthorized);
    }
    Ok(())
}

/// `GET /admin/v1/status` — uses [`ApiState`] sub-extracts (see [`crate::api::init_router_with_storage`]).
pub(crate) async fn handle_admin_status(
    State(storage): State<Arc<RocksDBStorage>>,
    State(consensus_config): State<Arc<Mutex<ConsensusConfig>>>,
    State(capacity_manager): State<Arc<CapacityManager>>,
    State(admin_token): State<Option<String>>,
    headers: HeaderMap,
) -> Result<Json<AdminStatusResponse>, AdminError> {
    check_bearer(&headers, &admin_token)?;

    let chain_id = consensus_config
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .chain_id
        .clone();

    let wallet_name = capacity_manager.capacity_wallet_name().to_string();
    let wallet_addr = capacity_manager.config().provider_id;
    let capacity_dir = capacity_manager.config().capacity_dir.clone();

    let slot_allocator = capacity_manager.slot_allocator();
    let slots = {
        let alloc = slot_allocator.lock().await;
        snapshot_slots(&alloc)
    };

    let storage = storage.clone();

    let response = tokio::task::spawn_blocking(move || {
        build_admin_status_blocking(
            &storage,
            capacity_dir,
            slots,
            chain_id,
            wallet_name,
            wallet_addr,
        )
    })
    .await
    .map_err(|e| AdminError::Internal(format!("join error: {e}")))?
    .map_err(|e| AdminError::Internal(e.to_string()))?;

    Ok(Json(response))
}

fn build_admin_status_blocking(
    storage: &RocksDBStorage,
    capacity_dir: PathBuf,
    slots: SlotsSnapshot,
    chain_id: String,
    wallet_name: String,
    wallet_addr: Address,
) -> Result<AdminStatusResponse, EldError> {
    let rocks = storage.admin_rocksdb_storage_stats()?;
    let capacity_directory_bytes =
        directory_tree_size_bytes(&capacity_dir).map_err(|e| EldError::FileSystemError {
            operation: "capacity_directory_size".to_string(),
            path: capacity_dir.display().to_string(),
            details: e.to_string(),
        })?;

    let eld_app_bytes_used = rocks
        .directory_bytes
        .saturating_add(capacity_directory_bytes);

    let (host, host_error, volume_available_bytes, max_storable_bytes_on_same_volume) =
        match statvfs_for_path(&rocks.path) {
            Ok(vol) => {
                let max_storable = rocks.directory_bytes.saturating_add(vol.available_bytes);
                let pct = if vol.total_bytes > 0 {
                    Some((eld_app_bytes_used as f64 / vol.total_bytes as f64) * 100.0_f64)
                } else {
                    None
                };
                (
                    Some(AdminHostSection {
                        statvfs_path: rocks.path.display().to_string(),
                        volume_total_bytes: vol.total_bytes,
                        volume_used_bytes: vol.used_bytes,
                        volume_available_bytes: vol.available_bytes,
                        eld_app_bytes_used,
                        eld_app_used_percent_of_volume: pct,
                    }),
                    None,
                    vol.available_bytes,
                    max_storable,
                )
            }
            Err(e) => {
                warn!(error = %e, "Admin status: statvfs failed");
                (None, Some(e.to_string()), 0u64, rocks.directory_bytes)
            }
        };

    let blockchain_state = AdminBlockchainStateSection {
        directory_path: rocks.path.display().to_string(),
        bytes_used: rocks.directory_bytes,
        max_storable_bytes_on_same_volume,
        volume_available_bytes,
        note: BLOCKCHAIN_MAX_NOTE.to_string(),
        rocksdb_pinboard_blob_cf_sst_bytes: rocks.pinboard_blob_cf_sst_bytes,
        rocksdb_other_cf_sst_bytes: rocks.other_cf_sst_bytes,
        rocksdb_cf_sst_breakdown_error: rocks.cf_sst_breakdown_error,
    };

    let capacity = AdminCapacitySection {
        capacity_directory_path: capacity_dir.display().to_string(),
        capacity_directory_bytes,
        slot_map_loaded: slots.slot_map_loaded,
        reserved_capacity_bytes: slots.reserved_capacity_bytes,
        total_slot_count: slots.total_slot_count,
        proof_slots: slots.proof_slots.clone(),
        open_slots: slots.open_slots.clone(),
        content_slots: slots.content_slots.clone(),
    };

    Ok(AdminStatusResponse {
        node: AdminNodeSection {
            chain_id,
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        signers: AdminSignersSection {
            capacity_wallet_name: wallet_name,
            capacity_wallet_address: wallet_addr.hex_with_prefix(),
        },
        blockchain_state,
        capacity,
        host,
        host_error,
        generated_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_rejects_missing_header_when_token_set() {
        let headers = HeaderMap::new();
        let token = Some("secret".to_string());
        assert!(check_bearer(&headers, &token).is_err());
    }

    #[test]
    fn bearer_accepts_valid_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer secret"),
        );
        let token = Some("secret".to_string());
        assert!(check_bearer(&headers, &token).is_ok());
    }

    #[test]
    fn bearer_allows_when_token_unset() {
        let headers = HeaderMap::new();
        assert!(check_bearer(&headers, &None).is_ok());
    }
}
