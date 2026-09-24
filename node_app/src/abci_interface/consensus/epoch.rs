use super::connection::ConsensusConnection;
use crate::app_state::AppState;
use crate::storage::traits::ConsensusConnectionStorage;
use eld_common::address::Address;
use eld_common::cado::{
    epoch_record_path_name, CADOMetadata, CadoBody, CadoPath, CadoPathKey, CadoType,
};
use eld_common::constants::{cado::LATEST, p2p::ELD_STORAGE_PROOF_TOPIC_PREFIX};
use eld_common::error::EldError;
use eld_common::validator::{ActiveCapacityValidator, EpochRecord, ValidatorInfo};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use tracing::warn;

impl<S> ConsensusConnection<S>
where
    S: ConsensusConnectionStorage,
{
    pub(crate) fn compare_validator_priority(a: &ValidatorInfo, b: &ValidatorInfo) -> Ordering {
        // Deterministic ordering is required for consensus safety when stakes tie.
        b.stake
            .cmp(&a.stake)
            .then_with(|| a.address.cmp(&b.address))
            .then_with(|| a.public_key.cmp(&b.public_key))
    }

    pub(crate) fn select_validators_for_epoch(&self, current_state: &mut AppState) {
        // Create a copy of validators list to sort
        let mut validators = current_state.envelope.validators.clone();

        // Sort deterministically so equal-stake validators resolve identically on all nodes.
        validators.sort_by(Self::compare_validator_priority);

        let active_validators: Vec<ValidatorInfo> = validators
            .into_iter()
            .take(self.protocol_constants().validators_per_epoch)
            .collect();

        // Update active validators list
        current_state.envelope.active_validators = active_validators;
    }

    /// Select the epoch's active capacity validator from on-chain `capacity_validators`
    /// and choose challenge recipients (self-challenge only when a single eligible exists).
    pub(crate) fn select_active_capacity_validator_for_epoch(
        &self,
        current_state: &mut AppState,
        epoch: i64,
    ) {
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
            let num_to_challenge = self
                .protocol_constants()
                .challenges_per_epoch
                .min(challenge_pool.len());
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

    pub(crate) fn build_epoch_record(
        current_state: &AppState,
        new_epoch: i64,
        blocks_per_epoch: i64,
    ) -> EpochRecord {
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
            start_block: new_epoch * blocks_per_epoch,
            active_validators: current_state.envelope.active_validators.clone(),
            active_capacity_validator: current_state.envelope.active_capacity_validator.clone(),
            challenged_capacity_validators,
        }
    }

    pub(crate) fn store_epoch_record(
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
}
