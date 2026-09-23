use crate::app_state::committed_cado_cache::CommittedCadoCache;
use crate::app_state::AppState;
use eld_common::cado::CadoType;
use eld_common::cado::{CADOMetadata, CadoBody, CadoPath, CadoPathKey, DeserializableBin};
use eld_common::error::EldError;
use eld_common::namespace::NamespaceRecord;
use eld_common::validator::{ActiveCapacityValidator, CapacityValidatorInfo, ValidatorInfo};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use tracing::{error, info};

/// Canonical epoch-boundary snapshot of committed [`AppState`](crate::app_state::AppState) fields.
///
/// Persisted as a CADO at epoch boundaries. Staging caches (`cado_cache`, pinboard staging,
/// `namespace_registry_cache`, etc.) and `pending_fee_rewards` are omitted — they are empty
/// after commit. `state_trie` is omitted — rebuild from [`Self::committed_cado_cache`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStateSnapshot {
    pub block_height: i64,
    pub app_hash: [u8; 32],
    pub chain_id: String,
    pub current_epoch: i64,
    pub committed_cado_cache: CommittedCadoCache,
    /// Merkle Patricia Trie root (`AppStateTip.cado_root_hash`) at snapshot time.
    pub state_trie_root: [u8; 32],
    pub epoch_records_index: BTreeSet<i64>,
    pub namespace_registry_index: BTreeMap<String, NamespaceRecord>,
    pub validators: Vec<ValidatorInfo>,
    pub active_validators: Vec<ValidatorInfo>,
    #[serde(alias = "capacity_providers")]
    pub capacity_validators: Vec<CapacityValidatorInfo>,
    #[serde(alias = "active_storage_validator")]
    pub active_capacity_validator: Option<ActiveCapacityValidator>,
}

impl AppStateSnapshot {
    /// Builds a snapshot from committed `state` at epoch boundary.
    pub fn new(state: &AppState) -> Result<Self, EldError> {
        let envelope = &state.envelope;
        let state_trie_root = envelope.state_trie.root_hash();
        let app_hash = state.app_hash().bytes()?;
        Ok(Self {
            block_height: envelope.block_height,
            app_hash,
            chain_id: state.chain_id.clone(),
            current_epoch: envelope.current_epoch,
            committed_cado_cache: envelope.committed_cado_cache.without_infrastructure(),
            state_trie_root,
            epoch_records_index: envelope.epoch_records_index.clone(),
            namespace_registry_index: envelope.namespace_registry_index.clone(),
            validators: envelope.validators.clone(),
            active_validators: envelope.active_validators.clone(),
            capacity_validators: envelope.capacity_validators.clone(),
            active_capacity_validator: envelope.active_capacity_validator.clone(),
        })
    }

    /// Restores committed snapshot fields into `state` and validates trie roots.
    pub fn apply_to_state(
        &self,
        state: &mut AppState,
        cado_root_hash: [u8; 32],
    ) -> Result<(), EldError> {
        state.set_app_hash(self.app_hash);
        state.chain_id = self.chain_id.clone();
        state.envelope.block_height = self.block_height;
        state.envelope.current_epoch = self.current_epoch;
        state.envelope.committed_cado_cache = self.committed_cado_cache.without_infrastructure();
        state.envelope.epoch_records_index = self.epoch_records_index.clone();
        state.envelope.namespace_registry_index = self.namespace_registry_index.clone();
        state.envelope.validators = self.validators.clone();
        state.envelope.active_validators = self.active_validators.clone();
        state.envelope.capacity_validators = self.capacity_validators.clone();
        state.envelope.active_capacity_validator = self.active_capacity_validator.clone();

        state.envelope.rebuild_state_trie_from_cache();
        let rebuilt_root = state.envelope.state_trie.root_hash();

        info!(
            block_height = self.block_height,
            state_trie_root = %hex::encode(rebuilt_root),
            "Restored AppStateSnapshot committed cache and envelope fields"
        );

        if rebuilt_root != cado_root_hash {
            error!(
                "State trie root mismatch! AppStateTip: 0x{}, Rebuilt: 0x{}",
                hex::encode(cado_root_hash),
                hex::encode(rebuilt_root)
            );
            return Err(EldError::ValidationError {
                field: "trie_root_hash".to_string(),
                value: hex::encode(rebuilt_root),
                details: format!(
                    "Critical: Rebuilt state trie root (0x{}) does not match AppStateTip (0x{}). \
                     Wipe volumes or restore from backup.",
                    hex::encode(rebuilt_root),
                    hex::encode(cado_root_hash)
                ),
            });
        }

        if self.state_trie_root != cado_root_hash {
            return Err(EldError::ValidationError {
                field: "state_trie_root".to_string(),
                value: hex::encode(self.state_trie_root),
                details: format!(
                    "Snapshot state_trie_root (0x{}) does not match AppStateTip (0x{})",
                    hex::encode(self.state_trie_root),
                    hex::encode(cado_root_hash)
                ),
            });
        }

        Ok(())
    }

    /// Serializes the snapshot to bytes.
    pub fn serialize(&self) -> Result<Vec<u8>, EldError> {
        bincode::serialize(self).map_err(|e| EldError::StorageError {
            operation: "serialize_app_state_snapshot".to_string(),
            details: e.to_string(),
        })
    }

    /// Deserializes a snapshot from bytes.
    pub fn deserialize(data: &[u8]) -> Result<Self, EldError> {
        bincode::deserialize(data).map_err(|e| EldError::StorageError {
            operation: "deserialize_app_state_snapshot".to_string(),
            details: e.to_string(),
        })
    }

    /// Same as [`Self::deserialize`] — CADO payload bytes for [`CadoType::AppStateSnapshot`].
    pub fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        Self::deserialize(data)
    }

    /// Converts the snapshot to a CADO for storage.
    pub fn to_cado(&self) -> Result<CadoBody, EldError> {
        let serialized = self.serialize()?;
        let meta = CADOMetadata::new(CadoType::AppStateSnapshot, "system");
        Ok(CadoBody::immutable(serialized, meta))
    }

    /// CADO path for storing the snapshot (hash of `identifier`, e.g. `"latest"`).
    pub fn create_path(identifier: &str) -> Result<CadoPath, EldError> {
        let hash = Sha256::digest(identifier.as_bytes());
        let name_hex = format!("0x{}", hex::encode(hash));
        CadoPath::new(CadoType::AppStateSnapshot, CadoPathKey::Name(&name_hex))
    }

    /// Path for the latest epoch app state snapshot.
    pub fn latest_path() -> Result<CadoPath, EldError> {
        Self::create_path("latest")
    }
}

impl DeserializableBin for AppStateSnapshot {
    fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        AppStateSnapshot::deserialize_bin(data)
    }
}

#[cfg(test)]
impl AppStateSnapshot {
    /// Restores from a CADO envelope (payload bytes only).
    #[allow(dead_code)]
    pub fn from_cado(cado: CadoBody) -> Result<Self, EldError> {
        let data = match cado {
            CadoBody::Immutable(cado) => cado.data().to_vec(),
            CadoBody::Mutable(cado_mut) => cado_mut.data().to_vec(),
        };

        Self::deserialize(&data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use ed25519_dalek::SigningKey;
    use eld_common::address::Address;
    use eld_common::cado::{CADOMetadata, CadoBody, CadoPath, CadoType};
    use eld_common::coin::Coin;
    use eld_common::constants::cado::PATH_PREFIX_ACCOUNT;
    use eld_common::public_key::PublicKey;

    #[allow(clippy::field_reassign_with_default)]
    fn sample_app_state_for_snapshot() -> AppState {
        let mut state = AppState::default();
        state.set_app_hash([0xAB; 32]);
        state.chain_id = "test-chain".to_string();
        state.envelope.init_empty_trie();
        state.envelope.block_height = 40;
        state.envelope.current_epoch = 2;

        let account_path = CadoPath::parse(&format!(
            "{}{}",
            PATH_PREFIX_ACCOUNT, "0x1234567890123456789012345678901234567890"
        ))
        .expect("account path");
        let account_cado = CadoBody::mutable_new(
            vec![1, 2, 3, 4],
            CADOMetadata::new(
                CadoType::Account,
                "0x1234567890123456789012345678901234567890",
            ),
        );
        let account_hash = account_cado.content_hash();
        state
            .envelope
            .committed_cado_cache
            .insert(account_path.as_str().as_bytes(), account_cado);
        state
            .envelope
            .state_trie
            .insert(account_path.as_str().as_bytes(), &account_hash);

        state.envelope.epoch_records_index = [0, 2, 5].into_iter().collect();

        let owner =
            Address::parse_hex_str("0xe17404c417fa10cc04fdf73604fcacca8d0a687c").expect("address");
        state.envelope.namespace_registry_index.insert(
            "alpha".to_string(),
            NamespaceRecord {
                namespace_slug: "alpha".to_string(),
                owner,
                registered_height: 10,
            },
        );

        let validator = ValidatorInfo {
            address: Address::parse_hex_str("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .expect("address"),
            stake: Coin::new(1_000).expect("stake"),
            public_key: vec![0x11; 32],
        };
        state.envelope.validators = vec![validator.clone()];
        state.envelope.active_validators = vec![validator];

        let capacity_provider = CapacityValidatorInfo {
            address: Address::parse_hex_str("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
                .expect("address"),
            stake: Coin::new(500).expect("capacity stake"),
            public_key: PublicKey::from(SigningKey::from_bytes(&[0x44; 32]).verifying_key()),
            storage_capacity: 1_024 * 1_024,
            merkle_root: Some([0x22; 32]),
            seed: Some([0x33; 32]),
            chunk_count: Some(16),
            registered_at: Some(39),
            last_merkle_root_update: Some(40),
            registered_block: 30,
            registration_duration: 100,
        };
        state.envelope.capacity_validators = vec![capacity_provider];

        state.envelope.active_capacity_validator = Some(ActiveCapacityValidator {
            validator_address: Address::parse_hex_str("0xcccccccccccccccccccccccccccccccccccccccc")
                .expect("address"),
            epoch: 2,
            challenged_providers: vec![Address::parse_hex_str(
                "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            )
            .expect("address")],
            subscribed_topics: vec!["capacity.proof".to_string()],
        });

        state
    }

    fn assert_app_state_snapshot_fields_equal(
        expected: &AppStateSnapshot,
        restored: &AppStateSnapshot,
    ) {
        assert_eq!(restored.block_height, expected.block_height);
        assert_eq!(restored.app_hash, expected.app_hash);
        assert_eq!(restored.chain_id, expected.chain_id);
        assert_eq!(restored.current_epoch, expected.current_epoch);
        assert_eq!(restored.state_trie_root, expected.state_trie_root);
        assert_eq!(
            bincode::serialize(&restored.committed_cado_cache).expect("serialize restored cache"),
            bincode::serialize(&expected.committed_cado_cache).expect("serialize expected cache")
        );
        assert_eq!(restored.epoch_records_index, expected.epoch_records_index);
        assert_eq!(
            restored.namespace_registry_index,
            expected.namespace_registry_index
        );
        assert_eq!(
            bincode::serialize(&restored.validators).expect("serialize restored validators"),
            bincode::serialize(&expected.validators).expect("serialize expected validators")
        );
        assert_eq!(
            bincode::serialize(&restored.active_validators)
                .expect("serialize restored active_validators"),
            bincode::serialize(&expected.active_validators)
                .expect("serialize expected active_validators")
        );
        assert_eq!(
            bincode::serialize(&restored.capacity_validators)
                .expect("serialize restored capacity_validators"),
            bincode::serialize(&expected.capacity_validators)
                .expect("serialize expected capacity_validators")
        );
        assert_eq!(
            bincode::serialize(&restored.active_capacity_validator)
                .expect("serialize restored active_capacity_validator"),
            bincode::serialize(&expected.active_capacity_validator)
                .expect("serialize expected active_capacity_validator")
        );
    }

    #[test]
    fn app_state_snapshot_to_cado_roundtrip() {
        use eld_common::constants::cado::TYPE_APP_STATE_SNAPSHOT;

        let state = sample_app_state_for_snapshot();
        let snapshot = AppStateSnapshot::new(&state).expect("snapshot");
        let cado = snapshot.to_cado().expect("to_cado");

        match &cado {
            CadoBody::Immutable(cado) => {
                assert_eq!(cado.metadata().type_(), TYPE_APP_STATE_SNAPSHOT);
            }
            _ => panic!("expected immutable CADO"),
        }

        let restored = AppStateSnapshot::deserialize_bin(cado.data()).expect("deserialize");
        assert_app_state_snapshot_fields_equal(&snapshot, &restored);
        assert_eq!(
            bincode::serialize(&snapshot).expect("serialize original"),
            bincode::serialize(&restored).expect("serialize restored")
        );
    }

    #[test]
    fn app_state_snapshot_excludes_infrastructure_from_committed_cache() {
        let mut state = AppState::default();
        state.envelope.init_empty_trie();
        let test_cado =
            CadoBody::mutable_new(vec![1, 2, 3], CADOMetadata::new(CadoType::Account, "test"));
        state
            .envelope
            .committed_cado_cache
            .insert(b"test_key", test_cado.clone());
        let hash = test_cado.content_hash();
        state.envelope.state_trie.insert(b"test_key", &hash);

        let infra_path = AppStateSnapshot::latest_path().expect("infra path");
        let infra_cado = CadoBody::immutable(
            vec![9, 9, 9],
            CADOMetadata::new(CadoType::AppStateSnapshot, "system"),
        );
        state
            .envelope
            .committed_cado_cache
            .insert(infra_path.as_str().as_bytes(), infra_cado);

        assert_eq!(state.envelope.committed_cado_cache.len(), 2);

        state.set_app_hash([0xAB; 32]);
        let snapshot = AppStateSnapshot::new(&state).expect("snapshot");
        assert_eq!(snapshot.committed_cado_cache.len(), 1);
        assert!(snapshot.committed_cado_cache.get(b"test_key").is_some());
        assert!(snapshot
            .committed_cado_cache
            .get(infra_path.as_str().as_bytes())
            .is_none());
    }
}
