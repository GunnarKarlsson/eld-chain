use super::connection::ConsensusConnection;
use crate::errors::handle_fatal_eld_error;
use crate::storage::traits::ConsensusConnectionStorage;
use abci::types::*;
use eld_common::address::Address;
use eld_common::constants::protocol::BLOCKS_PER_EPOCH;
use eld_common::error::EldError;
use tracing::{info, warn};

impl<S> ConsensusConnection<S>
where
    S: ConsensusConnectionStorage,
{
    pub(crate) async fn end_block_inner(
        &self,
        end_block_request: RequestEndBlock,
    ) -> ResponseEndBlock {
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
}
