use crate::app_state::cado_hash::{
    AccountWithCadoHash, InstanceWithCadoHash, StakingAccountWithCadoHash,
};
use crate::app_state::committed_cado_cache::CommittedCadoCache;
use crate::app_state::state_trie::StateTrie;
use crate::errors::handle_recoverable_eld_error;
use crate::storage::traits::{CADOStorage, VerifiedProofRewardDedupStorage};
use eld_common::account::Account;
use eld_common::address::Address;
use eld_common::cado::{
    epoch_from_record_path_name, epoch_record_path_name, CADOMarkedForDeletion, CadoBody, CadoPath,
    CadoPathKey, CadoType,
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
use eld_common::validation::{safe_deserialize_account_data, safe_deserialize_cado_data};
use eld_common::validator::{
    ActiveCapacityValidator, CapacityValidatorInfo, EpochRecord, ValidatorInfo,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use tracing::{error, info, warn};

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
    /// VerifiedProof challenge IDs counted as failures during the current block (flushed at commit).
    #[serde(default)]
    pub failed_proof_counted_cache: HashSet<String>,
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

    /// Failed `VerifiedProof` challenges per capacity provider. Hashed. Reset to zero on slash.
    #[serde(default)]
    pub failed_proof_counts: BTreeMap<Address, u32>,
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
        self.failed_proof_counts.hash(state);
    }
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

    /// True when this `challenge_id` was already counted as a failed proof in-block or in committed storage.
    pub fn is_verified_proof_challenge_failed<S: VerifiedProofRewardDedupStorage>(
        &self,
        storage: &S,
        challenge_id: &str,
    ) -> Result<bool, EldError> {
        if self.failed_proof_counted_cache.contains(challenge_id) {
            return Ok(true);
        }
        storage.is_verified_proof_challenge_failed(challenge_id)
    }

    /// SHA-256 over the same fields as [`Hash`] for this envelope.
    pub fn calculate_app_hash(&self) -> [u8; 32] {
        let mut collector = HashBytes::default();
        self.hash(&mut collector);
        Sha256::digest(&collector.0).into()
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

    /// Epoch-start snapshot: staged cache, then committed cache, then storage.
    ///
    /// Bytes are [`EpochRecord::deserialize_bin`] (bincode), not serde.
    pub fn get_epoch_record(
        &self,
        storage: &impl CADOStorage,
        epoch: i64,
    ) -> Result<Option<EpochRecord>, EldError> {
        let epoch_key = epoch_record_path_name(epoch)?;
        let path = CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(&epoch_key))?;
        let Some(cado) = self.lookup_cado_staged_committed_then_db(storage, &path) else {
            return Ok(None);
        };
        Ok(Some(EpochRecord::deserialize_bin(cado.data())?))
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

/// Collects the byte stream produced by [`std::hash::Hash`] so it can be SHA-256'd.
#[derive(Default)]
struct HashBytes(Vec<u8>);

impl Hasher for HashBytes {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }

    fn finish(&self) -> u64 {
        0
    }
}
