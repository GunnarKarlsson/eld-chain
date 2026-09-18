use crate::coin::Coin;
use crate::error::EldError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorConfig {
    /// The fee charged for each transaction
    pub transaction_fee: Coin,

    /// The amount of new coins minted for each block
    pub block_reward: Coin,

    /// The minimum amount that can be staked
    pub min_stake: Coin,

    /// The maximum number of validators per epoch
    pub max_validators: usize,

    /// The number of blocks per epoch
    pub blocks_per_epoch: i64,
}

impl Default for ValidatorConfig {
    fn default() -> Self {
        Self {
            transaction_fee: Coin::new(500).expect("Can create coin"), // 3 coin per transaction
            block_reward: Coin::new(10).expect("Can create coin"),     // 10 new coins per block
            min_stake: Coin::new(100).expect("Can create coin"),       // Minimum 100 coins to stake
            max_validators: 100,    // Maximum 100 validators per epoch
            blocks_per_epoch: 1000, // 1000 blocks per epoch
        }
    }
}

impl ValidatorConfig {
    pub fn new(
        transaction_fee: Coin,
        block_reward: Coin,
        min_stake: Coin,
        max_validators: usize,
        blocks_per_epoch: i64,
    ) -> Self {
        Self {
            transaction_fee,
            block_reward,
            min_stake,
            max_validators,
            blocks_per_epoch,
        }
    }

    /// Validates the configuration
    pub fn validate(&self) -> Result<(), EldError> {
        if self.transaction_fee == Coin::new(0).expect("Can create coin") {
            return Err(EldError::ValidationError {
                field: "transaction_fee".to_string(),
                value: self.transaction_fee.to_string(),
                details: "Transaction fee cannot be zero".to_string(),
            });
        }
        if self.block_reward == Coin::new(0).expect("Can create coin") {
            return Err(EldError::ValidationError {
                field: "block_reward".to_string(),
                value: self.block_reward.to_string(),
                details: "Block reward cannot be zero".to_string(),
            });
        }
        if self.min_stake == Coin::new(0).expect("Can create coin") {
            return Err(EldError::ValidationError {
                field: "min_stake".to_string(),
                value: self.min_stake.to_string(),
                details: "Minimum stake cannot be zero".to_string(),
            });
        }
        if self.max_validators == 0 {
            return Err(EldError::ValidationError {
                field: "max_validators".to_string(),
                value: self.max_validators.to_string(),
                details: "Maximum validators cannot be zero".to_string(),
            });
        }
        if self.blocks_per_epoch <= 0 {
            return Err(EldError::ValidationError {
                field: "blocks_per_epoch".to_string(),
                value: self.blocks_per_epoch.to_string(),
                details: "Blocks per epoch must be positive".to_string(),
            });
        }
        Ok(())
    }
}
