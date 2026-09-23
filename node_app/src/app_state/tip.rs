use crate::app_state::state_trie::StateTrie;
use eld_common::error::EldError;
use serde::{Deserialize, Serialize};
use sha2::Digest;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStateTip {
    pub block_height: i64,
    pub cado_root_hash: [u8; 32],
    pub app_hash: [u8; 32],
}

impl PartialEq for AppStateTip {
    fn eq(&self, other: &Self) -> bool {
        self.block_height == other.block_height
            && self.cado_root_hash == other.cado_root_hash
            && self.app_hash == other.app_hash
    }
}

impl std::fmt::Display for AppStateTip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "AppStateTip at block height {}", self.block_height)?;
        writeln!(
            f,
            "  CADO Root Hash: 0x{}",
            hex::encode(self.cado_root_hash)
        )?;
        writeln!(f, "  App Hash: 0x{}", hex::encode(self.app_hash))
    }
}

impl AppStateTip {
    pub fn genesis() -> Self {
        Self {
            block_height: 0,
            cado_root_hash: StateTrie::empty_root_hash(),
            app_hash: sha2::Sha256::digest("genesis").into(),
        }
    }

    /// Deserializes from [`CadoType::AppStateTip`] CADO payload bytes.
    pub fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        bincode::deserialize(data).map_err(|e| EldError::StorageError {
            operation: "deserialize_app_state_tip".to_string(),
            details: format!("Failed to deserialize AppStateTip: {e}"),
        })
    }
}

impl eld_common::cado::DeserializableBin for AppStateTip {
    fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        AppStateTip::deserialize_bin(data)
    }
}
