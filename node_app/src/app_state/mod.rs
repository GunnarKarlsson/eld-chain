use crate::app_state::app_state_snapshot::AppStateSnapshot;
use crate::app_state::committed_cado_cache::CommittedCadoCache;
use crate::app_state::state_trie::StateTrie;
use crate::errors::{handle_fatal_eld_error, handle_recoverable_eld_error};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use tracing::{error, info, warn};
pub mod app_state_snapshot;
pub mod committed_cado_cache;
pub mod nibbles;
pub mod state_trie;

use crate::storage::traits::{CADOStorage, VerifiedProofRewardDedupStorage};
use eld_common::account::Account;
use eld_common::cado::{
    epoch_from_record_path_name, CADOMarkedForDeletion, CadoBody, CadoPath, CadoPathKey, CadoType,
};
use eld_common::coin::Coin;
use eld_common::constants::cado::PATH_PREFIX_EPOCH_RECORD;
use eld_common::constants::cado::PATH_PREFIX_NAMESPACE_REGISTRY;
use eld_common::error::EldError;
use eld_common::namespace::{
    normalize_namespace_slug, slug_to_namespace_cadopath, NamespaceRecord,
};
use eld_common::pinboard::PinboardMessageMetadata;
use eld_common::staking_account::StakingAccount;
use eld_common::storage::AccountStorage;
use eld_common::validation::{safe_deserialize_account_data, safe_deserialize_cado_data};
use eld_common::validator::CapacityValidatorInfo;
use eld_common::validator::{ActiveCapacityValidator, ValidatorInfo};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStateTip {
    pub block_height: i64,
    pub cado_root_hash: [u8; 32],
    pub app_hash: Vec<u8>, // TODO: Change to [u8; 32]
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
        writeln!(f, "  App Hash: 0x{}", hex::encode(&self.app_hash))
    }
}

impl AppStateTip {
    pub fn genesis() -> Self {
        Self {
            block_height: 0,
            cado_root_hash: StateTrie::empty_root_hash(),
            app_hash: sha2::Sha256::digest("genesis").to_vec(),
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

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AppStateEnvelope {
    // CASHING
    // caches
    pub cado_cache: BTreeMap<String, CadoBody>,
    // cados to delete
    pub cado_cache_to_delete: Vec<CADOMarkedForDeletion>,
    // pinboard (PostMessage) caches - written during consensus commit
    pub pinboard_meta_cache: BTreeMap<String, PinboardMessageMetadata>,
    /// Blob refcount updates staged during the block (content_key -> delta).
    pub pinboard_refcount_deltas: HashMap<String, i64>,
    /// Secondary index entries to insert (wallet, committed_height, message_id).
    pub pinboard_idx_wallet_add: Vec<(String, u64, String)>,
    /// Secondary index entries to insert (tag, committed_height, message_id).
    pub pinboard_idx_tag_add: Vec<(String, u64, String)>,
    /// Expiry index entries to insert (expires_height, message_id).
    pub pinboard_idx_expiry_add: Vec<(u64, String)>,
    /// Global feed index entries to insert (committed_height, message_id).
    pub pinboard_idx_commit_add: Vec<(u64, String)>,
    /// VerifiedProof challenge IDs rewarded during the current block (flushed at commit).
    #[serde(default)]
    pub verified_proof_rewarded_cache: HashSet<String>,
    /// Namespace registrations staged during the block (flushed at commit).
    pub namespace_registry_cache: BTreeMap<String, NamespaceRecord>,
    /// Committed namespace registry records keyed by canonical slug.
    #[serde(default)]
    pub namespace_registry_index: BTreeMap<String, NamespaceRecord>,
    // validator rewards cache
    pub pending_fee_rewards: Coin, // fees accumulated during current block
    // HASHED STATE
    // general block info
    pub block_height: i64,
    pub current_epoch: i64,
    // committed cado cache (PatriciaMap lookup index — not the Merkle trie)
    pub committed_cado_cache: CommittedCadoCache,
    /// Epoch numbers with committed `EpochRecord` CADOs (excludes the `LATEST` alias).
    #[serde(default)]
    pub epoch_records_index: BTreeSet<i64>,
    /// Merkle Patricia Trie for `AppStateTip.cado_root_hash` and future proofs.
    #[serde(skip, default = "StateTrie::empty")]
    pub state_trie: StateTrie,

    // validator lists
    pub validators: Vec<ValidatorInfo>, // all potential validators
    pub active_validators: Vec<ValidatorInfo>, // Validators for current epoch

    // capacity validator registry (RegisterCapacity); one may be selected as active per epoch
    #[serde(alias = "capacity_providers", alias = "storage_providers")]
    pub capacity_validators: Vec<CapacityValidatorInfo>,

    /// Active capacity validator for the current epoch (challenges capacity validators).
    #[serde(alias = "active_storage_validator")]
    pub active_capacity_validator: Option<ActiveCapacityValidator>,
}

impl Hash for AppStateEnvelope {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.block_height.hash(state);
        self.current_epoch.hash(state);
        self.state_trie.root_hash().hash(state);

        for validator in &self.validators {
            validator.hash(state);
        }
        for validator in &self.active_validators {
            validator.hash(state);
        }
        for provider in &self.capacity_validators {
            provider.hash(state);
        }
        if let Some(ref storage_validator) = self.active_capacity_validator {
            storage_validator.hash(state);
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstanceWithCadoHash<I> {
    pub instance: I,
    pub hash: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct StakingAccountWithCadoHash {
    pub account: StakingAccount,
    pub hash: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct AccountWithCadoHash {
    pub account: Account,
    pub hash: [u8; 32],
}

impl AppStateEnvelope {
    pub fn update_cado_cache(&mut self, path: CadoPath, cado: CadoBody) {
        if path.is_infrastructure() {
            warn!(
                path = %path.as_str(),
                "Refusing to stage infrastructure CADO in cado_cache"
            );
            return;
        }
        let path_str = path.as_str();
        info!("INSERTING INFO CACHE: {}", path_str);
        self.cado_cache.insert(path_str.to_string(), cado);
    }

    /// True when `path` is in staged `cado_cache` or last-committed `committed_cado_cache`.
    pub fn has_cado_in_working_set(&self, path: &CadoPath) -> bool {
        let path_str = path.as_str();
        self.cado_cache.contains_key(path_str)
            || self.committed_cado_cache.get(path_str.as_bytes()).is_some()
    }

    /// True when this `challenge_id` was already rewarded in-block or in committed storage.
    pub fn is_verified_proof_challenge_rewarded<S: VerifiedProofRewardDedupStorage>(
        &self,
        storage: &S,
        challenge_id: &str,
    ) -> Result<bool, EldError> {
        if self.verified_proof_rewarded_cache.contains(challenge_id) {
            return Ok(true);
        }
        storage.is_verified_proof_challenge_rewarded(challenge_id)
    }

    // Key is device id or sender (both should be updated)

    pub fn calculate_hash(&self) -> u64 {
        let mut s = DefaultHasher::new();
        self.hash(&mut s);
        s.finish()
    }

    pub fn init_empty_trie(&mut self) {
        self.state_trie = StateTrie::empty();
        self.committed_cado_cache.calculate_hash();
    }

    /// Rebuilds the in-memory MPT from `committed_cado_cache` (Phase 1 snapshot stores root only).
    pub fn rebuild_state_trie_from_cache(&mut self) {
        self.state_trie = StateTrie::empty();
        for (path, cado) in self.committed_cado_cache.iter() {
            let hash = cado.content_hash();
            self.state_trie.insert(&path, &hash);
        }
    }

    /// Rebuilds [`Self::epoch_records_index`] from [`Self::committed_cado_cache`].
    pub fn rebuild_epoch_records_index(&mut self) {
        self.epoch_records_index =
            Self::epoch_numbers_from_committed_cache(&self.committed_cado_cache);
    }

    fn epoch_numbers_from_committed_cache(cache: &CommittedCadoCache) -> BTreeSet<i64> {
        let mut epochs = BTreeSet::new();
        for (path_bytes, _) in cache.iter() {
            let Ok(path_str) = std::str::from_utf8(&path_bytes) else {
                continue;
            };
            if let Some(epoch) = Self::epoch_number_from_record_path(path_str) {
                epochs.insert(epoch);
            }
        }
        epochs
    }

    fn epoch_number_from_record_path(path_str: &str) -> Option<i64> {
        if !path_str.starts_with(PATH_PREFIX_EPOCH_RECORD) {
            return None;
        }
        let name = path_str.strip_prefix(PATH_PREFIX_EPOCH_RECORD)?;
        epoch_from_record_path_name(name)
    }

    pub fn insert_epoch_records_index_from_path(&mut self, path_str: &str) {
        if let Some(epoch) = Self::epoch_number_from_record_path(path_str) {
            self.epoch_records_index.insert(epoch);
        }
    }

    /// Committed epoch numbers plus any epoch records staged in the current block.
    pub fn collect_epoch_numbers(&self, current: Option<&AppStateEnvelope>) -> BTreeSet<i64> {
        let mut epochs =
            if self.epoch_records_index.is_empty() && !self.committed_cado_cache.is_empty() {
                Self::epoch_numbers_from_committed_cache(&self.committed_cado_cache)
            } else {
                self.epoch_records_index.clone()
            };
        if let Some(current) = current {
            for path_str in current.cado_cache.keys() {
                if let Some(epoch) = Self::epoch_number_from_record_path(path_str) {
                    epochs.insert(epoch);
                }
            }
        }
        epochs
    }

    /// Rebuilds [`Self::namespace_registry_index`] from [`Self::committed_cado_cache`].
    pub fn rebuild_namespace_registry_index(&mut self) -> Result<(), EldError> {
        self.namespace_registry_index =
            Self::namespace_registry_index_from_committed_cache(&self.committed_cado_cache)?;
        Ok(())
    }

    fn namespace_slug_from_registry_path(path_str: &str) -> Option<String> {
        if !path_str.starts_with(PATH_PREFIX_NAMESPACE_REGISTRY) {
            return None;
        }
        let slug = path_str.strip_prefix(PATH_PREFIX_NAMESPACE_REGISTRY)?;
        if slug.is_empty() {
            return None;
        }
        Some(slug.to_string())
    }

    fn namespace_registry_index_from_committed_cache(
        cache: &CommittedCadoCache,
    ) -> Result<BTreeMap<String, NamespaceRecord>, EldError> {
        let mut index = BTreeMap::new();
        for (path_bytes, cado) in cache.iter() {
            let Ok(path_str) = std::str::from_utf8(&path_bytes) else {
                continue;
            };
            let Some(slug) = Self::namespace_slug_from_registry_path(path_str) else {
                continue;
            };
            let CadoBody::Immutable(immutable) = cado else {
                continue;
            };
            let record = NamespaceRecord::deserialize_bin(immutable.data())?;
            index.insert(slug, record);
        }
        Ok(index)
    }

    /// Committed namespace records plus any registrations staged in the current block.
    pub fn collect_namespace_records(
        &self,
        current: Option<&AppStateEnvelope>,
    ) -> Result<Vec<NamespaceRecord>, EldError> {
        let mut by_slug =
            if self.namespace_registry_index.is_empty() && !self.committed_cado_cache.is_empty() {
                Self::namespace_registry_index_from_committed_cache(&self.committed_cado_cache)?
            } else {
                self.namespace_registry_index.clone()
            };
        if let Some(current) = current {
            for (slug, record) in &current.namespace_registry_cache {
                by_slug.insert(slug.clone(), record.clone());
            }
        }
        Ok(by_slug.into_values().collect())
    }

    /// Layer B: registry path present in committed CADO cache (prior blocks).
    pub fn is_namespace_registered_on_chain(&self, namespace_slug: &str) -> Result<bool, EldError> {
        let path = slug_to_namespace_cadopath(namespace_slug)?;
        Ok(self
            .committed_cado_cache
            .get(path.as_str().as_bytes())
            .is_some())
    }

    /// Layer A + B: reject duplicate namespace registration.
    pub fn validate_add_namespace_not_taken(&self, namespace_slug: &str) -> Result<(), EldError> {
        let canonical = normalize_namespace_slug(namespace_slug)?;
        if self.namespace_registry_cache.contains_key(&canonical) {
            return Err(EldError::ValidationError {
                field: "namespace_slug".to_string(),
                value: canonical.clone(),
                details: "Namespace already registered in this block".to_string(),
            });
        }
        if self.is_namespace_registered_on_chain(&canonical)? {
            return Err(EldError::ValidationError {
                field: "namespace_slug".to_string(),
                value: canonical,
                details: "Namespace already registered on chain".to_string(),
            });
        }
        Ok(())
    }

    /// True if the namespace is staged this block or committed on chain.
    pub fn is_namespace_registered(&self, namespace_slug: &str) -> Result<bool, EldError> {
        let canonical = normalize_namespace_slug(namespace_slug)?;
        if self.namespace_registry_cache.contains_key(&canonical) {
            return Ok(true);
        }
        self.is_namespace_registered_on_chain(&canonical)
    }

    /// Resolves a namespace record from the block cache or committed registry CADO.
    pub fn resolve_namespace(
        &self,
        namespace_slug: &str,
    ) -> Result<Option<NamespaceRecord>, EldError> {
        let canonical = normalize_namespace_slug(namespace_slug)?;
        if let Some(record) = self.namespace_registry_cache.get(&canonical) {
            return Ok(Some(record.clone()));
        }
        let path = slug_to_namespace_cadopath(&canonical)?;
        let Some(cado) = self.committed_cado_cache.get(path.as_str().as_bytes()) else {
            return Ok(None);
        };
        let record = NamespaceRecord::deserialize_bin(cado.data())?;
        Ok(Some(record))
    }

    fn lookup_cado_staged_committed_then_db(
        &self,
        storage: &impl CADOStorage,
        path: &CadoPath,
    ) -> Option<CadoBody> {
        if let Some(cado) = self.cado_cache.get(path.as_str()) {
            return Some(cado.clone());
        }

        if let Some(cado) = self.committed_cado_cache.get(path.as_str().as_bytes()) {
            return Some(cado.clone());
        }

        match storage.get_cado_by_path(path.clone()) {
            Ok(Some(cado)) => Some(cado),
            Ok(None) => None,
            Err(e) => {
                handle_recoverable_eld_error(e);
                None
            }
        }
    }

    pub fn get_cado_instance<I: for<'de> serde::Deserialize<'de>>(
        &mut self,
        storage: &impl CADOStorage,
        path: &CadoPath,
    ) -> Result<Option<InstanceWithCadoHash<I>>, EldError> {
        let cado = self.lookup_cado_staged_committed_then_db(storage, path);

        match cado {
            Some(CadoBody::Mutable(cado_mut)) => {
                match safe_deserialize_cado_data::<I>(
                    cado_mut.data(),
                    &format!("mutable CADO at {}", path.as_str()),
                ) {
                    Ok(instance) => Ok(Some(InstanceWithCadoHash {
                        instance,
                        hash: cado_mut.hash_bytes(),
                    })),
                    Err(e) => {
                        handle_recoverable_eld_error(e);
                        Ok(None)
                    }
                }
            }
            Some(CadoBody::Immutable(cado_imm)) => {
                match safe_deserialize_cado_data::<I>(
                    cado_imm.data(),
                    &format!("immutable CADO at {}", path.as_str()),
                ) {
                    Ok(instance) => Ok(Some(InstanceWithCadoHash {
                        instance,
                        hash: cado_imm.hash_bytes(),
                    })),
                    Err(e) => {
                        handle_recoverable_eld_error(e);
                        Ok(None)
                    }
                }
            }
            None => Ok(None),
        }
    }

    pub fn get_staking_account_from_cado(
        &mut self,
        storage: &impl CADOStorage,
        path: &CadoPath,
    ) -> Option<StakingAccountWithCadoHash> {
        if let Some(CadoBody::Mutable(cado_mut)) =
            self.lookup_cado_staged_committed_then_db(storage, path)
        {
            if let Ok(account) = safe_deserialize_account_data::<StakingAccount>(
                cado_mut.data(),
                &format!("staking account at {}", path.as_str()),
            ) {
                return Some(StakingAccountWithCadoHash {
                    account,
                    hash: cado_mut.hash_bytes(),
                });
            }
        }

        None
    }

    pub fn get_account_from_cado(
        &mut self,
        storage: &impl CADOStorage,
        path: &CadoPath,
    ) -> Option<AccountWithCadoHash> {
        info!("Attempting to get account from path: {}", path.as_str());

        if let Some(cado) = self.lookup_cado_staged_committed_then_db(storage, path) {
            info!(
                "Found CADO via staged/committed/db lookup for {}: {:?}",
                path.as_str(),
                cado
            );
            if let CadoBody::Mutable(cado_mut) = cado {
                // Try to deserialize the account from the CADO data
                match safe_deserialize_account_data::<Account>(
                    cado_mut.data(),
                    &format!("account at {}", path.as_str()),
                ) {
                    Ok(account) => {
                        info!(
                            "Successfully deserialized account for {}: {:?}",
                            path.as_str(),
                            account
                        );
                        return Some(AccountWithCadoHash {
                            account,
                            hash: cado_mut.hash_bytes(),
                        });
                    }
                    Err(e) => {
                        error!("Failed to deserialize account for {}: {}", path.as_str(), e);
                    }
                }
            }
        } else {
            info!("No CADO found for {}", path.as_str());
        }

        error!(
            "Failed to get account through all methods for {}",
            path.as_str()
        );
        None
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abci_interface::snapshot::codec::deserialize_abci_snapshot;
    use crate::abci_interface::snapshot::SnapshotManager;
    use crate::storage::hybrid_storage::HybridStorage;
    use crate::storage::rocksdb::RocksDBStorage;
    use crate::storage::traits::CADOStorage;
    use eld_common::address::Address;
    use eld_common::cado::{CADOMap, CADOMetadata};
    use eld_common::constants::cado::{LATEST, PATH_PREFIX_ACCOUNT};
    use eld_common::nonce::Nonce;
    use rocksdb::Transaction;

    // Simple mock storage for testing
    struct MockStorage;

    impl CADOStorage for MockStorage {
        fn put_cado_type(&self, _path: CadoPath, _cado_type: CadoBody) -> Result<(), EldError> {
            Ok(())
        }

        fn put_cado_data(
            &self,
            _path: CadoPath,
            _cado_data: Vec<u8>,
            _metadata: CADOMetadata,
        ) -> Result<(), EldError> {
            Ok(())
        }

        fn put_cado_map(&self, _path: CadoPath, _mapping: CADOMap) -> Result<(), EldError> {
            Ok(())
        }

        fn get_cado_by_path(&self, _path: CadoPath) -> Result<Option<CadoBody>, EldError> {
            Ok(None)
        }

        fn get_cado_paths_by_prefix(&self, _prefix: &str) -> Result<Vec<CadoPath>, EldError> {
            Ok(vec![])
        }

        fn get_cados_by_prefix(&self, _prefix: &str) -> Result<Vec<CadoBody>, EldError> {
            Ok(vec![])
        }

        fn list_epoch_records_chron(
            &self,
            _order: crate::storage::traits::EpochRecordListOrder,
            _after_epoch: Option<i64>,
            _fetch_limit: usize,
        ) -> Result<Vec<eld_common::validator::EpochRecord>, EldError> {
            Ok(vec![])
        }

        fn count_epoch_records(&self) -> Result<u64, EldError> {
            Ok(0)
        }

        fn search_cado_hash(&self, _prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError> {
            Ok(vec![])
        }

        fn search_cado_name(&self, _prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError> {
            Ok(vec![])
        }

        fn search_cado_path(&self, _prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError> {
            Ok(vec![])
        }

        fn get_cado_map(&self, _path: CadoPath) -> Result<Option<CADOMap>, EldError> {
            Ok(None)
        }

        fn delete_cado(
            &self,
            _path: CadoPath,
            _owner: &str,
            _signature: &str,
            _public_key: &str,
            _chain_id: &str,
        ) -> Result<(), EldError> {
            Ok(())
        }

        fn system_delete_cado(&self, _path: CadoPath, _owner: &str) -> Result<(), EldError> {
            Ok(())
        }

        fn system_delete_cado_with_tx(
            &self,
            _path: CadoPath,
            _owner: &str,
            _tx: &Transaction<'_, rocksdb::TransactionDB>,
        ) -> Result<(), EldError> {
            Ok(())
        }

        fn begin_transaction(&self) -> Transaction<'_, rocksdb::TransactionDB> {
            unimplemented!("Mock storage doesn't support transactions")
        }

        fn put_cado_type_with_tx(
            &self,
            _path: CadoPath,
            _cado_type: CadoBody,
            _tx: &Transaction<'_, rocksdb::TransactionDB>,
        ) -> Result<(), EldError> {
            Ok(())
        }

        fn put_cado_data_with_tx(
            &self,
            _path: CadoPath,
            _cado_data: Vec<u8>,
            _metadata: CADOMetadata,
            _tx: &Transaction<'_, rocksdb::TransactionDB>,
        ) -> Result<(), EldError> {
            Ok(())
        }

        fn delete_cado_with_tx(
            &self,
            _path: CadoPath,
            _owner: &str,
            _signature: &str,
            _public_key: &str,
            _chain_id: &str,
            _tx: &Transaction<'_, rocksdb::TransactionDB>,
        ) -> Result<(), EldError> {
            Ok(())
        }

        fn put_cado_map_with_tx(
            &self,
            _path: CadoPath,
            _mapping: CADOMap,
            _tx: &Transaction<'_, rocksdb::TransactionDB>,
        ) -> Result<(), EldError> {
            Ok(())
        }
    }

    #[test]
    fn has_cado_in_working_set_checks_staged_and_committed_cache() {
        use eld_common::address::Address;
        use eld_common::cado::{CADOMetadata, CadoBody, CadoPath, CadoPathKey, CadoType};

        let address =
            Address::parse_hex_str("0x1234567890123456789012345678901234567890").expect("address");
        let path =
            CadoPath::new(CadoType::Account, CadoPathKey::Address(address)).expect("account path");
        let cado = CadoBody::mutable_new(
            vec![1],
            CADOMetadata::new(CadoType::Account, address.hex_with_prefix()),
        );

        let mut envelope = AppStateEnvelope::default();
        assert!(!envelope.has_cado_in_working_set(&path));

        envelope.update_cado_cache(path.clone(), cado.clone());
        assert!(envelope.has_cado_in_working_set(&path));

        envelope.cado_cache.clear();
        envelope
            .committed_cado_cache
            .insert(path.as_str().as_bytes(), cado);
        assert!(envelope.has_cado_in_working_set(&path));
    }

    #[test]
    fn update_cado_cache_refuses_infrastructure_paths() {
        use crate::app_state::app_state_snapshot::AppStateSnapshot;
        use eld_common::cado::{CADOMetadata, CadoBody, CadoType};

        let mut envelope = AppStateEnvelope::default();
        let path = AppStateSnapshot::latest_path().expect("path");
        let cado = CadoBody::immutable(
            vec![1],
            CADOMetadata::new(CadoType::AppStateSnapshot, "system"),
        );

        envelope.update_cado_cache(path.clone(), cado);

        assert!(
            !envelope.cado_cache.contains_key(path.as_str()),
            "infrastructure CADO must not be staged in cado_cache"
        );
    }

    #[test]
    fn has_persisted_app_state_tip_detects_latest_app_state_tip_in_path_index() {
        let temp_dir = tempfile::TempDir::new().expect("tempdir");
        let storage = RocksDBStorage::new(temp_dir.path()).expect("rocksdb");

        assert!(
            !AppState::has_persisted_app_state_tip(&storage).expect("check empty db"),
            "empty db should have no persisted AppStateTip"
        );

        let app_state_tip = AppStateTip {
            block_height: 42,
            cado_root_hash: [7u8; 32],
            app_hash: vec![1, 2, 3],
        };
        let serialized = bincode::serialize(&app_state_tip).expect("serialize");
        let metadata = CADOMetadata::new(CadoType::AppStateTip, "system");
        let cado = CadoBody::immutable(serialized, metadata);
        let latest_path =
            CadoPath::new(CadoType::AppStateTip, CadoPathKey::Name(LATEST)).expect("path");

        storage
            .put_cado_type(latest_path.clone(), cado)
            .expect("persist AppStateTip");

        assert!(
            storage
                .get_cado_map(latest_path)
                .expect("map lookup")
                .is_none(),
            "AppStateTip commit path does not use cado_map"
        );
        assert!(
            AppState::has_persisted_app_state_tip(&storage).expect("check persisted db"),
            "AppStateTip at LATEST path_index should be detected"
        );
    }

    #[test]
    fn initialize_with_data_restores_committed_cado_objects_from_app_state_snapshot() {
        let temp_dir = tempfile::TempDir::new().expect("tempdir");
        let rocksdb = std::sync::Arc::new(RocksDBStorage::new(temp_dir.path()).expect("rocksdb"));
        let storage = HybridStorage::new(rocksdb.clone());

        let mut original = AppState::default();
        original.envelope.init_empty_trie();
        original.envelope.block_height = 55;
        original.app_hash = vec![0xAB; 32];

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
        original
            .envelope
            .committed_cado_cache
            .insert(account_path.as_str().as_bytes(), account_cado.clone());
        original
            .envelope
            .state_trie
            .insert(account_path.as_str().as_bytes(), &account_hash);
        original.envelope.committed_cado_cache.calculate_hash();

        let app_state_tip = AppStateTip {
            block_height: original.envelope.block_height,
            cado_root_hash: original.envelope.state_trie.root_hash(),
            app_hash: original.app_hash.clone(),
        };
        let latest_app_state_tip_path =
            CadoPath::new(CadoType::AppStateTip, CadoPathKey::Name(LATEST)).expect("sv path");
        let app_state_tip_cado = CadoBody::immutable(
            bincode::serialize(&app_state_tip).expect("serialize app state tip"),
            CADOMetadata::new(CadoType::AppStateTip, "system"),
        );
        storage
            .put_cado_type(latest_app_state_tip_path, app_state_tip_cado)
            .expect("persist app state tip");

        let app_state_snapshot = AppStateSnapshot::new(&original);
        let app_state_snapshot_path =
            AppStateSnapshot::latest_path().expect("app state snapshot path");
        storage
            .put_cado_type(
                app_state_snapshot_path,
                app_state_snapshot
                    .to_cado()
                    .expect("app state snapshot to cado"),
            )
            .expect("persist app state snapshot");

        let mut restored = AppState::default();
        restored
            .initialize_with_data(&storage)
            .expect("initialize with persisted data");

        assert_eq!(restored, original);
        let restored_account = restored
            .envelope
            .committed_cado_cache
            .get(account_path.as_str().as_bytes())
            .expect("restored committed CADO missing");
        assert_eq!(restored_account.content_hash(), account_hash);
    }

    #[test]
    fn initialize_with_data_strips_infrastructure_from_app_state_snapshot() {
        use crate::app_state::app_state_snapshot::AppStateSnapshot;

        let temp_dir = tempfile::TempDir::new().expect("tempdir");
        let rocksdb = std::sync::Arc::new(RocksDBStorage::new(temp_dir.path()).expect("rocksdb"));
        let storage = HybridStorage::new(rocksdb.clone());

        let mut original = AppState::default();
        original.envelope.init_empty_trie();
        original.envelope.block_height = 60;
        original.app_hash = vec![0xAB; 32];

        let account_path = CadoPath::parse(&format!(
            "{}{}",
            PATH_PREFIX_ACCOUNT, "0x1234567890123456789012345678901234567890"
        ))
        .expect("account path");
        let account_cado = CadoBody::mutable_new(
            vec![1, 2, 3],
            CADOMetadata::new(
                CadoType::Account,
                "0x1234567890123456789012345678901234567890",
            ),
        );
        let account_hash = account_cado.content_hash();
        original
            .envelope
            .committed_cado_cache
            .insert(account_path.as_str().as_bytes(), account_cado);
        original
            .envelope
            .state_trie
            .insert(account_path.as_str().as_bytes(), &account_hash);

        let app_state_tip = AppStateTip {
            block_height: original.envelope.block_height,
            cado_root_hash: original.envelope.state_trie.root_hash(),
            app_hash: original.app_hash.clone(),
        };
        let latest_app_state_tip_path =
            CadoPath::new(CadoType::AppStateTip, CadoPathKey::Name(LATEST)).expect("sv path");
        storage
            .put_cado_type(
                latest_app_state_tip_path,
                CadoBody::immutable(
                    bincode::serialize(&app_state_tip).expect("serialize app state tip"),
                    CADOMetadata::new(CadoType::AppStateTip, "system"),
                ),
            )
            .expect("persist app state tip");

        let mut app_state_snapshot = AppStateSnapshot::new(&original);
        let infra_path = AppStateSnapshot::latest_path().expect("infra path");
        app_state_snapshot.committed_cado_cache.insert(
            infra_path.as_str().as_bytes(),
            CadoBody::immutable(
                vec![9, 9, 9],
                CADOMetadata::new(CadoType::AppStateSnapshot, "system"),
            ),
        );
        storage
            .put_cado_type(
                AppStateSnapshot::latest_path().expect("app state path"),
                app_state_snapshot
                    .to_cado()
                    .expect("app state snapshot cado"),
            )
            .expect("persist polluted app state snapshot");

        let mut restored = AppState::default();
        restored
            .initialize_with_data(&storage)
            .expect("initialize with polluted trie snapshot");

        assert_eq!(restored.envelope.committed_cado_cache.len(), 1);
        assert!(restored
            .envelope
            .committed_cado_cache
            .get(account_path.as_str().as_bytes())
            .is_some());
        assert!(restored
            .envelope
            .committed_cado_cache
            .get(infra_path.as_str().as_bytes())
            .is_none());
    }

    #[tokio::test]
    async fn app_state_snapshot_roundtrip_via_abci_payload_restores_original_state() {
        let src_dir = tempfile::TempDir::new().expect("src tempdir");
        let src_rocksdb =
            std::sync::Arc::new(RocksDBStorage::new(src_dir.path()).expect("rocksdb"));
        let src_storage = HybridStorage::new(src_rocksdb.clone());

        let mut original = AppState::default();
        original.envelope.init_empty_trie();
        original.envelope.block_height = 77;
        original.app_hash = vec![0xCD; 32];

        let account_path = CadoPath::parse(&format!(
            "{}{}",
            PATH_PREFIX_ACCOUNT, "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ))
        .expect("account path");
        let account_cado = CadoBody::mutable_new(
            vec![9, 8, 7, 6, 5],
            CADOMetadata::new(
                CadoType::Account,
                "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
        );
        let account_hash = account_cado.content_hash();
        original
            .envelope
            .committed_cado_cache
            .insert(account_path.as_str().as_bytes(), account_cado);
        original
            .envelope
            .state_trie
            .insert(account_path.as_str().as_bytes(), &account_hash);
        original.envelope.committed_cado_cache.calculate_hash();

        let app_state_tip = AppStateTip {
            block_height: original.envelope.block_height,
            cado_root_hash: original.envelope.state_trie.root_hash(),
            app_hash: original.app_hash.clone(),
        };
        let latest_app_state_tip_path =
            CadoPath::new(CadoType::AppStateTip, CadoPathKey::Name(LATEST)).expect("sv path");
        let app_state_tip_cado = CadoBody::immutable(
            bincode::serialize(&app_state_tip).expect("serialize app state tip"),
            CADOMetadata::new(CadoType::AppStateTip, "system"),
        );
        src_storage
            .put_cado_type(latest_app_state_tip_path, app_state_tip_cado)
            .expect("persist app state tip");

        let app_state_snapshot = AppStateSnapshot::new(&original);
        let app_state_snapshot_path =
            AppStateSnapshot::latest_path().expect("app state snapshot path");
        src_storage
            .put_cado_type(
                app_state_snapshot_path,
                app_state_snapshot
                    .to_cado()
                    .expect("app state snapshot to cado"),
            )
            .expect("persist app state snapshot");

        let manager = SnapshotManager::new(src_rocksdb.clone());
        manager
            .create_snapshot_from_latest_state(original.envelope.block_height)
            .await
            .expect("create snapshot from canonical state");
        let payload = manager
            .get_snapshot(original.envelope.block_height)
            .await
            .expect("read created snapshot")
            .expect("snapshot payload missing");

        let decoded = deserialize_abci_snapshot(&payload).expect("decode abci snapshot payload");

        let restore_dir = tempfile::TempDir::new().expect("restore tempdir");
        let restore_rocksdb =
            std::sync::Arc::new(RocksDBStorage::new(restore_dir.path()).expect("restore rocksdb"));
        let restore_storage = HybridStorage::new(restore_rocksdb.clone());

        let restore_app_state_tip_path =
            CadoPath::new(CadoType::AppStateTip, CadoPathKey::Name(LATEST)).expect("tip path");
        let restored_app_state_tip = AppStateTip {
            block_height: decoded.app_state_snapshot.block_height,
            cado_root_hash: decoded.app_state_snapshot.state_trie_root,
            app_hash: decoded.app_state_snapshot.app_hash.clone(),
        };
        let restore_app_state_tip_cado = CadoBody::immutable(
            bincode::serialize(&restored_app_state_tip).expect("serialize restored app state tip"),
            CADOMetadata::new(CadoType::AppStateTip, "system"),
        );
        restore_storage
            .put_cado_type(restore_app_state_tip_path, restore_app_state_tip_cado)
            .expect("persist restored app state tip");

        restore_storage
            .put_cado_type(
                AppStateSnapshot::latest_path().expect("app state path"),
                decoded
                    .app_state_snapshot
                    .to_cado()
                    .expect("restored app state snapshot to cado"),
            )
            .expect("persist restored app state snapshot");

        let mut restored = AppState::default();
        restored
            .initialize_with_data(&restore_storage)
            .expect("initialize restored app state");

        assert_eq!(restored, original);
    }

    #[test]
    fn test_get_cado_instance_invalid_data() {
        let mut state = AppState::default();
        let storage = MockStorage;
        let path = CadoPath::parse(&format!(
            "{}{}",
            PATH_PREFIX_ACCOUNT, "0x1234567890123456789012345678901234567890"
        ))
        .unwrap();
        state.envelope.cado_cache.insert(
            path.as_str().to_string(),
            CadoBody::mutable_new(
                vec![0xFF, 0xFF], // Invalid data
                CADOMetadata::new(
                    CadoType::Account,
                    "0x1234567890123456789012345678901234567890",
                ),
            ),
        );
        let result = state.envelope.get_cado_instance::<i32>(&storage, &path);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn get_account_from_cado_prefers_committed_cache_before_db() {
        let temp_dir = tempfile::TempDir::new().expect("tempdir");
        let storage = RocksDBStorage::new(temp_dir.path()).expect("rocksdb");
        let mut envelope = AppStateEnvelope::default();
        let address =
            Address::parse_hex_str("0x1234567890123456789012345678901234567890").expect("address");
        let path =
            CadoPath::new(CadoType::Account, CadoPathKey::Address(address)).expect("account path");

        let db_account = Account::new(
            address,
            Coin::new(1).expect("coin"),
            Nonce::new(Nonce::ZERO),
        );
        let db_cado = CadoBody::mutable_new(
            bincode::serialize(&db_account).expect("serialize db account"),
            CADOMetadata::new(CadoType::Account, address.hex_with_prefix()),
        );
        storage
            .put_cado_type(path.clone(), db_cado)
            .expect("put db account");

        let committed_account = Account::new(
            address,
            Coin::new(2).expect("coin"),
            Nonce::new(Nonce::ZERO),
        );
        let committed_cado = CadoBody::mutable_new(
            bincode::serialize(&committed_account).expect("serialize committed account"),
            CADOMetadata::new(CadoType::Account, address.hex_with_prefix()),
        );
        envelope
            .committed_cado_cache
            .insert(path.as_str().as_bytes(), committed_cado);

        let got = envelope
            .get_account_from_cado(&storage, &path)
            .expect("account from lookup");
        assert_eq!(got.account.balance(), committed_account.balance());
    }

    #[test]
    fn get_cado_instance_falls_back_to_db_when_missing_in_caches() {
        let temp_dir = tempfile::TempDir::new().expect("tempdir");
        let storage = RocksDBStorage::new(temp_dir.path()).expect("rocksdb");
        let mut envelope = AppStateEnvelope::default();
        let address =
            Address::parse_hex_str("0x1234567890123456789012345678901234567890").expect("address");
        let path =
            CadoPath::new(CadoType::Account, CadoPathKey::Address(address)).expect("account path");

        let db_account = Account::new(
            address,
            Coin::new(3).expect("coin"),
            Nonce::new(Nonce::ZERO),
        );
        let db_cado = CadoBody::mutable_new(
            bincode::serialize(&db_account).expect("serialize db account"),
            CADOMetadata::new(CadoType::Account, address.hex_with_prefix()),
        );
        storage
            .put_cado_type(path.clone(), db_cado)
            .expect("put db account");

        let got = envelope
            .get_cado_instance::<Account>(&storage, &path)
            .expect("lookup result")
            .expect("account exists");
        assert_eq!(got.instance.balance(), db_account.balance());
    }

    #[test]
    fn validate_add_namespace_layer_a_same_block() {
        let mut envelope = AppStateEnvelope::default();
        let owner =
            Address::parse_hex_str("0xe17404c417fa10cc04fdf73604fcacca8d0a687c").expect("address");
        envelope.namespace_registry_cache.insert(
            "peter".to_string(),
            NamespaceRecord {
                namespace_slug: "peter".to_string(),
                owner,
                registered_height: 1,
            },
        );
        assert!(envelope.validate_add_namespace_not_taken("peter").is_err());
    }

    #[test]
    fn validate_add_namespace_layer_b_committed_cache() {
        let mut envelope = AppStateEnvelope::default();
        let path = slug_to_namespace_cadopath("peter").expect("path");
        envelope.committed_cado_cache.insert(
            path.as_str().as_bytes(),
            CadoBody::mutable_new(
                vec![1, 2, 3],
                CADOMetadata::new(CadoType::Namespace, "peter"),
            ),
        );
        assert!(envelope.validate_add_namespace_not_taken("peter").is_err());
    }

    #[test]
    fn resolve_namespace_prefers_block_cache() {
        let mut envelope = AppStateEnvelope::default();
        let owner =
            Address::parse_hex_str("0xe17404c417fa10cc04fdf73604fcacca8d0a687c").expect("address");
        let staged = NamespaceRecord {
            namespace_slug: "peter".to_string(),
            owner,
            registered_height: 99,
        };
        envelope
            .namespace_registry_cache
            .insert("peter".to_string(), staged.clone());
        let resolved = envelope.resolve_namespace("peter").expect("resolve");
        assert_eq!(resolved, Some(staged));
    }

    #[test]
    fn insert_epoch_records_index_from_path_inserts_valid_epoch_paths_only() {
        use eld_common::cado::epoch_record_path_name;
        use eld_common::constants::cado::LATEST;

        let mut envelope = AppStateEnvelope::default();

        let epoch_key = epoch_record_path_name(4).expect("name");
        let epoch_path =
            CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(&epoch_key)).expect("path");
        envelope.insert_epoch_records_index_from_path(epoch_path.as_str());
        assert_eq!(
            envelope
                .epoch_records_index
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![4]
        );

        let latest_path =
            CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(LATEST)).expect("latest");
        envelope.insert_epoch_records_index_from_path(latest_path.as_str());
        assert_eq!(
            envelope
                .epoch_records_index
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![4]
        );

        let account_path = CadoPath::parse(&format!(
            "{}{}",
            PATH_PREFIX_ACCOUNT, "0x1234567890123456789012345678901234567890"
        ))
        .expect("account path");
        envelope.insert_epoch_records_index_from_path(account_path.as_str());
        assert_eq!(
            envelope
                .epoch_records_index
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![4]
        );
    }

    #[test]
    fn rebuild_epoch_records_index_populates_from_committed_cache() {
        let mut envelope = AppStateEnvelope::default();
        insert_test_epoch_cado(&mut envelope, 1);
        insert_test_epoch_cado(&mut envelope, 3);
        insert_test_epoch_cado(&mut envelope, 7);
        envelope.epoch_records_index.insert(99);

        envelope.rebuild_epoch_records_index();

        assert_eq!(
            envelope
                .epoch_records_index
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![1, 3, 7]
        );
    }

    #[test]
    fn collect_epoch_numbers_scans_cache_when_index_empty() {
        let mut committed = AppStateEnvelope::default();
        insert_test_epoch_cado(&mut committed, 0);
        insert_test_epoch_cado(&mut committed, 2);

        let epochs = committed.collect_epoch_numbers(None);
        assert_eq!(epochs.into_iter().collect::<Vec<_>>(), vec![0, 2]);
    }

    #[test]
    fn collect_epoch_numbers_uses_index_when_populated() {
        let mut committed = AppStateEnvelope::default();
        insert_test_epoch_cado(&mut committed, 5);
        committed.epoch_records_index.insert(0);
        committed.epoch_records_index.insert(2);

        let epochs = committed.collect_epoch_numbers(None);
        assert_eq!(epochs.into_iter().collect::<Vec<_>>(), vec![0, 2]);
    }

    #[test]
    fn epoch_records_index_collects_committed_and_staged_epochs() {
        use eld_common::cado::epoch_record_path_name;
        use eld_common::constants::cado::LATEST;

        let mut committed = AppStateEnvelope::default();
        for epoch in [0i64, 2] {
            insert_test_epoch_cado(&mut committed, epoch);
            let key = epoch_record_path_name(epoch).expect("name");
            let path = CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(&key)).expect("path");
            committed.insert_epoch_records_index_from_path(path.as_str());
        }

        let mut current = AppStateEnvelope::default();
        let key = epoch_record_path_name(5).expect("name");
        let path = CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(&key)).expect("path");
        current.cado_cache.insert(
            path.as_str().to_string(),
            CadoBody::immutable(vec![], CADOMetadata::new(CadoType::EpochRecord, &key)),
        );
        let latest_path =
            CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(LATEST)).expect("latest");
        current.cado_cache.insert(
            latest_path.as_str().to_string(),
            CadoBody::immutable(vec![], CADOMetadata::new(CadoType::EpochRecord, LATEST)),
        );

        let epochs = committed.collect_epoch_numbers(Some(&current));
        assert_eq!(epochs.into_iter().collect::<Vec<_>>(), vec![0, 2, 5]);
    }

    fn insert_test_epoch_cado(envelope: &mut AppStateEnvelope, epoch: i64) {
        use eld_common::cado::epoch_record_path_name;

        let key = epoch_record_path_name(epoch).expect("name");
        let path = CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(&key)).expect("path");
        envelope.committed_cado_cache.insert(
            path.as_str().as_bytes(),
            CadoBody::immutable(vec![], CADOMetadata::new(CadoType::EpochRecord, &key)),
        );
    }

    fn insert_test_namespace_cado(envelope: &mut AppStateEnvelope, record: &NamespaceRecord) {
        let path = slug_to_namespace_cadopath(&record.namespace_slug).expect("namespace path");
        let bytes = record.serialize_bin().expect("serialize");
        envelope.committed_cado_cache.insert(
            path.as_str().as_bytes(),
            CadoBody::immutable(
                bytes,
                CADOMetadata::new(CadoType::Namespace, &record.namespace_slug),
            ),
        );
    }

    #[test]
    fn rebuild_namespace_registry_index_populates_from_committed_cache() {
        let owner =
            Address::parse_hex_str("0xe17404c417fa10cc04fdf73604fcacca8d0a687c").expect("address");
        let alpha = NamespaceRecord {
            namespace_slug: "alpha".to_string(),
            owner,
            registered_height: 10,
        };
        let beta = NamespaceRecord {
            namespace_slug: "beta".to_string(),
            owner,
            registered_height: 20,
        };

        let mut envelope = AppStateEnvelope::default();
        insert_test_namespace_cado(&mut envelope, &alpha);
        insert_test_namespace_cado(&mut envelope, &beta);
        envelope
            .namespace_registry_index
            .insert("stale".to_string(), alpha.clone());

        envelope
            .rebuild_namespace_registry_index()
            .expect("rebuild namespace index");

        assert_eq!(envelope.namespace_registry_index.len(), 2);
        assert_eq!(
            envelope
                .namespace_registry_index
                .get("alpha")
                .map(|record| record.registered_height),
            Some(10)
        );
        assert_eq!(
            envelope
                .namespace_registry_index
                .get("beta")
                .map(|record| record.registered_height),
            Some(20)
        );
    }

    #[test]
    fn collect_namespace_records_scans_cache_when_index_empty() {
        let owner =
            Address::parse_hex_str("0xe17404c417fa10cc04fdf73604fcacca8d0a687c").expect("address");
        let record = NamespaceRecord {
            namespace_slug: "peter".to_string(),
            owner,
            registered_height: 42,
        };

        let mut envelope = AppStateEnvelope::default();
        insert_test_namespace_cado(&mut envelope, &record);

        let records = envelope.collect_namespace_records(None).expect("collect");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].namespace_slug, "peter");
    }
}
