use crate::address::Address;
use crate::coin::Coin;
use crate::error::EldError;
use crate::logging::{LogSanitizer, SanitizedLoggable};
use bincode;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StakingAccount {
    pub address: Address,    // Address of the staking account
    pub stake_balance: Coin, // Amount staked
    pub originator: Address, // Validator address that created this staking account
}

impl Hash for StakingAccount {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.address.hash(state);
        self.stake_balance.hash(state);
        self.originator.hash(state);
    }
}

impl std::fmt::Display for StakingAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "StakingAccount {{ address: {}, stake_balance: {}, originator: {} }}",
            self.address, self.stake_balance, self.originator
        )
    }
}

impl SanitizedLoggable for StakingAccount {
    fn sanitized_log(&self) -> String {
        format!(
            "StakingAccount {{ address: {}, stake_balance: {} units, originator: {} }}",
            LogSanitizer::sanitize_address(&self.address.hex_with_prefix()),
            self.stake_balance,
            LogSanitizer::sanitize_address(&self.originator.hex_with_prefix())
        )
    }
}

impl StakingAccount {
    /// Serializes this staking account to a binary representation using bincode.
    pub fn serialize_bin(&self) -> Result<Vec<u8>, EldError> {
        bincode::serialize(self).map_err(|e| EldError::StorageError {
            operation: "serialize_staking_account".to_string(),
            details: format!("Failed to serialize StakingAccount: {e}"),
        })
    }

    /// Deserializes a staking account from a binary representation using bincode.
    pub fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        bincode::deserialize(data).map_err(|e| EldError::StorageError {
            operation: "deserialize_staking_account".to_string(),
            details: format!("Failed to deserialize StakingAccount: {e}"),
        })
    }
}

impl crate::cado::DeserializableBin for StakingAccount {
    fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        StakingAccount::deserialize_bin(data)
    }
}
