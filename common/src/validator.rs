use crate::address::Address;
use crate::coin::Coin;
use crate::public_key::PublicKey;
use bincode;
use hex;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct ValidatorInfo {
    pub address: Address,
    pub stake: Coin,
    #[serde(with = "hex_vec_u8")]
    pub public_key: Vec<u8>,
}

impl std::fmt::Display for ValidatorInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ValidatorInfo {{ address: {}, stake: {}, public_key: {} }}",
            self.address,
            self.stake,
            hex::encode(&self.public_key)
        )
    }
}

// Add serde helper module for hex string to Vec<u8> conversion
mod hex_vec_u8 {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        hex::decode(s).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ActiveValidatorsInfo {
    pub validators: Vec<ValidatorInfo>,
    pub total_stake: Coin,
    pub current_epoch: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AllValidatorsInfo {
    pub validators: Vec<ValidatorInfo>,
    pub total_stake: Coin,
    pub current_epoch: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CapacityValidatorsInfo {
    /// All registered capacity validators
    pub capacity_validators: Vec<CapacityValidatorInfo>,
    /// Total stake across registered capacity validators
    pub total_stake: Coin,
    /// Total capacity across registered capacity validators (bytes)
    pub total_capacity: u64,
    pub current_epoch: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EpochInfo {
    pub current_epoch: i64,
    pub current_block: i64,
    pub blocks_per_epoch: i64,
    pub validators_per_epoch: usize,
    pub blocks_until_next_epoch: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StakingAccountInfo {
    pub address: Address,
    pub stake_balance: Coin,
    pub originator: Address,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub enum StakingOperation {
    Stake,
    Unstake,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StorageStakingAccount {
    /// Address of the capacity provider
    pub address: Address,
    /// Amount staked
    pub stake_balance: Coin,
    /// Address that created this account (same as address for self-stake)
    pub originator: Address,
    /// Total storage capacity declared (in bytes)
    pub storage_capacity: u64,
}

impl Hash for StorageStakingAccount {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.address.hash(state);
        self.stake_balance.hash(state);
        self.originator.hash(state);
        self.storage_capacity.hash(state);
    }
}

impl StorageStakingAccount {
    /// Serializes this account for CADO storage (`storage_staking_account`).
    pub fn serialize_bin(&self) -> Result<Vec<u8>, crate::error::EldError> {
        bincode::serialize(self).map_err(|e| crate::error::EldError::StorageError {
            operation: "serialize_storage_staking_account".to_string(),
            details: format!("Failed to serialize StorageStakingAccount: {e}"),
        })
    }

    /// Deserializes from CADO payload bytes.
    pub fn deserialize_bin(data: &[u8]) -> Result<Self, crate::error::EldError> {
        bincode::deserialize(data).map_err(|e| crate::error::EldError::StorageError {
            operation: "deserialize_storage_staking_account".to_string(),
            details: format!("Failed to deserialize StorageStakingAccount: {e}"),
        })
    }
}

impl crate::cado::DeserializableBin for StorageStakingAccount {
    fn deserialize_bin(data: &[u8]) -> Result<Self, crate::error::EldError> {
        StorageStakingAccount::deserialize_bin(data)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct CapacityValidatorInfo {
    pub address: Address,
    pub stake: Coin,
    /// Ed25519 public key frozen on first RegisterCapacity (outer tx pubkey).
    #[serde(default)]
    pub public_key: PublicKey,
    pub storage_capacity: u64, // Total storage capacity in bytes
    // Capacity proof fields
    #[serde(with = "hex_vec_u8_32")]
    pub merkle_root: Option<[u8; 32]>, // Merkle root of capacity proof
    #[serde(with = "hex_vec_u8_32")]
    pub seed: Option<[u8; 32]>, // Seed for capacity proof
    pub chunk_count: Option<u32>,             // Number of chunks
    pub registered_at: Option<u64>,           // Block height when registered
    pub last_merkle_root_update: Option<u64>, // Block height of last root update
    /// Block at which capacity was registered (block when RegisterCapacity tx was applied). Set when the RegisterCapacity tx is processed. 0 after expiry or before any registration.
    pub registered_block: u64,
    /// Registration duration in blocks; capacity expires when current_block > registered_block + registration_duration. Set when the RegisterCapacity tx is processed. 0 after expiry or before any registration.
    pub registration_duration: u64,
}

/// Active capacity validator for the current epoch (issues challenges).
#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct ActiveCapacityValidator {
    /// Address of the active capacity validator
    pub validator_address: Address,
    /// Epoch this validator is active for
    pub epoch: i64,
    /// Capacity-validator addresses being challenged this epoch
    pub challenged_providers: Vec<Address>,
    /// Response topics currently subscribed to (for receiving proof responses)
    pub subscribed_topics: Vec<String>,
}

impl std::fmt::Display for ActiveCapacityValidator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ActiveCapacityValidator {{ validator: {}, epoch: {}, challenged_providers: {}, subscribed_topics: {} }}",
            self.validator_address,
            self.epoch,
            self.challenged_providers.len(),
            self.subscribed_topics.len()
        )
    }
}

/// Historical snapshot persisted at each epoch boundary (CADO `epoch_record`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpochRecord {
    pub epoch: i64,
    pub start_block: i64,
    pub active_validators: Vec<ValidatorInfo>,
    #[serde(alias = "storage_validator")]
    pub active_capacity_validator: Option<ActiveCapacityValidator>,
    #[serde(alias = "challenged_capacity_providers")]
    pub challenged_capacity_validators: Vec<CapacityValidatorInfo>,
}

impl EpochRecord {
    /// Serializes this record for CADO storage (`epoch_record`).
    pub fn serialize_bin(&self) -> Result<Vec<u8>, crate::error::EldError> {
        bincode::serialize(self).map_err(|e| crate::error::EldError::StorageError {
            operation: "serialize_epoch_record".to_string(),
            details: format!("Failed to serialize EpochRecord: {e}"),
        })
    }

    /// Deserializes from CADO payload bytes.
    pub fn deserialize_bin(data: &[u8]) -> Result<Self, crate::error::EldError> {
        bincode::deserialize(data).map_err(|e| crate::error::EldError::StorageError {
            operation: "deserialize_epoch_record".to_string(),
            details: format!("Failed to deserialize EpochRecord: {e}"),
        })
    }
}

impl crate::cado::DeserializableBin for EpochRecord {
    fn deserialize_bin(data: &[u8]) -> Result<Self, crate::error::EldError> {
        EpochRecord::deserialize_bin(data)
    }
}

// Helper module for serializing [u8; 32] as hex string
mod hex_vec_u8_32 {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S>(bytes: &Option<[u8; 32]>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match bytes {
            Some(b) => serializer.serialize_some(&hex::encode(b)),
            None => serializer.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Option<[u8; 32]>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: Option<String> = Option::deserialize(deserializer)?;
        match s {
            Some(hex_str) => {
                let bytes = hex::decode(hex_str).map_err(serde::de::Error::custom)?;
                if bytes.len() != 32 {
                    return Err(serde::de::Error::custom("Expected 32 bytes"));
                }
                let mut array = [0u8; 32];
                array.copy_from_slice(&bytes);
                Ok(Some(array))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(hex: &str) -> Address {
        Address::parse_hex_str(hex).expect("test address")
    }

    const VALIDATOR: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ORIGINATOR: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn epoch_record_bincode_round_trip() {
        let record = EpochRecord {
            epoch: 3,
            start_block: 60,
            active_validators: vec![ValidatorInfo {
                address: addr(VALIDATOR),
                stake: Coin::new(100).expect("coin"),
                public_key: vec![1, 2, 3],
            }],
            active_capacity_validator: None,
            challenged_capacity_validators: vec![],
        };
        let bytes = record.serialize_bin().expect("serialize");
        let restored = EpochRecord::deserialize_bin(&bytes).expect("deserialize");
        assert_eq!(restored.epoch, 3);
        assert_eq!(restored.start_block, 60);
        assert_eq!(restored.active_validators.len(), 1);
        assert_eq!(restored.active_validators[0].address, addr(VALIDATOR));
    }

    #[test]
    fn validator_info_address_json_round_trip() {
        let info = ValidatorInfo {
            address: addr(VALIDATOR),
            stake: Coin::new(100).expect("coin"),
            public_key: vec![0x11; 32],
        };
        let json = serde_json::to_string(&info).expect("serialize");
        assert!(json.contains(VALIDATOR));
        let restored: ValidatorInfo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.address, addr(VALIDATOR));
    }

    #[test]
    fn staking_account_info_address_json_round_trip() {
        let info = StakingAccountInfo {
            address: addr(VALIDATOR),
            stake_balance: Coin::new(50).expect("coin"),
            originator: addr(ORIGINATOR),
        };
        let json = serde_json::to_string(&info).expect("serialize");
        assert!(json.contains(VALIDATOR));
        assert!(json.contains(ORIGINATOR));
        let restored: StakingAccountInfo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.address, addr(VALIDATOR));
        assert_eq!(restored.originator, addr(ORIGINATOR));
    }

    #[test]
    fn storage_staking_account_address_json_and_bincode_round_trip() {
        let account = StorageStakingAccount {
            address: addr(VALIDATOR),
            stake_balance: Coin::new(25).expect("coin"),
            originator: addr(ORIGINATOR),
            storage_capacity: 1_024,
        };
        let json = serde_json::to_string(&account).expect("serialize json");
        assert!(json.contains(VALIDATOR));
        let from_json: StorageStakingAccount =
            serde_json::from_str(&json).expect("deserialize json");
        assert_eq!(from_json, account);

        let bytes = account.serialize_bin().expect("serialize bin");
        let from_bin = StorageStakingAccount::deserialize_bin(&bytes).expect("deserialize bin");
        assert_eq!(from_bin, account);
    }
}
