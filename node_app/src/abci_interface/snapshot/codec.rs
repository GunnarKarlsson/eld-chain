use crate::app_state::app_state_snapshot::AppStateSnapshot;
use eld_common::error::EldError;
use serde::{Deserialize, Serialize};

pub const SNAPSHOT_FORMAT_VERSION: u32 = 2;

/// ABCI snapshot transport payload built from canonical persisted state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbciSnapshot {
    pub version: u32,
    pub app_state_snapshot: AppStateSnapshot,
}

impl AbciSnapshot {
    pub fn new(app_state_snapshot: AppStateSnapshot) -> Self {
        Self {
            version: SNAPSHOT_FORMAT_VERSION,
            app_state_snapshot,
        }
    }
}

pub fn serialize_abci_snapshot(snapshot: &AbciSnapshot) -> Result<Vec<u8>, EldError> {
    bincode::serialize(snapshot).map_err(|e| EldError::StorageError {
        operation: "serialize_abci_snapshot".to_string(),
        details: e.to_string(),
    })
}

#[cfg(test)]
pub fn deserialize_abci_snapshot(bytes: &[u8]) -> Result<AbciSnapshot, EldError> {
    let snapshot: AbciSnapshot =
        bincode::deserialize(bytes).map_err(|e| EldError::StorageError {
            operation: "deserialize_abci_snapshot".to_string(),
            details: e.to_string(),
        })?;

    if snapshot.version != SNAPSHOT_FORMAT_VERSION {
        return Err(EldError::ValidationError {
            field: "snapshot.version".to_string(),
            value: snapshot.version.to_string(),
            details: format!(
                "Unsupported snapshot format version. Expected {}, got {}",
                SNAPSHOT_FORMAT_VERSION, snapshot.version
            ),
        });
    }

    Ok(snapshot)
}
