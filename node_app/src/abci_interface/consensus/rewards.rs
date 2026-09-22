use crate::app_state::AppState;
use crate::errors::handle_recoverable_eld_error;
use eld_common::coin::Coin;
use eld_common::constants::protocol::BLOCK_REWARD;
use eld_common::storage::AccountStorage;
use std::sync::Arc;
use tracing::error;

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
    pub(crate) fn new(storage: Arc<T>) -> Self {
        Self { _storage: storage }
    }

    pub(crate) fn calculate_validator_rewards(&self, current_state: &mut AppState) {
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
