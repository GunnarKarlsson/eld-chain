//! Strongly typed contract identifier.
//!
//! Canonical display is `0x` + 64 lowercase hex digits; parsing accepts inputs with or without
//! the `0x` prefix for boundary normalization.

use crate::error::EldError;
use hex;
use serde::de::{Error as SerdeError, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ContractId {
    bytes: [u8; Self::LEN],
}

impl ContractId {
    pub const LEN: usize = 32;

    #[must_use]
    pub fn new(bytes: [u8; Self::LEN]) -> Self {
        Self { bytes }
    }

    pub fn from_input(s: &str) -> Result<Self, EldError> {
        Self::parse_hex(s)
    }

    /// Parses an optional string field into a canonical contract id.
    ///
    /// Accepts inputs with or without `0x` prefix and normalizes to canonical output via
    /// [`std::fmt::Display`].
    pub fn parse(input: Option<&str>) -> Result<Self, EldError> {
        let s = input.ok_or_else(|| EldError::ValidationError {
            field: "contract_id".to_string(),
            value: "null".to_string(),
            details: "Missing or invalid contract_id field".to_string(),
        })?;
        Self::parse_hex(s)
    }

    fn parse_hex(s: &str) -> Result<Self, EldError> {
        let hex_str = s.strip_prefix("0x").unwrap_or(s);
        if hex_str.is_empty() {
            return Err(EldError::ValidationError {
                field: "contract ID".to_string(),
                value: s.to_string(),
                details: "Contract ID cannot be empty".to_string(),
            });
        }

        let decoded = hex::decode(hex_str).map_err(|_| EldError::ValidationError {
            field: "contract ID".to_string(),
            value: s.to_string(),
            details: "Contract ID must be valid hex (with optional 0x prefix)".to_string(),
        })?;

        if decoded.len() != Self::LEN {
            return Err(EldError::ValidationError {
                field: "contract ID".to_string(),
                value: s.to_string(),
                details: format!(
                    "Contract ID must be {} bytes ({} hex digits), got {} bytes",
                    Self::LEN,
                    Self::LEN * 2,
                    decoded.len()
                ),
            });
        }

        let mut bytes = [0u8; Self::LEN];
        bytes.copy_from_slice(&decoded);
        Ok(Self { bytes })
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.bytes
    }

    #[must_use]
    pub fn hex_with_prefix(&self) -> String {
        format!("0x{}", hex::encode(self.bytes))
    }

    #[must_use]
    pub fn hex_without_prefix(&self) -> String {
        hex::encode(self.bytes)
    }
}

impl FromStr for ContractId {
    type Err = EldError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_hex(s)
    }
}

impl fmt::Display for ContractId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.hex_with_prefix())
    }
}

impl fmt::Debug for ContractId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContractId({self})")
    }
}

impl Hash for ContractId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bytes.hash(state);
    }
}

impl Serialize for ContractId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.hex_with_prefix())
    }
}

impl<'de> Deserialize<'de> for ContractId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ContractIdVisitor;

        impl<'de> Visitor<'de> for ContractIdVisitor {
            type Value = ContractId;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a 32-byte hex contract ID (with or without 0x prefix)")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: SerdeError,
            {
                ContractId::parse(Some(value)).map_err(SerdeError::custom)
            }
        }

        deserializer.deserialize_str(ContractIdVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn parse_accepts_with_or_without_prefix() {
        let with_prefix = ContractId::parse(Some(SAMPLE)).expect("valid with prefix");
        let without_prefix = ContractId::parse(Some(&SAMPLE[2..])).expect("valid without prefix");
        assert_eq!(with_prefix, without_prefix);
        assert_eq!(with_prefix.to_string(), SAMPLE);
    }

    #[test]
    fn parse_rejects_wrong_length() {
        assert!(ContractId::parse(Some("0x00")).is_err());
    }

    #[test]
    fn parse_normalizes_uppercase() {
        let upper = "0x0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF";
        let id = ContractId::parse(Some(upper)).expect("valid uppercase");
        assert_eq!(id.to_string(), SAMPLE);
    }

    #[test]
    fn parse_rejects_missing_field() {
        assert!(ContractId::parse(None).is_err());
    }
}
