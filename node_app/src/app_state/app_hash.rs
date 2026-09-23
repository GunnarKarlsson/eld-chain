use eld_common::error::EldError;
use serde::{Deserialize, Serialize};

/// In-memory application hash.
///
/// Starts unset. [`Self::set`] stores exactly 32 bytes and is the only way to fill `data`,
/// so a value cannot become empty again through this API. Replacing the whole `AppHash`
/// with [`Self::new`] is a different value; [`crate::app_state::AppState`] does not expose that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AppHash {
    data: Option<[u8; 32]>,
}

impl AppHash {
    /// Creates an unset hash.
    pub fn new() -> Self {
        Self { data: None }
    }

    /// True until [`Self::set`] has been called on this value.
    pub fn is_empty(&self) -> bool {
        self.data.is_none()
    }

    /// Stores a 32-byte hash. Later calls replace those bytes and leave the hash set.
    pub fn set(&mut self, new_value: [u8; 32]) {
        self.data = Some(new_value);
    }

    /// Shared view of the 32-byte hash when set.
    pub fn get(&self) -> Option<&[u8; 32]> {
        self.data.as_ref()
    }

    /// The 32-byte hash, or an error while unset.
    pub fn bytes(&self) -> Result<[u8; 32], EldError> {
        self.data.ok_or_else(|| EldError::ValidationError {
            field: "app_hash".to_string(),
            value: "empty".to_string(),
            details: "App hash cannot be empty".to_string(),
        })
    }

    /// ABCI byte vector: 32 bytes when set, empty while unset.
    pub fn to_vec(self) -> Vec<u8> {
        self.data.map(|bytes| bytes.to_vec()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::{AppState, AppStateTip};

    #[test]
    fn unset_app_hash_roundtrips_through_bincode() {
        let hash = AppHash::new();
        let bytes = bincode::serialize(&hash).expect("serialize unset");
        let restored: AppHash = bincode::deserialize(&bytes).expect("deserialize unset");
        assert!(restored.is_empty());
        assert!(restored.get().is_none());
        assert!(restored.bytes().is_err());
        assert!(restored.to_vec().is_empty());
    }

    #[test]
    fn set_app_hash_roundtrips_through_bincode() {
        let mut hash = AppHash::new();
        let value = [0xABu8; 32];
        hash.set(value);
        hash.set([0xCDu8; 32]);

        let bytes = bincode::serialize(&hash).expect("serialize set");
        let restored: AppHash = bincode::deserialize(&bytes).expect("deserialize set");
        assert!(!restored.is_empty());
        assert_eq!(restored.get(), Some(&[0xCDu8; 32]));
        assert_eq!(restored.bytes().expect("bytes"), [0xCDu8; 32]);
        assert_eq!(restored.to_vec(), vec![0xCDu8; 32]);
    }

    #[test]
    fn deserialize_rejects_truncated_set_app_hash() {
        let mut hash = AppHash::new();
        hash.set([0xABu8; 32]);
        let mut bytes = bincode::serialize(&hash).expect("serialize");
        bytes.pop();
        assert!(bincode::deserialize::<AppHash>(&bytes).is_err());
    }

    #[test]
    fn app_state_app_hash_roundtrips_through_bincode() {
        let unset = AppState::default();
        let unset_bytes = bincode::serialize(&unset).expect("serialize unset state");
        let restored_unset: AppState =
            bincode::deserialize(&unset_bytes).expect("deserialize unset state");
        assert!(restored_unset.app_hash().is_empty());

        let mut set = AppState::default();
        set.set_app_hash([0x11u8; 32]);
        let set_bytes = bincode::serialize(&set).expect("serialize set state");
        let restored_set: AppState =
            bincode::deserialize(&set_bytes).expect("deserialize set state");
        assert_eq!(restored_set.app_hash().get(), Some(&[0x11u8; 32]));
    }

    #[test]
    fn app_state_tip_app_hash_roundtrips_as_32_bytes() {
        let tip = AppStateTip {
            block_height: 7,
            cado_root_hash: [1u8; 32],
            app_hash: [2u8; 32],
        };
        let bytes = bincode::serialize(&tip).expect("serialize tip");
        let restored: AppStateTip = bincode::deserialize(&bytes).expect("deserialize tip");
        assert_eq!(restored, tip);
        assert_eq!(restored.app_hash.len(), 32);

        let mut truncated = bytes.clone();
        truncated.pop();
        assert!(bincode::deserialize::<AppStateTip>(&truncated).is_err());
    }
}
