use crate::app_state::app_state_snapshot::AppStateSnapshot;
use crate::app_state::envelope::AppStateEnvelope;
use crate::app_state::tip::AppStateTip;
use crate::errors::handle_fatal_eld_error;
use crate::storage::traits::CADOStorage;
use eld_common::cado::{CadoPath, CadoPathKey, CadoType};
use eld_common::error::EldError;
use eld_common::storage::AccountStorage;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AppState {
    pub app_hash: Vec<u8>,
    pub envelope: AppStateEnvelope,
    pub chain_id: String,
}

impl PartialEq for AppState {
    fn eq(&self, other: &Self) -> bool {
        if self.app_hash != other.app_hash || self.chain_id != other.chain_id {
            return false;
        }
        if self.envelope.state_trie.root_hash() != other.envelope.state_trie.root_hash() {
            return false;
        }

        // Compare the full serialized envelope (state_trie is skipped by serde and compared above).
        match (
            bincode::serialize(&self.envelope),
            bincode::serialize(&other.envelope),
        ) {
            (Ok(lhs), Ok(rhs)) => lhs == rhs,
            _ => false,
        }
    }
}

impl AppState {
    /// True when RocksDB has a committed `AppStateTip` at the fixed `LATEST` path.
    ///
    /// `Commit` persists `AppStateTip` via `put_cado_type` (path_index), not `cado_map`.
    pub fn has_persisted_app_state_tip(storage: &impl CADOStorage) -> Result<bool, EldError> {
        use eld_common::constants::cado::LATEST;

        let latest_app_state_tip_path =
            CadoPath::new(CadoType::AppStateTip, CadoPathKey::Name(LATEST))?;
        Ok(storage
            .get_cado_by_path(latest_app_state_tip_path)?
            .is_some())
    }

    pub fn initialize_with_data(
        &mut self,
        storage: &(impl AccountStorage + CADOStorage),
    ) -> Result<(), EldError> {
        self.envelope.init_empty_trie();
        use eld_common::constants::cado::LATEST;

        let latest_app_state_tip_path =
            match CadoPath::new(CadoType::AppStateTip, CadoPathKey::Name(LATEST)) {
                Ok(path) => path,
                Err(e) => handle_fatal_eld_error(e),
            };

        match storage.get_deserialized_cado_by_path::<AppStateTip>(latest_app_state_tip_path) {
            Ok(app_state_tip) => {
                self.app_hash = app_state_tip.app_hash.clone();
                self.envelope.block_height = app_state_tip.block_height;

                info!(
                    block_height = app_state_tip.block_height,
                    "Restored AppStateTip from RocksDB"
                );

                // Restore epoch app state snapshot from storage
                info!("Attempting to restore app state snapshot from storage...");
                let app_state_snapshot_path = match AppStateSnapshot::latest_path() {
                    Ok(path) => path,
                    Err(e) => {
                        error!("Failed to create app state snapshot path: {}", e);
                        handle_fatal_eld_error(e);
                    }
                };

                match storage.get_deserialized_cado_by_path::<AppStateSnapshot>(
                    app_state_snapshot_path.clone(),
                ) {
                    Ok(snapshot) => {
                        info!(
                            "Found app state snapshot at: {}",
                            app_state_snapshot_path.as_str()
                        );

                        if snapshot.block_height != app_state_tip.block_height {
                            handle_fatal_eld_error(EldError::ValidationError {
                                field: "app_state_snapshot.block_height".to_string(),
                                value: snapshot.block_height.to_string(),
                                details: format!(
                                    "AppStateSnapshot height {} does not match AppStateTip height {}",
                                    snapshot.block_height, app_state_tip.block_height
                                ),
                            });
                        }

                        if snapshot.app_hash != app_state_tip.app_hash {
                            handle_fatal_eld_error(EldError::ValidationError {
                                field: "app_state_snapshot.app_hash".to_string(),
                                value: hex::encode(&snapshot.app_hash),
                                details: "AppStateSnapshot app_hash does not match AppStateTip"
                                    .to_string(),
                            });
                        }

                        if let Err(e) = snapshot.apply_to_state(self, app_state_tip.cado_root_hash)
                        {
                            handle_fatal_eld_error(e);
                        }

                        info!("State trie root verified successfully");
                    }
                    Err(EldError::NotFoundError { .. }) => {
                        error!(
                            "No app state snapshot found at: {}",
                            app_state_snapshot_path.as_str()
                        );
                        handle_fatal_eld_error(EldError::StorageError {
                            operation: "load_app_state_snapshot".to_string(),
                            details: format!(
                                "Critical: App state snapshot missing from storage. Expected at: {}. \
                                 This indicates incomplete state persistence or database corruption. \
                                 To recover, either restore from backup or start with clean data.",
                                app_state_snapshot_path.as_str()
                            ),
                        });
                    }
                    Err(e) => {
                        error!("Error loading app state snapshot from storage: {}", e);
                        handle_fatal_eld_error(EldError::StorageError {
                            operation: "load_app_state_snapshot".to_string(),
                            details: format!(
                                "Critical: Cannot restore app state from snapshot at {}: {}",
                                app_state_snapshot_path.as_str(),
                                e
                            ),
                        });
                    }
                }

                info!("Restored block height: {}", self.envelope.block_height,);
            }
            Err(EldError::NotFoundError { .. }) => {
                info!("No latest AppStateTip found in DB, initializing with genesis state");
                let genesis_state = AppStateTip::genesis();
                self.app_hash = genesis_state.app_hash;
                self.envelope.block_height = genesis_state.block_height;
                info!("Genesis state initialized with empty trie");
            }
            Err(e) => return Err(e),
        }

        // Validate app_hash is not empty
        if self.app_hash.is_empty() {
            return Err(EldError::ValidationError {
                field: "app_hash".to_string(),
                value: "empty".to_string(),
                details: "App hash cannot be empty".to_string(),
            });
        }

        Ok(())
    }
}
