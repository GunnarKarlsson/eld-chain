use crate::coin::Coin;
use crate::error::EldError;
use crate::logging::{LogSanitizer, SanitizedLoggable};
use crate::nonce::Nonce;
use crate::Address;
use bincode;
use core::fmt;
use serde::{Deserialize, Serialize};
use std::hash::Hash;

#[derive(Debug, Clone, Serialize, Deserialize, Hash, PartialEq)]
pub struct Account {
    address: Address,
    balance: Coin,
    nonce: Nonce,
}

impl Account {
    /// Creates a new account with the given address, balance, and nonce.
    pub fn new(address: Address, balance: Coin, nonce: Nonce) -> Self {
        Self {
            address,
            balance,
            nonce,
        }
    }

    /// Returns a reference to the account's address.
    pub fn address(&self) -> &Address {
        &self.address
    }

    /// Returns the account's balance (Copy, so by value).
    pub fn balance(&self) -> Coin {
        self.balance
    }

    /// Returns the account's nonce.
    pub fn nonce(&self) -> Nonce {
        self.nonce
    }

    /// Serializes this account to a binary representation using bincode.
    pub fn serialize_bin(&self) -> Result<Vec<u8>, EldError> {
        bincode::serialize(self).map_err(|e| EldError::StorageError {
            operation: "serialize_account".to_string(),
            details: format!("Failed to serialize Account: {e}"),
        })
    }

    pub fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        bincode::deserialize(data).map_err(|e| EldError::ValidationError {
            field: "account_data".to_string(),
            value: format!("{data:?}"),
            details: format!("Failed to deserialize Account: {e}"),
        })
    }
}

impl crate::cado::DeserializableBin for Account {
    fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        Account::deserialize_bin(data)
    }
}

impl fmt::Display for Account {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "\taddress: \t{}\n\tbalance: \t{}\n\tnonce: \t\t{}\n\n",
            self.address(),
            self.balance(),
            self.nonce()
        )
    }
}

impl SanitizedLoggable for Account {
    fn sanitized_log(&self) -> String {
        format!(
            "Account {{ address: {}, balance: {} units, nonce: {} }}",
            LogSanitizer::sanitize_address(&self.address().hex_with_prefix()),
            self.balance(),
            self.nonce()
        )
    }
}
