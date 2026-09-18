use std::collections::BTreeMap;

use crate::error::EldError;
use bincode;
use serde::{Deserialize, Serialize};
use serde_json;

#[derive(Debug, Default, Clone, Serialize, Deserialize, Hash, PartialEq)]
pub struct ContractInfo {
    pub contract_id: String,
    /// Remains `String` (not [`crate::address::Address`]): usually the deployer account
    /// hex, but instantiate-from-contract stores the sender **contract id** (32-byte hex)
    /// here. Changing to `Address` would break that path and existing bincode/JSON blobs.
    pub owner: String,
    pub bytecode: Vec<u8>,
    pub active: bool,      // For potential deactivation
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, Hash, PartialEq)]
pub struct ContractState {
    pub contract_id: String, // the contract which this storage serves
    /// Same as [`ContractInfo::owner`]: account hex or contract id depending on call path.
    pub owner: String,
    pub state_root: Vec<u8>, // Merkle root of contract's state trie
    pub state_data: BTreeMap<String, Vec<u8>>, // The actual key-value pairs
}

impl ContractInfo {
    /// Serialize `ContractInfo` into bytes for storage.
    pub fn serialize_bin(&self) -> Result<Vec<u8>, EldError> {
        bincode::serialize(self).map_err(|e| EldError::StorageError {
            operation: "serialize_contract_info".to_string(),
            details: format!("Failed to serialize ContractInfo: {e}"),
        })
    }

    /// Deserialize `ContractInfo` from bytes retrieved from storage.
    pub fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        bincode::deserialize(data).map_err(|e| EldError::StorageError {
            operation: "deserialize_contract_info".to_string(),
            details: format!("Failed to deserialize ContractInfo: {e}"),
        })
    }

    /// Serialize `ContractInfo` into JSON.
    pub fn to_json(&self) -> Result<String, EldError> {
        serde_json::to_string(self).map_err(|e| EldError::StorageError {
            operation: "serialize_contract_info_json".to_string(),
            details: format!("Failed to serialize ContractInfo to JSON: {e}"),
        })
    }

    /// Deserialize `ContractInfo` from JSON.
    pub fn from_json(json: &str) -> Result<Self, EldError> {
        serde_json::from_str(json).map_err(|e| EldError::StorageError {
            operation: "deserialize_contract_info_json".to_string(),
            details: format!("Failed to deserialize ContractInfo from JSON: {e}"),
        })
    }
}

impl crate::cado::DeserializableBin for ContractInfo {
    fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        ContractInfo::deserialize_bin(data)
    }
}

impl ContractState {
    /// Serialize `ContractState` into bytes for storage.
    pub fn serialize_bin(&self) -> Result<Vec<u8>, EldError> {
        bincode::serialize(self).map_err(|e| EldError::StorageError {
            operation: "serialize_contract_state".to_string(),
            details: format!("Failed to serialize ContractState: {e}"),
        })
    }

    /// Deserialize `ContractState` from bytes retrieved from storage.
    pub fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        bincode::deserialize(data).map_err(|e| EldError::StorageError {
            operation: "deserialize_contract_state".to_string(),
            details: format!("Failed to deserialize ContractState: {e}"),
        })
    }

    /// Serialize `ContractState` into JSON.
    pub fn to_json(&self) -> Result<String, EldError> {
        serde_json::to_string(self).map_err(|e| EldError::StorageError {
            operation: "serialize_contract_state_json".to_string(),
            details: format!("Failed to serialize ContractState to JSON: {e}"),
        })
    }

    /// Deserialize `ContractState` from JSON.
    pub fn from_json(json: &str) -> Result<Self, EldError> {
        serde_json::from_str(json).map_err(|e| EldError::StorageError {
            operation: "deserialize_contract_state_json".to_string(),
            details: format!("Failed to deserialize ContractState from JSON: {e}"),
        })
    }
}

impl crate::cado::DeserializableBin for ContractState {
    fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        ContractState::deserialize_bin(data)
    }
}
