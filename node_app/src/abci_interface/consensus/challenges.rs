use super::connection::ConsensusConnection;
use crate::app_state::AppState;
use crate::content::sync::{P2pCoordinatorTrait, SyncMsg};
use crate::storage::traits::ConsensusConnectionStorage;
use eld_common::address::Address;
use eld_common::capacity_challenge::{compute_challenge_id, select_challenge_chunk_indices};
use eld_common::constants::p2p::{
    ELD_STORAGE_CHALLENGE_TOPIC_PREFIX, ELD_STORAGE_PROOF_TOPIC_PREFIX,
};
use eld_common::error::EldError;
use std::sync::Arc;
use tracing::{error, info, warn};

pub(crate) type ChallengedCapacityProviderData =
    (Address, Option<[u8; 32]>, Option<[u8; 32]>, Option<u32>);

impl<S> ConsensusConnection<S>
where
    S: ConsensusConnectionStorage,
{
    /// Returns true when this process is the epoch's active capacity validator
    /// (local capacity-validator **wallet** address matches on-chain selected address).
    pub(crate) fn should_this_node_send_challenges(&self, selected_validator: &Address) -> bool {
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

    pub(crate) fn build_challenged_capacity_provider_data(
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
    pub(crate) fn generate_challenges_internal(
        providers_data: Vec<ChallengedCapacityProviderData>,
        epoch: i64,
        block_height: i64,
        validator_address: Address,
        p2p_coordinator: Arc<dyn P2pCoordinatorTrait>,
        chunks_per_challenge: usize,
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
                chunks_per_challenge,
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
