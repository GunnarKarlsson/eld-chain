use super::tx_deliver::ConsensusTxDeliver;
use crate::abci_interface::chain_tip::ChainTip;
use crate::abci_interface::snapshot::SnapshotManager;
use crate::app_state::app_state_snapshot::AppStateSnapshot;
use crate::app_state::{AppState, AppStateTip};
use crate::capacity::capacity_manager::CapacityManager;
use crate::config::ConsensusConfig;
use crate::content::sync::{P2pCoordinatorTrait, SyncMsg};
use crate::errors::handle_fatal_eld_error;
use crate::errors::handle_recoverable_eld_error;
use crate::errors::{
    response_deliver_tx_error_fee_calculation_failed,
    response_deliver_tx_error_fee_validation_failed,
    response_deliver_tx_error_hex_validation_failed, response_deliver_tx_error_insufficient_fee,
    response_deliver_tx_error_insufficient_funds, response_deliver_tx_error_internal_lock_failed,
    response_deliver_tx_error_invalid_coin_amount, response_deliver_tx_error_invalid_hex_encoding,
    response_deliver_tx_error_invalid_path_format, response_deliver_tx_error_invalid_utf8_encoding,
    response_deliver_tx_error_json_parsing_failed,
    response_deliver_tx_error_json_validation_failed,
    response_deliver_tx_error_nonce_not_sequential, response_deliver_tx_error_nonce_overflow,
    response_deliver_tx_error_sender_doesnt_exist,
    response_deliver_tx_error_transaction_structure_invalid,
    response_deliver_tx_error_tx_too_large, response_deliver_tx_error_verification_failed,
};
use crate::node_identity::{ensure_identity_from_status, LocalNodeIdentity};
use crate::storage::traits::ConsensusConnectionStorage;
use abci::{async_api::Consensus, async_trait, types::ResponseDeliverTx, types::*};
use ed25519_dalek::VerifyingKey;
use eld_common::account::Account;
use eld_common::address::Address;
use eld_common::cado::{
    epoch_record_path_name, CADOMetadata, CadoBody, CadoPath, CadoPathKey, CadoType,
};
use eld_common::capacity_challenge::{compute_challenge_id, select_challenge_chunk_indices};
use eld_common::coin::Coin;
use eld_common::constants::{
    cado::LATEST,
    p2p::{ELD_STORAGE_CHALLENGE_TOPIC_PREFIX, ELD_STORAGE_PROOF_TOPIC_PREFIX},
    pinboard::MAX_CHUNK_SIZE,
    protocol::{BLOCKS_PER_EPOCH, BLOCK_REWARD, CHALLENGES_PER_EPOCH, VALIDATORS_PER_EPOCH},
};
use eld_common::error::{EldError, ErrorBuilder};
use eld_common::nonce::Nonce;
use eld_common::storage::AccountStorage;
use eld_common::tx::HasSender;
use eld_common::tx::Tx;
use eld_common::validation::{
    self, validate_hex_string, validate_json_string, validate_transaction_structure,
};
use eld_common::validator::{ActiveCapacityValidator, EpochRecord, ValidatorInfo};
use hex;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use serde_json;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use tokio::sync::oneshot;
use tracing::{debug, error, info, warn};

#[derive(Debug)]
pub struct ValidatorRewardManager<T>
where
    T: AccountStorage,
{
    _storage: Arc<T>,
}

impl<T> ValidatorRewardManager<T>
where
    T: AccountStorage,
{
    fn new(storage: Arc<T>) -> Self {
        Self { _storage: storage }
    }

    fn calculate_validator_rewards(&self, current_state: &mut AppState) {
        // Calculate total reward for this block (pending fees + block reward)
        let block_reward = match Coin::new(BLOCK_REWARD) {
            Ok(coin) => coin,
            Err(e) => {
                handle_recoverable_eld_error(e);
                return;
            }
        };

        let reward_this_block = match current_state.envelope.pending_fee_rewards + block_reward {
            Ok(reward) => reward,
            Err(e) => {
                handle_recoverable_eld_error(e);
                return; // Skip reward calculation if arithmetic fails
            }
        };

        // Get total stake of active validators
        let total_stake = match current_state
            .envelope
            .active_validators
            .iter()
            .try_fold(Coin::zero(), |acc, v| acc + v.stake)
        {
            Ok(coin) => coin,
            Err(e) => {
                error!("Failed to sum validator stakes: {}", e);
                return;
            }
        };

        if total_stake.is_zero() {
            return;
        }

        // Distribute rewards proportionally based on stake
        // Use Coin operations: (reward * validator_stake) / total_stake
        let validator_rewards: Vec<(String, Coin)> = current_state
            .envelope
            .active_validators
            .iter()
            .filter_map(|validator| {
                // Calculate proportional share using Coin operations
                // Formula: (reward * validator_stake) / total_stake
                let product = match reward_this_block * validator.stake {
                    Ok(coin) => coin,
                    Err(e) => {
                        handle_recoverable_eld_error(e);
                        return None;
                    }
                };
                match product / total_stake {
                    Ok(reward) => Some((validator.address.to_string(), reward)),
                    Err(e) => {
                        handle_recoverable_eld_error(e);
                        None // Skip this validator if division fails
                    }
                }
            })
            .collect();

        // Update accounts with rewards
        for (_address, _reward) in validator_rewards {
            // TODO: update cado accounts with rewards
        }

        // Clear pending fees after distribution
        current_state.envelope.pending_fee_rewards = match Coin::new(0) {
            Ok(coin) => coin,
            Err(e) => {
                error!("Can't clear pending fee rewards");
                // TODO: How handle this case?
                handle_recoverable_eld_error(e);
                // Keep existing pending fees if we can't create zero coin
                current_state.envelope.pending_fee_rewards
            }
        };
    }
}

type ChallengedCapacityProviderData = (Address, Option<[u8; 32]>, Option<[u8; 32]>, Option<u32>);

pub struct ConsensusConnection<S>
where
    S: ConsensusConnectionStorage,
{
    pub consensus_config: Arc<Mutex<ConsensusConfig>>,
    pub committed_state: Arc<Mutex<AppState>>,
    pub chain_tip: Arc<ChainTip>,
    pub current_state: Arc<Mutex<Option<AppState>>>,
    pub storage: Arc<S>,
    pub reward_manager: ValidatorRewardManager<S>,
    pub snapshot_manager: Arc<SnapshotManager<S>>,
    pub p2p_sync_coordinator: Arc<dyn P2pCoordinatorTrait>,
    last_commit_time: Arc<Mutex<Instant>>,
    cado_type_counts: Arc<Mutex<HashMap<String, usize>>>,
    ready_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    /// Promoted pinboard blobs are also mirrored into capacity slots after RocksDB write.
    capacity_manager: Arc<CapacityManager>,
    /// Paired Tendermint validator identity (from `/status`); not part of app state.
    local_identity: Arc<RwLock<LocalNodeIdentity>>,
}

/// Construction inputs for [`ConsensusConnection::new`].
pub struct ConsensusConnectionNewContext<S>
where
    S: ConsensusConnectionStorage,
{
    pub consensus_config: Arc<Mutex<ConsensusConfig>>,
    pub committed_state: Arc<Mutex<AppState>>,
    pub chain_tip: Arc<ChainTip>,
    pub current_state: Arc<Mutex<Option<AppState>>>,
    pub storage: Arc<S>,
    pub snapshot_manager: Arc<SnapshotManager<S>>,
    pub p2p_sync_coordinator: Arc<dyn P2pCoordinatorTrait>,
    pub ready_tx: Option<oneshot::Sender<()>>,
    pub capacity_manager: Arc<CapacityManager>,
    pub local_identity: Arc<RwLock<LocalNodeIdentity>>,
}

impl<S> ConsensusConnection<S>
where
    S: ConsensusConnectionStorage,
{
    fn compare_validator_priority(a: &ValidatorInfo, b: &ValidatorInfo) -> Ordering {
        // Deterministic ordering is required for consensus safety when stakes tie.
        b.stake
            .cmp(&a.stake)
            .then_with(|| a.address.cmp(&b.address))
            .then_with(|| a.public_key.cmp(&b.public_key))
    }

    pub fn new(ctx: ConsensusConnectionNewContext<S>) -> Self {
        Self {
            consensus_config: ctx.consensus_config,
            committed_state: ctx.committed_state,
            chain_tip: ctx.chain_tip,
            current_state: ctx.current_state,
            storage: ctx.storage.clone(),
            reward_manager: ValidatorRewardManager::new(ctx.storage),
            snapshot_manager: ctx.snapshot_manager,
            p2p_sync_coordinator: ctx.p2p_sync_coordinator,
            last_commit_time: Arc::new(Mutex::new(Instant::now())),
            cado_type_counts: Arc::new(Mutex::new(HashMap::new())),
            ready_tx: Arc::new(Mutex::new(ctx.ready_tx)),
            capacity_manager: ctx.capacity_manager,
            local_identity: ctx.local_identity,
        }
    }

    fn select_validators_for_epoch(&self, current_state: &mut AppState) {
        // Create a copy of validators list to sort
        let mut validators = current_state.envelope.validators.clone();

        // Sort deterministically so equal-stake validators resolve identically on all nodes.
        validators.sort_by(Self::compare_validator_priority);

        // Take top VALIDATORS_PER_EPOCH validators
        let active_validators: Vec<ValidatorInfo> =
            validators.into_iter().take(VALIDATORS_PER_EPOCH).collect();

        // Update active validators list
        current_state.envelope.active_validators = active_validators;
    }

    /// Select the epoch's active capacity validator from on-chain `capacity_validators`
    /// and choose challenge recipients (self-challenge only when a single eligible exists).
    fn select_active_capacity_validator_for_epoch(&self, current_state: &mut AppState, epoch: i64) {
        let current_block = current_state.envelope.block_height as u64;

        let mut eligible: Vec<_> = current_state
            .envelope
            .capacity_validators
            .iter()
            .filter(|cv| {
                cv.merkle_root.is_some()
                    && current_block <= cv.registered_block + cv.registration_duration
            })
            .cloned()
            .collect();

        if eligible.is_empty() {
            warn!(
                epoch,
                "No eligible capacity validators for epoch; clearing active_capacity_validator"
            );
            current_state.envelope.active_capacity_validator = None;
            return;
        }

        eligible.sort_by(|a, b| a.address.cmp(&b.address));

        let mut seed_bytes = [0u8; 32];
        let mut hasher = Sha256::new();
        hasher.update(b"CAPACITY_VALIDATOR_SELECTION");
        hasher.update(epoch.to_be_bytes());
        hasher.update(current_state.chain_id.as_bytes());
        for cv in &eligible {
            hasher.update(cv.address.hex_with_prefix().as_bytes());
            hasher.update(cv.public_key.as_bytes());
        }
        seed_bytes.copy_from_slice(&hasher.finalize());
        // Deterministic PRNG from a shared seed
        let mut rng = StdRng::from_seed(seed_bytes);

        let mut shuffled = eligible.clone();
        shuffled.shuffle(&mut rng);
        let selected_address = shuffled
            .into_iter()
            .next()
            .expect("eligible is non-empty")
            .address;

        let mut challenge_pool: Vec<Address> = if eligible.len() == 1 {
            eligible.iter().map(|cv| cv.address).collect()
        } else {
            eligible
                .iter()
                .filter(|cv| cv.address != selected_address)
                .map(|cv| cv.address)
                .collect()
        };
        challenge_pool.sort();

        let challenged_providers: Vec<Address> = if challenge_pool.is_empty() {
            Vec::new()
        } else {
            challenge_pool.shuffle(&mut rng);
            let num_to_challenge = CHALLENGES_PER_EPOCH.min(challenge_pool.len());
            challenge_pool.into_iter().take(num_to_challenge).collect()
        };

        let subscribed_topics: Vec<String> = challenged_providers
            .iter()
            .map(|provider_id| format!("{ELD_STORAGE_PROOF_TOPIC_PREFIX}{provider_id}"))
            .collect();

        current_state.envelope.active_capacity_validator = Some(ActiveCapacityValidator {
            validator_address: selected_address,
            epoch,
            challenged_providers,
            subscribed_topics,
        });
    }

    fn build_epoch_record(current_state: &AppState, new_epoch: i64) -> EpochRecord {
        let challenged_capacity_validators = current_state
            .envelope
            .active_capacity_validator
            .as_ref()
            .map(|sv| {
                sv.challenged_providers
                    .iter()
                    .filter_map(|addr| {
                        current_state
                            .envelope
                            .capacity_validators
                            .iter()
                            .find(|p| p.address == *addr)
                            .cloned()
                    })
                    .collect()
            })
            .unwrap_or_default();

        EpochRecord {
            epoch: new_epoch,
            start_block: new_epoch * BLOCKS_PER_EPOCH,
            active_validators: current_state.envelope.active_validators.clone(),
            active_capacity_validator: current_state.envelope.active_capacity_validator.clone(),
            challenged_capacity_validators,
        }
    }

    fn store_epoch_record(
        current_state: &mut AppState,
        record: &EpochRecord,
    ) -> Result<(), EldError> {
        let serialized = record.serialize_bin()?;
        let metadata = CADOMetadata::new(CadoType::EpochRecord, record.epoch.to_string());
        let cado = CadoBody::immutable(serialized, metadata);

        let epoch_key = epoch_record_path_name(record.epoch)?;
        let epoch_path = CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(&epoch_key))?;
        current_state
            .envelope
            .update_cado_cache(epoch_path, cado.clone());

        let latest_path = CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(LATEST))?;
        current_state.envelope.update_cado_cache(latest_path, cado);

        Ok(())
    }

    /// Returns true when this process is the epoch's active capacity validator
    /// (local capacity-validator **wallet** address matches on-chain selected address).
    fn should_this_node_send_challenges(&self, selected_validator: &Address) -> bool {
        let identity_guard = match self.local_identity.read() {
            Ok(guard) => guard,
            Err(e) => {
                warn!(
                    error = %e,
                    "Failed to read local identity; deferring capacity challenges"
                );
                return false;
            }
        };

        if identity_guard.matches_capacity_validator_wallet(selected_validator) {
            return true;
        }

        match identity_guard.capacity_validator_address {
            Some(local) => {
                info!(
                    local_capacity_validator = %local,
                    selected_capacity_validator = %selected_validator,
                    "Not the active capacity validator on this node; skipping capacity challenges"
                );
                false
            }
            None => {
                warn!(
                    "Local capacity validator wallet not configured; deferring capacity challenges"
                );
                false
            }
        }
    }

    fn build_challenged_capacity_provider_data(
        current_state: &AppState,
        challenged_capacity_provider_ids: &[Address],
    ) -> Vec<ChallengedCapacityProviderData> {
        challenged_capacity_provider_ids
            .iter()
            .filter_map(|provider_id| {
                current_state
                    .envelope
                    .capacity_validators
                    .iter()
                    .find(|cp| cp.address == *provider_id)
                    .map(|cp| (*provider_id, cp.merkle_root, cp.seed, cp.chunk_count))
            })
            .collect()
    }

    /// Internal helper to generate challenges (takes cloned data to avoid Send issues)
    fn generate_challenges_internal(
        providers_data: Vec<ChallengedCapacityProviderData>,
        epoch: i64,
        block_height: i64,
        validator_address: Address,
        p2p_coordinator: Arc<dyn P2pCoordinatorTrait>,
    ) -> Result<(), EldError> {
        if providers_data.is_empty() {
            warn!(
                epoch = epoch,
                "No providers to challenge, skipping challenge generation"
            );
            return Ok(());
        }

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let expiration_block = block_height + 100; // Challenges expire after 100 blocks

        // Generate challenges for each provider
        for (provider_id, merkle_root_opt, seed_opt, chunk_count_opt) in providers_data {
            // Get provider's merkle root and seed
            let merkle_root = merkle_root_opt.ok_or_else(|| EldError::ValidationError {
                field: "merkle_root".to_string(),
                value: "none".to_string(),
                details: format!("Provider {provider_id} has no merkle root"),
            })?;

            let seed = seed_opt.ok_or_else(|| EldError::ValidationError {
                field: "seed".to_string(),
                value: "none".to_string(),
                details: format!("Provider {provider_id} has no seed"),
            })?;

            // Generate deterministic chunk indices
            let chunk_count = chunk_count_opt.unwrap_or(0);
            if chunk_count == 0 {
                warn!(
                    provider_id = %provider_id,
                    "Provider has no chunks, skipping challenge"
                );
                continue;
            }

            // Deterministic chunk indices (shared with deliver_tx recomputation).
            let chunk_indices = select_challenge_chunk_indices(
                epoch,
                &provider_id,
                block_height,
                &validator_address,
                chunk_count,
            );

            // challenge_id omits wall-clock timestamp so every node can recompute it.
            // `timestamp` / `expiration_block` below are P2P-only (logging / soft expiry).
            let challenge_id = compute_challenge_id(
                &validator_address,
                &provider_id,
                block_height,
                &chunk_indices,
            );

            // Create challenge message (edge: hex string on SyncMsg)
            let challenge = SyncMsg::CapacityChallenge {
                challenge_id: challenge_id.to_hex(),
                challenger: validator_address,
                provider_id,
                chunk_indices: chunk_indices.clone(),
                block_height,
                merkle_root,
                seed,
                expiration_block,
                timestamp,
            };

            // Subscribe to proof topic before sending challenge
            let proof_topic = format!("{ELD_STORAGE_PROOF_TOPIC_PREFIX}{provider_id}");
            if let Err(e) = p2p_coordinator.subscribe_to_topic(&proof_topic) {
                warn!(
                    provider_id = %provider_id,
                    proof_topic = %proof_topic,
                    error = %e,
                    "Failed to subscribe to proof topic, but continuing with challenge"
                );
            } else {
                info!(
                    provider_id = %provider_id,
                    proof_topic = %proof_topic,
                    "Subscribed to proof topic for provider"
                );
            }

            // Publish to provider-specific challenge topic
            let challenge_topic = format!("{ELD_STORAGE_CHALLENGE_TOPIC_PREFIX}{provider_id}");

            info!(
                challenge_id = %challenge_id,
                provider_id = %provider_id,
                challenger = %validator_address,
                challenge_topic = %challenge_topic,
                chunk_count = chunk_indices.len(),
                block_height = block_height,
                expiration_block = expiration_block,
                "Sending capacity challenge via P2P"
            );
            info!(
                "Publishing challenge to topic provider_id={} topic={}",
                provider_id, challenge_topic
            );

            if let Err(e) = p2p_coordinator.publish_to_topic(&challenge_topic, challenge) {
                error!(
                    provider_id = %provider_id,
                    challenge_topic = %challenge_topic,
                    error = %e,
                    "Failed to publish capacity challenge"
                );
                continue;
            }
        }

        Ok(())
    }
}

#[async_trait]
impl<S> Consensus for ConsensusConnection<S>
where
    S: ConsensusConnectionStorage,
{
    async fn init_chain(&self, init_chain_request: RequestInitChain) -> ResponseInitChain {
        info!("init_chain_request: {:?}", init_chain_request);
        let consensus_config = match self.consensus_config.lock() {
            Ok(config) => config.clone(),
            Err(e) => {
                info!("error reading 'match self.consensus_config.lock(): {:?}", e);
                handle_fatal_eld_error(e.into());
            }
        };

        info!(
            chain_id = %init_chain_request.chain_id,
            expected_chain_id = %consensus_config.chain_id,
            "Initializing chain"
        );

        if init_chain_request.chain_id != consensus_config.chain_id {
            error!(
                "Chain ID mismatch: expected {}, got {}",
                consensus_config.chain_id, init_chain_request.chain_id
            );
            handle_fatal_eld_error(EldError::InitializationError {
                component: "chain id".into(),
                details: "chain ids don't match".into(),
            });
        }

        // Lock committed state to update validators and create accounts
        let mut committed_state = match self.committed_state.lock() {
            Ok(state) => state,
            Err(e) => {
                error!("Failed to lock committed_state in init_chain: {:?}", e);
                handle_fatal_eld_error(e.into());
            }
        };

        // Extract validators from RequestInitChain and convert to ValidatorInfo
        let mut validators = Vec::new();
        for validator_update in init_chain_request.validators {
            // Extract Ed25519 public key from ValidatorUpdate
            let public_key_bytes = match validator_update.pub_key {
                Some(pk) => match pk.sum {
                    Some(Sum::Ed25519(bytes)) => {
                        if bytes.len() != 32 {
                            error!(
                                "Invalid Ed25519 public key length: {} (expected 32)",
                                bytes.len()
                            );
                            continue;
                        }
                        bytes
                    }
                    _ => {
                        warn!("ValidatorUpdate has non-Ed25519 public key, skipping");
                        continue;
                    }
                },
                None => {
                    warn!("ValidatorUpdate has no public key, skipping");
                    continue;
                }
            };

            // Convert public key bytes to VerifyingKey and then to Address
            let public_key_array: [u8; 32] = match public_key_bytes.clone().try_into() {
                Ok(arr) => arr,
                Err(_) => {
                    error!(
                        "Failed to convert public key bytes to array: {}",
                        hex::encode(&public_key_bytes)
                    );
                    continue;
                }
            };
            let verifying_key = match VerifyingKey::from_bytes(&public_key_array) {
                Ok(key) => key,
                Err(e) => {
                    error!("Failed to create VerifyingKey from bytes: {}", e);
                    continue;
                }
            };

            let validator_addr = match Address::from_public_key(&verifying_key) {
                Ok(addr) => addr,
                Err(e) => {
                    error!("Failed to derive address from public key: {}", e);
                    continue;
                }
            };
            let address = validator_addr.to_string();

            info!("validator address: {}", address);

            // Convert power (i64) to stake (u64)
            let stake = if validator_update.power < 0 {
                warn!("Validator has negative power, using 0");
                0
            } else {
                validator_update.power as u64
            };

            // Create ValidatorInfo
            let stake_coin = match Coin::new(stake as u128) {
                Ok(coin) => coin,
                Err(e) => {
                    error!("Failed to create coin for validator stake: {}", e);
                    continue;
                }
            };
            validators.push(ValidatorInfo {
                address: validator_addr,
                stake: stake_coin,
                public_key: public_key_bytes,
            });

            // Create account for validator if it doesn't exist
            let validator_path =
                match CadoPath::new(CadoType::Account, CadoPathKey::Address(validator_addr)) {
                    Ok(path) => path,
                    Err(e) => {
                        error!("Failed to create CadoPath for validator account: {}", e);
                        continue;
                    }
                };

            // Check if account already exists in the working set (no DB read).
            if !committed_state
                .envelope
                .has_cado_in_working_set(&validator_path)
            {
                info!("Staging validator account for first commit: {}", address);
                let validator_balance = match Coin::new(stake as u128) {
                    Ok(coin) => coin,
                    Err(e) => {
                        error!("Failed to create coin for validator stake: {}", e);
                        continue;
                    }
                };
                let validator_account =
                    Account::new(validator_addr, validator_balance, Nonce::new(Nonce::ZERO));

                let serialized = match bincode::serialize(&validator_account) {
                    Ok(data) => data,
                    Err(e) => {
                        error!("Failed to serialize validator account: {}", e);
                        handle_fatal_eld_error(ErrorBuilder::storage_error(
                            "serialize account",
                            &e.to_string(),
                        ));
                    }
                };
                let metadata = CADOMetadata::new(
                    CadoType::Account,
                    validator_account.address().hex_with_prefix(),
                );

                let account_cado = CadoBody::mutable_new(serialized, metadata);
                committed_state
                    .envelope
                    .update_cado_cache(validator_path, account_cado);
            } else {
                info!("Validator account already exists: {}", address);
            }
        }

        // Set validators list in app state
        committed_state.envelope.validators = validators.clone();
        info!(
            "Set validators list in init_chain: {} validators",
            committed_state.envelope.validators.len()
        );

        // Set active validators (deterministic priority order, take top VALIDATORS_PER_EPOCH)
        let mut sorted_validators = validators;
        sorted_validators.sort_by(Self::compare_validator_priority);
        committed_state.envelope.active_validators = sorted_validators
            .into_iter()
            .take(VALIDATORS_PER_EPOCH)
            .collect();

        info!(
            "Set active validators in init_chain: {} validators",
            committed_state.envelope.active_validators.len()
        );

        // Set active capacity validator for epoch 0
        self.select_active_capacity_validator_for_epoch(&mut committed_state, 0);

        info!(
            "Set active capacity validator in init_chain: {:?}",
            committed_state.envelope.active_capacity_validator
        );

        Default::default()
    }

    async fn begin_block(&self, begin_block_request: RequestBeginBlock) -> ResponseBeginBlock {
        if !self
            .local_identity
            .read()
            .map(|g| g.is_configured())
            .unwrap_or(false)
        {
            let rpc_url = self.capacity_manager.config().tendermint_rpc_url.clone();
            ensure_identity_from_status(&self.local_identity, &rpc_url).await;
        }

        // Process evidence of validators missing blocks
        let byzantine_validators = begin_block_request.byzantine_validators;
        for evidence in byzantine_validators {
            if let Some(_validator) = evidence.validator {
                // Find the validator in your system by public key
                //let validator_addr = self.find_validator_by_pubkey(&validator.pub_key);

                // Apply penalty - e.g., reduce stake
                //self.penalize_validator(validator_addr, evidence.height);
            }
        }

        // Process last commit info to track who's voting
        if let Some(last_commit_info) = begin_block_request.last_commit_info {
            for _vote_info in last_commit_info.votes {
                // Track validator participation
                // vote_info.signed_last_block tells you if they signed
            }
        }

        // copy last committed state to current state
        let committed_state = match self.committed_state.lock() {
            Ok(state) => state.clone(),
            Err(e) => {
                handle_recoverable_eld_error(EldError::InitializationError {
                    component: "committed state lock".to_string(),
                    details: e.to_string(),
                });
                return Default::default(); // Return default response on lock failure
            }
        };

        let abci_block_height = begin_block_request
            .header
            .as_ref()
            .map(|h| h.height)
            .unwrap_or(0);

        let this_node_provider_id = self.capacity_manager.config().provider_id;
        let validator_addresses: Vec<String> = committed_state
            .envelope
            .validators
            .iter()
            .map(|v| v.address.to_string())
            .collect();

        info!(
            abci_block_height,
            "validator addresses: {}",
            validator_addresses.join(", ")
        );
        info!(
            abci_block_height,
            this_node_provider_id = %this_node_provider_id,
            "this node capacity provider address"
        );

        let mut current_state = match self.current_state.lock() {
            Ok(state) => state,
            Err(e) => {
                handle_recoverable_eld_error(EldError::InitializationError {
                    component: "current state lock".to_string(),
                    details: e.to_string(),
                });
                return Default::default(); // Return default response on lock failure
            }
        };
        info!(
            "begin_block: committed_state.envelope.committed_cado_cache.len(): {}",
            committed_state.envelope.committed_cado_cache.len()
        );
        *current_state = Some(committed_state);

        Default::default()
    }

    async fn deliver_tx(&self, deliver_tx_request: RequestDeliverTx) -> ResponseDeliverTx {
        let tx_bytes = deliver_tx_request.tx;

        let max_tx_bytes = match self.consensus_config.lock() {
            Ok(config) => config.max_tx_bytes,
            Err(e) => {
                handle_recoverable_eld_error(e.into());
                return response_deliver_tx_error_internal_lock_failed(
                    "consensus config lock error for reading max_tx_bytes".to_string(),
                );
            }
        };
        if tx_bytes.len() > max_tx_bytes {
            warn!(
                tx_size = tx_bytes.len(),
                max_size = max_tx_bytes,
                "Transaction too large in deliver_tx"
            );
            return response_deliver_tx_error_tx_too_large(max_tx_bytes, tx_bytes.len());
        }

        // Step 1: Convert bytes to UTF-8 string with validation
        let hex_str = match String::from_utf8(tx_bytes) {
            Ok(s) => s,
            Err(e) => {
                handle_recoverable_eld_error(EldError::TransactionError {
                    tx_type: "UTF-8 validation".to_string(),
                    details: e.to_string(),
                });
                return response_deliver_tx_error_invalid_utf8_encoding(e.to_string());
            }
        };

        // Step 2: Validate hex string format and size before decoding
        const MAX_HEX_BYTES: usize = 1024 * 1024; // 1MB
        if let Err(e) = validate_hex_string(&hex_str, MAX_HEX_BYTES) {
            handle_recoverable_eld_error(EldError::ValidationError {
                field: "hex_string".to_string(),
                value: "transaction_hex".to_string(),
                details: e.to_string(),
            });
            return response_deliver_tx_error_hex_validation_failed(e.to_string());
        }

        // Step 3: Decode hex with size validation
        let decoded_bytes = match hex::decode(&hex_str) {
            Ok(bytes) => bytes,
            Err(e) => {
                handle_recoverable_eld_error(EldError::ValidationError {
                    field: "hex_encoding".to_string(),
                    value: "transaction_hex".to_string(),
                    details: e.to_string(),
                });
                return response_deliver_tx_error_invalid_hex_encoding(e.to_string());
            }
        };

        // Step 4: Convert decoded bytes to UTF-8 string
        let decoded_json = match String::from_utf8(decoded_bytes) {
            Ok(s) => s,
            Err(e) => {
                handle_recoverable_eld_error(EldError::TransactionError {
                    tx_type: "hex to UTF-8 conversion".to_string(),
                    details: e.to_string(),
                });
                return response_deliver_tx_error_invalid_utf8_encoding(e.to_string());
            }
        };

        // Step 5: Validate JSON string before parsing
        // Use reasonable limits: max 10MB JSON, max 10 levels of nesting
        const MAX_JSON_SIZE: usize = 10 * 1024 * 1024; // 10MB
        const MAX_JSON_DEPTH: usize = 10;
        if let Err(e) = validate_json_string(&decoded_json, MAX_JSON_SIZE, MAX_JSON_DEPTH) {
            info!("JSON too large. Error");
            handle_recoverable_eld_error(e.clone());
            return response_deliver_tx_error_json_validation_failed(e.to_string());
        }

        // Step 6: Parse JSON to Value first for structure validation
        let json_value: serde_json::Value = match serde_json::from_str(&decoded_json) {
            Ok(value) => value,
            Err(e) => {
                handle_recoverable_eld_error(EldError::TransactionError {
                    tx_type: "JSON parsing".to_string(),
                    details: e.to_string(),
                });
                return response_deliver_tx_error_json_parsing_failed(e.to_string());
            }
        };

        // Step 7: Validate transaction structure before deserialization
        if let Err(e) = validate_transaction_structure(&json_value) {
            handle_recoverable_eld_error(e.clone());
            return response_deliver_tx_error_transaction_structure_invalid(e.to_string());
        }

        // Step 9: Deserialize to Tx struct (now safe after validation)
        let tx: Tx = match serde_json::from_str(&decoded_json) {
            Ok(tx) => tx,
            Err(e) => {
                handle_recoverable_eld_error(EldError::TransactionError {
                    tx_type: "JSON deserialization".to_string(),
                    details: e.to_string(),
                });
                return response_deliver_tx_error_json_parsing_failed(e.to_string());
            }
        };

        // Get chain_id from committed state for transaction verification (not current state)
        let chain_id = match self.committed_state.lock() {
            Ok(state) => state.chain_id.clone(),
            Err(e) => {
                handle_recoverable_eld_error(e.into());
                return response_deliver_tx_error_internal_lock_failed(
                    "committed state lock error".to_string(),
                );
            }
        };

        // Verify transaction signature immediately after deserialization but before any changes to current state
        if !tx.verify(&chain_id).unwrap_or(false) {
            error!("Transaction verification failed in deliver_tx - signature invalid");
            return response_deliver_tx_error_verification_failed();
        }

        // Validate dynamic fee
        let fee_config = match self.consensus_config.lock() {
            Ok(config) => config.fee_config.clone(),
            Err(e) => {
                handle_recoverable_eld_error(e.into());
                return response_deliver_tx_error_internal_lock_failed(
                    "consensus config lock for fee validation".to_string(),
                );
            }
        };

        let required_fee = match eld_common::fee::calculate_dynamic_fee(&tx, &fee_config) {
            Ok(fee) => fee,
            Err(e) => {
                handle_recoverable_eld_error(EldError::TransactionError {
                    tx_type: "fee calculation".to_string(),
                    details: e.to_string(),
                });
                return response_deliver_tx_error_fee_calculation_failed(e.to_string());
            }
        };

        let provided_fee = match tx.fee.to_coin() {
            Ok(coin) => coin,
            Err(e) => {
                let error_string = e.to_string();
                handle_recoverable_eld_error(e);
                return response_deliver_tx_error_invalid_coin_amount(error_string);
            }
        };

        if provided_fee < required_fee {
            warn!(
                required_fee = %required_fee,
                provided_fee = %provided_fee,
                "Insufficient fee in deliver_tx"
            );
            return response_deliver_tx_error_insufficient_fee(
                required_fee.amount(),
                tx.fee.as_u128(),
            );
        }

        {
            let sender_addr = tx.payload.inner.sender();
            let sender_address = sender_addr.to_string();

            let mut current_state_lock = match self.current_state.lock() {
                Ok(lock) => lock,
                Err(e) => {
                    handle_recoverable_eld_error(e.into());
                    return response_deliver_tx_error_internal_lock_failed(
                        "current state lock error".to_string(),
                    );
                }
            };
            let current_state = match current_state_lock.as_mut() {
                Some(state) => state,
                None => {
                    handle_recoverable_eld_error(EldError::InitializationError {
                        component: "current state".to_string(),
                        details: "state is None".to_string(),
                    });
                    return response_deliver_tx_error_internal_lock_failed(
                        "current state is None".to_string(),
                    );
                }
            };

            let sender_path =
                match CadoPath::new(CadoType::Account, CadoPathKey::Address(sender_addr)) {
                    Ok(path) => path,
                    Err(e) => {
                        handle_recoverable_eld_error(EldError::ValidationError {
                            field: "account_path".to_string(),
                            value: "sender_account_path".to_string(),
                            details: e.to_string(),
                        });
                        return response_deliver_tx_error_invalid_path_format(e.to_string());
                    }
                };

            let sender_account_with_hash = match current_state
                .envelope
                .get_account_from_cado(&*self.storage, &sender_path)
            {
                Some(account_with_hash) => account_with_hash,
                None => {
                    error!("Sender account not found: {}", sender_address);
                    return response_deliver_tx_error_sender_doesnt_exist(sender_address.clone());
                }
            };
            let sender_account = sender_account_with_hash.account;
            let expected_nonce = match sender_account.nonce().next() {
                Some(nonce) => nonce,
                None => {
                    warn!(
                        account_nonce = sender_account.nonce().value(),
                        tx_nonce = tx.nonce.value(),
                        "Account nonce overflow"
                    );
                    return response_deliver_tx_error_nonce_overflow(
                        sender_account.address().hex_with_prefix(),
                    );
                }
            };
            if expected_nonce != tx.nonce {
                warn!(
                    account_nonce = sender_account.nonce().value(),
                    tx_nonce = tx.nonce.value(),
                    "Nonce is not sequential"
                );
                return response_deliver_tx_error_nonce_not_sequential(
                    expected_nonce.value(),
                    tx.nonce.value(),
                    sender_account.address().hex_with_prefix(),
                );
            }

            let fee = match tx.fee.to_coin() {
                Ok(coin) => coin,
                Err(e) => {
                    let error_string = e.to_string();
                    handle_recoverable_eld_error(e);
                    return response_deliver_tx_error_invalid_coin_amount(error_string);
                }
            };

            // Validate fee amount
            if let Err(e) = validation::validate_fee_amount(&fee) {
                error!(fee = %fee, error = %e, "Fee validation failed");
                return response_deliver_tx_error_fee_validation_failed(e.to_string());
            }

            // Check sender has sufficient funds
            if sender_account.balance() < fee {
                warn!(
                    sender_balance = %sender_account.balance(),
                    required_fee = %fee,
                    "Sender has insufficient balance"
                );
                return response_deliver_tx_error_insufficient_funds();
            }

            // Update sender account nonce and deduct fee
            let updated_balance = match sender_account.balance() - fee {
                Ok(balance) => balance,
                Err(e) => {
                    let error_string = e.to_string();
                    handle_recoverable_eld_error(e);
                    return response_deliver_tx_error_invalid_coin_amount(error_string);
                }
            };

            let updated_sender = Account::new(sender_addr, updated_balance, expected_nonce);

            // TODO: save sender cado
            let sender_serialized = match bincode::serialize(&updated_sender) {
                Ok(serialized) => serialized,
                Err(e) => {
                    handle_recoverable_eld_error(EldError::StorageError {
                        operation: "serialization".to_string(),
                        details: e.to_string(),
                    });
                    return response_deliver_tx_error_internal_lock_failed(
                        "serialization failed".to_string(),
                    );
                }
            };
            let sender_meta = CADOMetadata::new(CadoType::Account, &sender_address);
            let sender_cado = CadoBody::mutable_updated(
                sender_account_with_hash.hash,
                sender_serialized,
                sender_meta,
            );

            current_state
                .envelope
                .update_cado_cache(sender_path, sender_cado);
        }

        tx.process(self).await
    }

    async fn end_block(&self, end_block_request: RequestEndBlock) -> ResponseEndBlock {
        // calculate new epoch
        let new_block_height = end_block_request.height;
        let new_epoch = new_block_height / BLOCKS_PER_EPOCH;
        let validator_updates;
        let mut epoch_challenge_plan: Option<(Address, Vec<Address>, i64)> = None;
        let current_epoch: i64;

        // Scope for mutex lock
        {
            let mut current_state_lock = match self.current_state.lock() {
                Ok(lock) => lock,
                Err(e) => {
                    handle_fatal_eld_error(e.into());
                }
            };

            let current_state = match current_state_lock.as_mut() {
                Some(state) => state,
                None => {
                    handle_fatal_eld_error(EldError::InitializationError {
                        component: "current state".to_string(),
                        details: "state is None in end_block".to_string(),
                    });
                }
            };

            current_epoch = current_state.envelope.current_epoch;

            current_state.envelope.block_height = new_block_height;

            if new_epoch > current_epoch {
                info!(new_epoch = new_epoch, "Starting new epoch");

                // TODO: Resolve how to handle capacity registration lease expiry
                // (re-register, challenge failure, VerifiedProof renewal, etc.).
                // Disabled for now: do not drop/unregister providers when
                // current_block > registered_block + registration_duration.
                //
                // let current_block = new_block_height as u64;
                // let before_count = current_state.envelope.capacity_validators.len();
                // current_state
                //     .envelope
                //     .capacity_validators
                //     .retain(|provider| {
                //         let expired = current_block
                //             > provider.registered_block + provider.registration_duration;
                //         if expired {
                //             tracing::info!(
                //                 address = %provider.address,
                //                 registered_block = provider.registered_block,
                //                 registration_duration = provider.registration_duration,
                //                 current_block = current_block,
                //                 "Capacity registration expired, removing from capacity_validators"
                //             );
                //         }
                //         !expired
                //     });
                // let removed = before_count - current_state.envelope.capacity_validators.len();
                // if removed > 0 {
                //     info!(
                //         removed = removed,
                //         remaining = current_state.envelope.capacity_validators.len(),
                //         "Removed expired capacity registrations from capacity_validators"
                //     );
                // }

                // Calculate and log total reserved capacity across all capacity providers
                let total_reserved_capacity: u64 = current_state
                    .envelope
                    .capacity_validators
                    .iter()
                    .map(|p| p.storage_capacity)
                    .sum();

                // Convert to GB for readability
                let total_capacity_gb = total_reserved_capacity as f64 / (1024.0 * 1024.0 * 1024.0);
                let provider_count = current_state.envelope.capacity_validators.len();

                info!(
                    epoch = new_epoch,
                    total_reserved_capacity_bytes = total_reserved_capacity,
                    total_reserved_capacity_gb = format!("{:.2}", total_capacity_gb),
                    capacity_provider_count = provider_count,
                    "Total reserved capacity on chain"
                );

                self.select_validators_for_epoch(current_state);

                // Select active capacity validator for new epoch
                self.select_active_capacity_validator_for_epoch(current_state, new_epoch);

                let epoch_record = Self::build_epoch_record(current_state, new_epoch);
                if let Err(e) = Self::store_epoch_record(current_state, &epoch_record) {
                    handle_fatal_eld_error(e);
                }

                // Update the current epoch value
                current_state.envelope.current_epoch = new_epoch;
            }

            // Prepare validator updates if needed
            if new_epoch > current_epoch {
                let mut updates = Vec::new();

                // Update all validators in the active set with power = 1
                for validator in &current_state.envelope.active_validators {
                    let pk = PublicKey {
                        sum: Some(Sum::Ed25519(validator.public_key.clone())),
                    };
                    let validator_update = ValidatorUpdate {
                        power: 10, // DEV: set based on staking
                        pub_key: Some(pk),
                    };
                    updates.push(validator_update);
                }
                validator_updates = Some(updates);
            } else {
                validator_updates = None;
            }

            // Calculate app hash before releasing lock
            current_state.app_hash = current_state
                .envelope
                .calculate_hash()
                .to_be_bytes()
                .to_vec();

            // Capture challenger plan only when a new epoch starts (heavy data built after identity check).
            if new_epoch > current_epoch {
                epoch_challenge_plan = current_state
                    .envelope
                    .active_capacity_validator
                    .as_ref()
                    .map(|sv| {
                        (
                            sv.validator_address,
                            sv.challenged_providers.clone(),
                            current_state.envelope.block_height,
                        )
                    });
            }
        }

        if new_epoch > current_epoch {
            if let Some((
                selected_capacity_validator,
                challenged_capacity_provider_ids,
                block_height,
            )) = epoch_challenge_plan
            {
                if self.should_this_node_send_challenges(&selected_capacity_validator) {
                    let providers_data = match self.current_state.lock() {
                        Ok(lock) => match lock.as_ref() {
                            Some(state) => Self::build_challenged_capacity_provider_data(
                                state,
                                &challenged_capacity_provider_ids,
                            ),
                            None => {
                                warn!(
                                    epoch = new_epoch,
                                    "current_state is None; skipping capacity challenges"
                                );
                                Vec::new()
                            }
                        },
                        Err(e) => {
                            warn!(
                                epoch = new_epoch,
                                error = %e,
                                "Failed to lock current_state for capacity challenge data"
                            );
                            Vec::new()
                        }
                    };

                    info!(
                        epoch = new_epoch,
                        selected_capacity_validator = %selected_capacity_validator,
                        challenged_capacity_provider_count = providers_data.len(),
                        "Active capacity validator sending capacity challenges"
                    );
                    if let Err(e) = Self::generate_challenges_internal(
                        providers_data,
                        new_epoch,
                        block_height,
                        selected_capacity_validator,
                        self.p2p_sync_coordinator.clone(),
                    ) {
                        warn!(
                            epoch = new_epoch,
                            error = %e,
                            "Failed to generate and send capacity challenges"
                        );
                    }
                }
            } else {
                info!(
                    epoch = new_epoch,
                    "No active capacity validator selected for epoch"
                );
            }
        }

        // Create validator updates for Tendermint
        let mut resp = ResponseEndBlock::default();

        if let Some(updates) = validator_updates {
            resp.validator_updates = updates;
        }

        resp
    }

    async fn commit(&self, _commit_request: RequestCommit) -> ResponseCommit {
        // Calculate time since last commit
        let mut last_time = self.last_commit_time.lock().unwrap();
        let elapsed = last_time.elapsed().as_secs_f64();
        info!("COMMIT_METRICS: Time since last commit: {:.3}s", elapsed);
        *last_time = Instant::now();

        let current_state_lock = match self.current_state.lock() {
            Ok(lock) => lock,
            Err(e) => {
                handle_fatal_eld_error(e.into());
            }
        };

        let mut current_state = match current_state_lock.as_ref() {
            Some(state) => state.clone(),
            None => {
                handle_fatal_eld_error(EldError::InitializationError {
                    component: "current state".to_string(),
                    details: "state is None in commit".to_string(),
                });
            }
        };

        // Check if we should signal readiness (block_height > 0 means first consensus commit completed)
        if current_state.envelope.block_height > 0 {
            let mut ready_tx_guard = match self.ready_tx.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    return ResponseCommit {
                        data: vec![],
                        retain_height: 0,
                    }
                }
            };
            if let Some(tx) = ready_tx_guard.take() {
                drop(ready_tx_guard); // Release lock before sending
                let _ = tx.send(());
                info!(
                    block_height = current_state.envelope.block_height,
                    "Node is READY: First consensus commit completed, block height > 0"
                );
            }
        }

        // Update CADO type counts
        let mut type_counts = self.cado_type_counts.lock().unwrap();
        for cado in current_state.envelope.cado_cache.values() {
            let cado_type = match cado {
                CadoBody::Mutable(cado_mut) => cado_mut.metadata().type_().to_string(),
                CadoBody::Immutable(cado) => cado.metadata().type_().to_string(),
            };
            *type_counts.entry(cado_type).or_insert(0) += 1;
        }

        // Validate app_hash is not empty before proceeding
        if current_state.app_hash.is_empty() {
            handle_fatal_eld_error(EldError::ValidationError {
                field: "app_hash".to_string(),
                value: "empty".to_string(),
                details: "App hash cannot be empty during commit".to_string(),
            });
        }

        let storage = &self.storage;

        // Check if we need to select new validators for the next epoch
        let new_block_height = current_state.envelope.block_height + 1;
        let new_epoch = new_block_height / BLOCKS_PER_EPOCH;
        let current_epoch = current_state.envelope.current_epoch;

        if new_epoch > current_epoch {
            // Select new validators for the next epoch
            self.select_validators_for_epoch(&mut current_state);
        }

        // Calculate validator rewards
        self.reward_manager
            .calculate_validator_rewards(&mut current_state);

        // Begin a transaction for atomic storage writes.
        // Commit tx order: deletes → puts → pinboard → tip/snapshot.
        let tx = storage.begin_transaction();

        let pending_deletes: Vec<_> = current_state.envelope.cado_cache_to_delete.to_vec();

        for cado_to_delete in &pending_deletes {
            if let Err(e) = storage.system_delete_cado_with_tx(
                cado_to_delete.cado_path.clone(),
                &cado_to_delete.owner,
                &tx,
            ) {
                handle_fatal_eld_error(e);
            }
        }

        for cado_to_delete in &pending_deletes {
            let path_bytes = cado_to_delete.cado_path.as_str().as_bytes();
            current_state
                .envelope
                .cado_cache
                .remove(cado_to_delete.cado_path.as_str());
            current_state
                .envelope
                .committed_cado_cache
                .remove(path_bytes);
            current_state.envelope.state_trie.remove(path_bytes);
        }
        current_state.envelope.cado_cache_to_delete.clear();

        // Write all CADO cache entries to storage explicitly
        let mut failed_cados = Vec::new();
        let _cado_cache_size = current_state.envelope.cado_cache.len();

        for (key, cado) in &current_state.envelope.cado_cache {
            if eld_common::cado::is_infrastructure_cado_path(key) {
                warn!(
                    cado_path = %key,
                    "Skipping infrastructure CADO during commit storage write"
                );
                continue;
            }
            let cado_path = match CadoPath::parse(key) {
                Ok(path) => path,
                Err(e) => {
                    warn!(
                        cado_path = %key,
                        error = %e,
                        "Failed to create CadoPath for cache item, will remove from cache"
                    );
                    failed_cados.push(key.clone());
                    continue;
                }
            };

            if let Err(e) = storage.put_cado_type_with_tx(cado_path.clone(), cado.clone(), &tx) {
                warn!(
                    cado_path = %cado_path,
                    error = %e,
                    "Failed to write CADO to storage, will remove from cache"
                );
                failed_cados.push(key.clone());
                continue;
            }
        }

        // Remove failed CADOs from cache
        if !failed_cados.is_empty() {
            warn!(
                failed_count = failed_cados.len(),
                "Found failed CADOs during commit that will be removed from cache"
            );
            for key in failed_cados {
                current_state.envelope.cado_cache.remove(&key);
            }
        }

        // --------------------------------------------------------------------
        // Pinboard (PostMessage) staged writes
        // --------------------------------------------------------------------
        if !current_state.envelope.pinboard_meta_cache.is_empty()
            || !current_state.envelope.pinboard_idx_wallet_add.is_empty()
            || !current_state.envelope.pinboard_idx_tag_add.is_empty()
            || !current_state.envelope.pinboard_idx_expiry_add.is_empty()
            || !current_state.envelope.pinboard_idx_commit_add.is_empty()
            || !current_state.envelope.pinboard_refcount_deltas.is_empty()
        {
            info!(
                pinboard_meta = current_state.envelope.pinboard_meta_cache.len(),
                pinboard_wallet_idx = current_state.envelope.pinboard_idx_wallet_add.len(),
                pinboard_tag_idx = current_state.envelope.pinboard_idx_tag_add.len(),
                pinboard_expiry_idx = current_state.envelope.pinboard_idx_expiry_add.len(),
                pinboard_commit_idx = current_state.envelope.pinboard_idx_commit_add.len(),
                pinboard_refcount_keys = current_state.envelope.pinboard_refcount_deltas.len(),
                "COMMIT_METRICS: pinboard staged writes"
            );
        }

        let mut pinboard_capacity_blobs: HashMap<String, Vec<u8>> = HashMap::new();
        let mut pinboard_temp_blob_keys_to_delete: HashSet<String> = HashSet::new();

        // Pre-validate temp blobs before any pinboard writes to keep commit all-or-nothing.
        for (message_id, meta) in current_state.envelope.pinboard_meta_cache.iter() {
            match storage.get_pinboard_temp_blob(&meta.content_key) {
                Ok(Some(temp_bytes)) => {
                    pinboard_capacity_blobs.insert(meta.content_key.clone(), temp_bytes.clone());
                    pinboard_temp_blob_keys_to_delete.insert(meta.content_key.clone());
                }
                Ok(None) => {
                    handle_fatal_eld_error(EldError::StorageError {
                        operation: "prevalidate_pinboard_temp_blob".to_string(),
                        details: format!(
                            "Missing pinboard temp blob for message_id={} content_key={}",
                            message_id, meta.content_key
                        ),
                    });
                }
                Err(e) => {
                    handle_fatal_eld_error(EldError::StorageError {
                        operation: "prevalidate_pinboard_temp_blob".to_string(),
                        details: format!(
                            "Failed reading pinboard temp blob for message_id={} content_key={}: {}",
                            message_id, meta.content_key, e
                        ),
                    });
                }
            }
        }

        for (message_id, meta) in current_state.envelope.pinboard_meta_cache.iter() {
            if let Err(e) = storage.put_pinboard_metadata_with_tx(message_id, meta, &tx) {
                handle_fatal_eld_error(e);
            }
        }

        for (wallet, committed_height, message_id) in
            current_state.envelope.pinboard_idx_wallet_add.iter()
        {
            if let Err(e) = storage.put_pinboard_wallet_index_with_tx(
                wallet,
                *committed_height,
                message_id,
                &tx,
            ) {
                handle_fatal_eld_error(e);
            }
        }

        for (tag, committed_height, message_id) in
            current_state.envelope.pinboard_idx_tag_add.iter()
        {
            if let Err(e) =
                storage.put_pinboard_tag_index_with_tx(tag, *committed_height, message_id, &tx)
            {
                handle_fatal_eld_error(e);
            }
        }

        for (expires_height, message_id) in current_state.envelope.pinboard_idx_expiry_add.iter() {
            if let Err(e) =
                storage.put_pinboard_expiry_index_with_tx(*expires_height, message_id, &tx)
            {
                handle_fatal_eld_error(e);
            }
        }

        for (committed_height, message_id) in current_state.envelope.pinboard_idx_commit_add.iter()
        {
            if let Err(e) =
                storage.put_pinboard_commit_index_with_tx(*committed_height, message_id, &tx)
            {
                handle_fatal_eld_error(e);
            }
        }

        for (content_key, delta) in current_state.envelope.pinboard_refcount_deltas.iter() {
            if let Err(e) = storage.update_pinboard_refcount_with_tx(content_key, *delta, &tx) {
                handle_fatal_eld_error(e);
            }
        }

        for content_key in pinboard_temp_blob_keys_to_delete {
            if let Err(e) = storage.delete_pinboard_temp_blob_with_tx(&content_key, &tx) {
                warn!(
                    content_key = %content_key,
                    error = %e,
                    "COMMIT_WARN: failed to delete pinboard temp blob (non-fatal)"
                );
            }
        }

        current_state.envelope.pinboard_meta_cache.clear();
        current_state.envelope.pinboard_idx_wallet_add.clear();
        current_state.envelope.pinboard_idx_tag_add.clear();
        current_state.envelope.pinboard_idx_expiry_add.clear();
        current_state.envelope.pinboard_idx_commit_add.clear();
        current_state.envelope.pinboard_refcount_deltas.clear();

        for challenge_id in current_state.envelope.verified_proof_rewarded_cache.iter() {
            if let Err(e) = storage.put_verified_proof_challenge_rewarded_with_tx(challenge_id, &tx)
            {
                handle_fatal_eld_error(e);
            }
        }
        current_state.envelope.verified_proof_rewarded_cache.clear();

        // --------------------------------------------------------------------
        // Namespace registry (AddNamespace) staged writes
        // --------------------------------------------------------------------
        if !current_state.envelope.namespace_registry_cache.is_empty() {
            info!(
                namespace_count = current_state.envelope.namespace_registry_cache.len(),
                "COMMIT_METRICS: namespace registry staged writes"
            );

            let namespace_entries: Vec<_> = current_state
                .envelope
                .namespace_registry_cache
                .iter()
                .map(|(slug, record)| (slug.clone(), record.clone()))
                .collect();

            for (namespace_slug, record) in namespace_entries {
                let path = match eld_common::namespace::slug_to_namespace_cadopath(&namespace_slug)
                {
                    Ok(path) => path,
                    Err(e) => {
                        error!(
                            namespace_slug = %namespace_slug,
                            error = %e,
                            "COMMIT_ERROR: invalid namespace registry path (skipping)"
                        );
                        continue;
                    }
                };

                let payload = match record.serialize_bin() {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        error!(
                            namespace_slug = %namespace_slug,
                            error = %e,
                            "COMMIT_ERROR: namespace record serialize failed (skipping)"
                        );
                        continue;
                    }
                };

                let cado = CadoBody::immutable(
                    payload,
                    CADOMetadata::new(CadoType::Namespace, &namespace_slug),
                );
                let path_str = path.as_str();
                let path_bytes = path_str.as_bytes();

                if let Err(e) = storage.put_cado_type_with_tx(path.clone(), cado.clone(), &tx) {
                    error!(
                        registry_path = %path_str,
                        error = %e,
                        "COMMIT_ERROR: namespace registry CADO write failed (skipping)"
                    );
                    continue;
                }

                current_state
                    .envelope
                    .committed_cado_cache
                    .insert(path_bytes, cado.clone());
                current_state
                    .envelope
                    .state_trie
                    .insert(path_bytes, &cado.content_hash());
                current_state
                    .envelope
                    .namespace_registry_index
                    .insert(namespace_slug, record);
            }

            current_state.envelope.namespace_registry_cache.clear();
        }

        // Update eld trie with CADO updates
        let updates: Vec<(Vec<u8>, CadoBody)> = current_state
            .envelope
            .cado_cache
            .iter()
            .map(|(key, value)| (key.as_bytes().to_vec(), value.clone()))
            .collect();

        for (key_bytes, value) in &updates {
            let Ok(key_str) = std::str::from_utf8(key_bytes) else {
                continue;
            };
            if eld_common::cado::is_infrastructure_cado_path(key_str) {
                warn!(
                    path = %key_str,
                    "Skipping infrastructure CADO during commit merge into committed_cado_cache"
                );
                continue;
            }
            current_state
                .envelope
                .committed_cado_cache
                .insert(key_bytes, value.clone());
        }
        let epoch_index_paths: Vec<String> =
            current_state.envelope.cado_cache.keys().cloned().collect();
        for key_str in epoch_index_paths {
            current_state
                .envelope
                .insert_epoch_records_index_from_path(&key_str);
        }

        // Merkle Patricia Trie (incremental root for AppStateTip)
        for (key_str, cado) in &current_state.envelope.cado_cache {
            if eld_common::cado::is_infrastructure_cado_path(key_str) {
                continue;
            }
            let value_hash = cado.content_hash();
            current_state
                .envelope
                .state_trie
                .insert(key_str.as_bytes(), &value_hash);
        }

        let trie_root_hash = current_state.envelope.state_trie.root_hash();

        let app_state_tip = AppStateTip {
            block_height: current_state.envelope.block_height,
            cado_root_hash: trie_root_hash, // Store trie hash in cado_root_hash field
            app_hash: current_state.app_hash.clone(),
        };
        let app_state_tip_serialized = match bincode::serialize(&app_state_tip) {
            Ok(serialized) => serialized,
            Err(e) => {
                handle_fatal_eld_error(EldError::StorageError {
                    operation: "serialize_app_state_tip".to_string(),
                    details: e.to_string(),
                });
            }
        };
        let app_state_tip_meta = CADOMetadata::new(CadoType::AppStateTip, "system");
        let app_state_tip_cado =
            CadoBody::immutable(app_state_tip_serialized.clone(), app_state_tip_meta);
        let app_state_tip_hash = Sha256::digest(&app_state_tip_serialized);

        // Store at hash-based path
        let app_state_tip_id = format!("0x{}", hex::encode(app_state_tip_hash));
        let app_state_tip_path =
            match CadoPath::new(CadoType::AppStateTip, CadoPathKey::Name(&app_state_tip_id)) {
                Ok(path) => path,
                Err(e) => {
                    handle_fatal_eld_error(e);
                }
            };

        // Store at fixed "latest" path
        let latest_app_state_tip_path =
            match CadoPath::new(CadoType::AppStateTip, CadoPathKey::Name(LATEST)) {
                Ok(path) => path,
                Err(e) => {
                    handle_fatal_eld_error(e);
                }
            };

        // Store at both paths
        if let Err(e) =
            storage.put_cado_type_with_tx(app_state_tip_path, app_state_tip_cado.clone(), &tx)
        {
            handle_fatal_eld_error(e);
        }
        if let Err(e) =
            storage.put_cado_type_with_tx(latest_app_state_tip_path, app_state_tip_cado, &tx)
        {
            handle_fatal_eld_error(e);
        }

        // Persist trie snapshot once per epoch (at epoch-start block heights).
        let should_create_epoch_snapshot =
            current_state.envelope.block_height % BLOCKS_PER_EPOCH == 0;
        if should_create_epoch_snapshot {
            let snapshot_start = std::time::Instant::now();

            let app_state_snapshot = AppStateSnapshot::new(&current_state);
            let app_state_snapshot_cado = match app_state_snapshot.to_cado() {
                Ok(cado) => cado,
                Err(e) => handle_fatal_eld_error(e),
            };
            let app_state_snapshot_path = match AppStateSnapshot::latest_path() {
                Ok(path) => path,
                Err(e) => handle_fatal_eld_error(e),
            };
            if let Err(e) = storage.put_cado_type_with_tx(
                app_state_snapshot_path.clone(),
                app_state_snapshot_cado,
                &tx,
            ) {
                error!(
                    "COMMIT_ERROR: Failed to store app state snapshot at {}: {}",
                    app_state_snapshot_path.as_str(),
                    e
                );
                // TODO: Should this be fatal?
                handle_fatal_eld_error(e);
            }

            info!(
                "COMMIT_METRICS: app_state_snapshot_persist_ms={:.3} block_height={}",
                snapshot_start.elapsed().as_secs_f64() * 1000.0,
                current_state.envelope.block_height,
            );
        }

        // Announce pinboard blobs that were staged in this block (before clearing cache)
        self.p2p_sync_coordinator
            .find_pinboard_and_broadcast_announce(&current_state.envelope.pinboard_meta_cache);

        // Clear caches
        current_state.envelope.cado_cache.clear();

        // Commit the transaction to persist all changes
        if let Err(e) = tx.commit() {
            handle_fatal_eld_error(EldError::StorageError {
                operation: "commit_transaction".to_string(),
                details: e.to_string(),
            });
        }

        if should_create_epoch_snapshot {
            let snapshot_manager = self.snapshot_manager.clone();
            let snapshot_height = current_state.envelope.block_height;
            tokio::spawn(async move {
                if let Err(e) = snapshot_manager
                    .create_snapshot_from_latest_state(snapshot_height)
                    .await
                {
                    error!(
                        block_height = snapshot_height,
                        error = %e,
                        "Failed to create persisted-state snapshot"
                    );
                }
            });
        }

        let blobs = pinboard_capacity_blobs;
        if !blobs.is_empty() {
            info!(
                count = blobs.len(),
                "Commit: scheduling write of pinboard content to capacity slots after RocksDB commit"
            );
        }
        let cm = self.capacity_manager.clone();
        tokio::spawn(async move {
            for (content_key, bytes) in blobs {
                match cm.get_content_from_slots(&content_key).await {
                    Ok(_) => {
                        debug!(
                            content_key = %content_key,
                            "Pinboard capacity mirror: content already in slots, skipping store_content_chunks"
                        );
                    }
                    Err(_) => {
                        let chunks: Vec<Vec<u8>> =
                            bytes.chunks(MAX_CHUNK_SIZE).map(|c| c.to_vec()).collect();
                        match cm.store_content_chunks(content_key.clone(), chunks).await {
                            Ok(_) => {
                                info!(
                                    content_key = %content_key,
                                    byte_len = bytes.len(),
                                    "PINBOARD_CAPACITY_WRITE_OK: pinboard content written into registered capacity slots after RocksDB commit"
                                );
                            }
                            Err(e) => {
                                error!(
                                    content_key = %content_key,
                                    error = %e,
                                    "Pinboard capacity write failed"
                                );
                            }
                        }
                    }
                }
            }
        });

        let mut committed_state = match self.committed_state.lock() {
            Ok(lock) => lock,
            Err(e) => {
                handle_fatal_eld_error(e.into());
            }
        };
        *committed_state = current_state.clone();
        self.chain_tip.update_from_app_state(&committed_state);

        ResponseCommit {
            data: committed_state.app_hash.clone(),
            retain_height: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abci_interface::chain_tip::ChainTip;
    use crate::abci_interface::snapshot::SnapshotManager;
    use crate::abci_interface::ConsensusConnection;
    use crate::app_state::AppState;
    use crate::capacity::capacity_manager::CapacityManager;
    use crate::config::ConsensusConfig;
    use crate::config::FeeConfig;
    use crate::content::sync::{P2pConfig, P2pSyncCoordinator};
    use crate::storage::hybrid_storage::HybridStorage;
    use crate::storage::rocksdb::RocksDBStorage;
    use crate::wallet::VerifiedProofChainSubmitter;
    use abci::types::RequestDeliverTx;
    use eld_client::config::CliConfig;
    use eld_common::capacity::CapacityConfig;
    use std::sync::{Arc, Mutex};

    fn mock_consensus_connection() -> ConsensusConnection<HybridStorage> {
        let config = ConsensusConfig {
            chain_id: "test-chain".to_string(),
            app_host: "127.0.0.1".to_string(),
            app_port: "8080".to_string(),
            accounts: Default::default(),
            max_tx_bytes: 1024 * 1024,
            fee_config: FeeConfig::default(),
            storage_limits: crate::config::StorageLimits::default(),
        };
        let consensus_config = Arc::new(Mutex::new(config));
        let committed_state = Arc::new(Mutex::new(AppState::default()));
        let current_state = Arc::new(Mutex::new(Some(AppState::default())));
        let rocksdb_dir = tempfile::TempDir::new().unwrap();
        let capacity_dir = rocksdb_dir.path().join("capacity");
        std::fs::create_dir_all(&capacity_dir).unwrap();
        let rocksdb_storage = Arc::new(RocksDBStorage::new(rocksdb_dir.path()).unwrap());
        let storage = Arc::new(HybridStorage::new(rocksdb_storage.clone()));
        let snapshot_manager = Arc::new(SnapshotManager::new(storage.clone()));
        let _reward_manager = ValidatorRewardManager::new(storage.clone());

        // Create a mock P2P sync coordinator for tests
        // Use a separate thread with its own runtime to avoid nested runtime issues
        // Use random ports to avoid conflicts between parallel tests
        use std::sync::atomic::{AtomicU16, Ordering};
        static PORT_COUNTER: AtomicU16 = AtomicU16::new(5000);
        let tcp_port = PORT_COUNTER.fetch_add(2, Ordering::Relaxed);
        let udp_port = tcp_port + 1;

        let p2p_config = P2pConfig { tcp_port, udp_port };
        // Generate a random keypair for tests
        let test_keypair = libp2p::identity::Keypair::generate_ed25519();
        let storage_clone = storage.clone();
        let cli_config = CliConfig {
            node_host: "127.0.0.1".to_string(),
            node_port: "26657".to_string(),
            chain_id: "test-chain".to_string(),
            faucet_host: "127.0.0.1".to_string(),
            faucet_port: "8080".to_string(),
            faucet_end_point: "/faucet/request".to_string(),
            faucet_url: None,
            app_port: "9001".to_string(),
            node_url: None,
            app_url: None,
            p2p_tcp_port: None,
            p2p_udp_port: None,
            single_node: None,
            capacity_size_mb: None,
            capacity_storage_path: None,
            indexer: false,
        };
        let cli = Arc::new(eld_client::facade::ChainClient::new(
            cli_config,
            eld_common::fee::FeeConfig::default(),
        ));
        let verified_proof_submitter = Arc::new(VerifiedProofChainSubmitter::new(
            "wallet1".into(),
            cli.clone(),
            consensus_config.clone(),
            storage.clone(),
        ));
        let local_identity = Arc::new(RwLock::new(LocalNodeIdentity::default()));
        let local_identity_for_p2p = local_identity.clone();
        let (p2p_sync_coordinator, _rx) = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(P2pSyncCoordinator::new(
                p2p_config,
                test_keypair,
                storage_clone,
                verified_proof_submitter,
                local_identity_for_p2p,
            ))
            .map_err(|e| format!("Failed to create P2P coordinator: {e}"))
        })
        .join()
        .map_err(|e| format!("Thread join error: {e:?}"))
        .unwrap()
        .unwrap();
        let p2p_sync_coordinator = Arc::new(p2p_sync_coordinator);
        let chain_tip = Arc::new(ChainTip::default());

        use eld_common::address::Address;

        let test_provider =
            Address::parse_hex_str("0xcccccccccccccccccccccccccccccccccccccccc").expect("provider");
        let capacity_config = CapacityConfig {
            capacity_dir,
            max_capacity_gb: 10,
            provider_id: test_provider,
            auto_register: true,
            registration_retry_interval_secs: 60,
            tendermint_rpc_url: "http://127.0.0.1:26657".to_string(),
        };
        let capacity_manager = Arc::new(CapacityManager::new(
            capacity_config,
            "wallet1".to_string(),
            cli,
            consensus_config.clone(),
        ));

        ConsensusConnection::new(ConsensusConnectionNewContext {
            consensus_config,
            committed_state,
            chain_tip,
            current_state,
            storage,
            snapshot_manager,
            p2p_sync_coordinator,
            ready_tx: None,
            capacity_manager,
            local_identity,
        })
    }

    #[tokio::test]
    async fn test_deliver_tx_invalid_utf8() {
        let consensus = mock_consensus_connection();
        let invalid_tx = vec![0xFF, 0xFF]; // Invalid UTF-8
        let response = consensus
            .deliver_tx(RequestDeliverTx { tx: invalid_tx })
            .await;
        assert_eq!(response.code, 62); // response_deliver_tx_error_invalid_utf8_encoding
        assert!(response.log.contains("Invalid UTF-8 encoding"));
    }

    #[tokio::test]
    async fn test_deliver_tx_invalid_hex() {
        let consensus = mock_consensus_connection();
        let invalid_hex = String::from("invalid_hex").into_bytes();
        let response = consensus
            .deliver_tx(RequestDeliverTx { tx: invalid_hex })
            .await;
        assert_eq!(response.code, 71); // response_deliver_tx_error_hex_validation_failed (now happens before hex decoding)
        assert!(response.log.contains("Hex validation failed"));
    }

    #[tokio::test]
    async fn test_deliver_tx_hex_validation_failed() {
        let consensus = mock_consensus_connection();
        // Test hex string with odd length (invalid hex)
        let odd_length_hex = "aaa"; // 3 characters - odd length
        let response = consensus
            .deliver_tx(RequestDeliverTx {
                tx: odd_length_hex.as_bytes().to_vec(),
            })
            .await;
        assert_eq!(response.code, 71); // response_deliver_tx_error_hex_validation_failed
        assert!(response.log.contains("Hex validation failed"));
    }

    #[tokio::test]
    async fn test_deliver_tx_transaction_structure_invalid() {
        let consensus = mock_consensus_connection();
        // Test invalid transaction structure (missing required fields)
        let invalid_tx_json = r#"{"sig": "test", "nonce": 1}"#; // Missing payload, public_key, fee
        let hex_encoded = hex::encode(invalid_tx_json.as_bytes());
        let response = consensus
            .deliver_tx(RequestDeliverTx {
                tx: hex_encoded.into_bytes(),
            })
            .await;
        assert_eq!(response.code, 73); // response_deliver_tx_error_transaction_structure_invalid
        assert!(response.log.contains("Transaction structure invalid"));
    }

    #[tokio::test]
    async fn test_deliver_tx_deeply_nested_json() {
        let consensus_connection = mock_consensus_connection();

        // Create a deeply nested JSON structure that exceeds the limit
        let deeply_nested =
            r#"{"a":{"b":{"c":{"d":{"e":{"f":{"g":{"h":{"i":{"j":{"k":{"l":"v"}}}}}}}}}}}"#;
        let hex_encoded = hex::encode(deeply_nested.as_bytes());

        let request = abci::types::RequestDeliverTx {
            tx: hex_encoded.as_bytes().to_vec(),
        };

        let response = consensus_connection.deliver_tx(request).await;
        assert_ne!(response.code, 0, "Deeply nested JSON should be rejected");
    }

    fn seed_sender_account(
        state: &mut AppState,
        signing_key: &ed25519_dalek::SigningKey,
        balance: u128,
    ) {
        use eld_common::cado::{CADOMetadata, CadoBody, CadoPath, CadoPathKey, CadoType};
        use sha2::{Digest, Sha256};

        if state.app_hash.is_empty() {
            state.app_hash = Sha256::digest("genesis").to_vec();
        }
        if state.chain_id.is_empty() {
            state.chain_id = "test-chain".to_string();
        }

        let sender =
            Address::from_public_key(&signing_key.verifying_key()).expect("derive address in test");
        let account = Account::new(sender, Coin::new(balance).expect("balance"), Nonce::new(0));
        let serialized = bincode::serialize(&account).expect("serialize account");
        let sender_str = sender.to_string();
        let cado = CadoBody::mutable_new(
            serialized,
            CADOMetadata::new(CadoType::Account, &sender_str),
        );
        let path =
            CadoPath::new(CadoType::Account, CadoPathKey::Address(sender)).expect("account path");
        state.envelope.update_cado_cache(path.clone(), cado.clone());
        state
            .envelope
            .committed_cado_cache
            .insert(path.as_str().as_bytes(), cado);
    }

    fn signed_add_namespace_tx(
        signing_key: &ed25519_dalek::SigningKey,
        slug: &str,
        nonce: u32,
        chain_id: &str,
    ) -> Tx {
        use crate::config::FeeConfig;
        use eld_common::fee::calculate_dynamic_fee;
        use eld_common::tx::{AddNamespaceTx, Payload, TxPublicKey, TxSig};

        let sender =
            Address::from_public_key(&signing_key.verifying_key()).expect("derive address in test");
        let inner =
            AddNamespaceTx::new(sender, slug.to_string(), 1.into()).expect("valid AddNamespaceTx");
        let mut tx = Tx {
            sig: TxSig::empty(),
            nonce: nonce.into(),
            payload: Payload::new(inner),
            public_key: TxPublicKey::from(signing_key.verifying_key()),
            fee: 0.into(),
        };
        let fee_config = FeeConfig::default();
        let required_fee = calculate_dynamic_fee(&tx, &fee_config).expect("fee estimate");
        tx.fee = required_fee.amount().into();
        tx.sign(signing_key, chain_id).expect("sign");
        assert!(tx.verify(chain_id).expect("verify"));
        assert_eq!(
            tx.payload.r#type,
            eld_common::constants::tx_type::TX_TYPE_ADD_NAMESPACE
        );
        tx
    }

    fn hex_encode_tx(tx: &Tx) -> Vec<u8> {
        hex::encode(serde_json::to_string(tx).expect("serialize tx")).into_bytes()
    }

    #[tokio::test]
    async fn test_add_namespace_commit_flushes_registry_to_cache_and_trie() {
        let consensus = mock_consensus_connection();
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]);
        let chain_id = "test-chain";

        {
            let mut current = consensus.current_state.lock().expect("current lock");
            seed_sender_account(
                current.as_mut().expect("current state"),
                &signing_key,
                10_000_000,
            );
        }
        {
            let mut committed = consensus.committed_state.lock().expect("committed lock");
            committed.chain_id = chain_id.to_string();
            let mut current = consensus.current_state.lock().expect("current lock");
            current.as_mut().expect("current state").chain_id = chain_id.to_string();
        }

        let trie_root_before = {
            let current = consensus.current_state.lock().expect("current lock");
            current
                .as_ref()
                .expect("current state")
                .envelope
                .state_trie
                .root_hash()
        };

        let tx = signed_add_namespace_tx(&signing_key, "peter", 1, chain_id);
        let deliver = consensus
            .deliver_tx(RequestDeliverTx {
                tx: hex_encode_tx(&tx),
            })
            .await;
        assert_eq!(deliver.code, 0, "deliver: {}", deliver.log);

        {
            let current = consensus.current_state.lock().expect("current lock");
            let state = current.as_ref().expect("current state");
            assert!(state
                .envelope
                .namespace_registry_cache
                .contains_key("peter"));
            let path = eld_common::namespace::slug_to_namespace_cadopath("peter").expect("path");
            assert!(
                state
                    .envelope
                    .committed_cado_cache
                    .get(path.as_str().as_bytes())
                    .is_none(),
                "not committed until commit()"
            );
        }

        consensus.commit(RequestCommit::default()).await;

        let committed = consensus.committed_state.lock().expect("committed lock");
        assert!(committed.envelope.namespace_registry_cache.is_empty());
        let path = eld_common::namespace::slug_to_namespace_cadopath("peter").expect("path");
        assert!(committed
            .envelope
            .committed_cado_cache
            .get(path.as_str().as_bytes())
            .is_some());
        assert!(committed
            .envelope
            .is_namespace_registered("peter")
            .expect("check"));
        assert_ne!(
            committed.envelope.state_trie.root_hash(),
            trie_root_before,
            "trie root should change after namespace registry insert"
        );
    }

    #[tokio::test]
    async fn test_add_namespace_duplicate_slug_same_block_rejected() {
        let consensus = mock_consensus_connection();
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[4u8; 32]);
        let chain_id = "test-chain";

        {
            let mut current = consensus.current_state.lock().expect("current lock");
            seed_sender_account(
                current.as_mut().expect("current state"),
                &signing_key,
                10_000_000,
            );
            current.as_mut().expect("current state").chain_id = chain_id.to_string();
        }
        {
            let mut committed = consensus.committed_state.lock().expect("committed lock");
            committed.chain_id = chain_id.to_string();
        }

        let tx1 = signed_add_namespace_tx(&signing_key, "peter", 1, chain_id);
        assert_eq!(
            consensus
                .deliver_tx(RequestDeliverTx {
                    tx: hex_encode_tx(&tx1),
                })
                .await
                .code,
            0
        );

        let tx2 = signed_add_namespace_tx(&signing_key, "peter", 2, chain_id);
        let deliver2 = consensus
            .deliver_tx(RequestDeliverTx {
                tx: hex_encode_tx(&tx2),
            })
            .await;
        assert_eq!(deliver2.code, 51, "Layer A: {}", deliver2.log);
        assert!(deliver2.log.contains("already registered"));
    }

    #[tokio::test]
    async fn test_add_namespace_duplicate_slug_next_block_rejected() {
        let consensus = mock_consensus_connection();
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[5u8; 32]);
        let chain_id = "test-chain";

        {
            let mut current = consensus.current_state.lock().expect("current lock");
            seed_sender_account(
                current.as_mut().expect("current state"),
                &signing_key,
                10_000_000,
            );
            current.as_mut().expect("current state").chain_id = chain_id.to_string();
        }
        {
            let mut committed = consensus.committed_state.lock().expect("committed lock");
            committed.chain_id = chain_id.to_string();
        }

        let tx1 = signed_add_namespace_tx(&signing_key, "peter", 1, chain_id);
        assert_eq!(
            consensus
                .deliver_tx(RequestDeliverTx {
                    tx: hex_encode_tx(&tx1),
                })
                .await
                .code,
            0
        );
        consensus.commit(RequestCommit::default()).await;
        {
            let mut committed = consensus.committed_state.lock().expect("committed lock");
            committed.chain_id = chain_id.to_string();
        }

        let tx2 = signed_add_namespace_tx(&signing_key, "peter", 2, chain_id);
        let deliver2 = consensus
            .deliver_tx(RequestDeliverTx {
                tx: hex_encode_tx(&tx2),
            })
            .await;
        assert_eq!(deliver2.code, 51, "Layer B: {}", deliver2.log);
        assert!(deliver2.log.contains("already registered"));
    }

    #[tokio::test]
    #[allow(clippy::needless_update)]
    async fn init_chain_stages_validator_accounts_until_first_commit() {
        use crate::storage::traits::CADOStorage;
        use abci::async_api::Consensus;
        use abci::types::{
            RequestBeginBlock, RequestCommit, RequestEndBlock, RequestInitChain, Sum,
        };
        use eld_common::cado::{CadoPath, CadoPathKey, CadoType};

        let consensus = mock_consensus_connection();
        let storage = consensus.storage.clone();
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        let validator_addr =
            Address::from_public_key(&signing_key.verifying_key()).expect("validator address");
        let account_path =
            CadoPath::new(CadoType::Account, CadoPathKey::Address(validator_addr)).expect("path");

        assert!(
            storage
                .get_cado_by_path(account_path.clone())
                .expect("db read")
                .is_none(),
            "validator account must not exist in db before init_chain"
        );

        consensus
            .init_chain(RequestInitChain {
                chain_id: "test-chain".into(),
                validators: vec![ValidatorUpdate {
                    pub_key: Some(PublicKey {
                        sum: Some(Sum::Ed25519(
                            signing_key.verifying_key().to_bytes().to_vec(),
                        )),
                    }),
                    power: 100,
                }],
                ..Default::default()
            })
            .await;

        {
            let committed = consensus.committed_state.lock().expect("committed lock");
            assert!(
                committed
                    .envelope
                    .cado_cache
                    .contains_key(account_path.as_str()),
                "init_chain must stage validator account in cado_cache"
            );
            assert!(
                committed
                    .envelope
                    .committed_cado_cache
                    .get(account_path.as_str().as_bytes())
                    .is_none(),
                "committed_cado_cache must stay empty until commit"
            );
            assert_eq!(committed.envelope.validators.len(), 1);
        }

        assert!(
            storage
                .get_cado_by_path(account_path.clone())
                .expect("db read after init_chain")
                .is_none(),
            "init_chain must not persist validator accounts to db"
        );

        consensus
            .begin_block(RequestBeginBlock {
                header: Some(abci::types::Header {
                    height: 1,
                    chain_id: "test-chain".into(),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .await;
        consensus
            .end_block(RequestEndBlock {
                height: 1,
                ..Default::default()
            })
            .await;
        consensus.commit(RequestCommit::default()).await;

        assert!(
            storage
                .get_cado_by_path(account_path.clone())
                .expect("db read after commit")
                .is_some(),
            "first commit must persist staged validator account"
        );
        {
            let committed = consensus.committed_state.lock().expect("committed lock");
            assert!(committed
                .envelope
                .committed_cado_cache
                .get(account_path.as_str().as_bytes())
                .is_some());
            assert!(!committed
                .envelope
                .cado_cache
                .contains_key(account_path.as_str()));
        }
    }

    #[test]
    fn build_challenged_capacity_provider_data_skips_unregistered_providers() {
        use ed25519_dalek::SigningKey;
        use eld_common::address::Address;
        use eld_common::coin::Coin;
        use eld_common::public_key::PublicKey;
        use eld_common::validator::CapacityValidatorInfo;

        let registered = Address::parse_hex_str("0x2222222222222222222222222222222222222222")
            .expect("registered provider");
        let unregistered = Address::parse_hex_str("0x3333333333333333333333333333333333333333")
            .expect("unregistered provider");

        let mut state = AppState::default();
        state.envelope.capacity_validators = vec![CapacityValidatorInfo {
            address: registered,
            stake: Coin::zero(),
            public_key: PublicKey::from(SigningKey::from_bytes(&[7u8; 32]).verifying_key()),
            storage_capacity: 1_000,
            merkle_root: Some([9u8; 32]),
            seed: Some([8u8; 32]),
            chunk_count: Some(50),
            registered_at: Some(1),
            last_merkle_root_update: Some(1),
            registered_block: 1,
            registration_duration: 1000,
        }];

        let data = ConsensusConnection::<HybridStorage>::build_challenged_capacity_provider_data(
            &state,
            &[registered, unregistered],
        );

        assert_eq!(data.len(), 1);
        assert_eq!(data[0].0, registered);
        assert_eq!(data[0].1, Some([9u8; 32]));
        assert_eq!(data[0].2, Some([8u8; 32]));
        assert_eq!(data[0].3, Some(50));
    }
}
