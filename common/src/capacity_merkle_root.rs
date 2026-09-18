//! Strongly typed capacity proof merkle root (32-byte digest).
//!
//! For internal domain use. P2P, tx, DB, and API edges keep `[u8; 32]` / hex as today;
//! convert with [`CapacityMerkleRoot::new`] / [`CapacityMerkleRoot::as_bytes`] at those boundaries.
//!
//! See `TYPE_DESIGN.md` for ID representation and edge-stability conventions.

use crate::error::EldError;
use hex;
use serde::de::{Error as SerdeError, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

/// Merkle root of a capacity proof tree: 32 raw bytes inside; hex text for helpers/logs.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CapacityMerkleRoot {
    bytes: [u8; Self::LEN],
}

impl CapacityMerkleRoot {
    /// Digest length in bytes.
    pub const LEN: usize = 32;

    /// Constructs from raw digest bytes (trusted path, e.g. after merkle computation or DB read).
    #[must_use]
    pub fn new(bytes: [u8; Self::LEN]) -> Self {
        Self { bytes }
    }

    /// Parses 64 hexadecimal digits (case-insensitive). Optional `0x` prefix is accepted.
    ///
    /// Canonical [`fmt::Display`] / [`Self::to_hex`] output is lowercase hex **without** `0x`,
    /// matching capacity tx / validator edge encoding.
    ///
    /// # Errors
    ///
    /// Returns [`EldError::ValidationError`] if the string is not valid 32-byte hex.
    pub fn parse_hex(s: &str) -> Result<Self, EldError> {
        let cleaned = s.strip_prefix("0x").unwrap_or(s);
        if cleaned.is_empty() {
            return Err(EldError::ValidationError {
                field: "capacity merkle root".to_string(),
                value: s.to_string(),
                details: "capacity merkle root hex cannot be empty".to_string(),
            });
        }
        let decoded = hex::decode(cleaned).map_err(|e| EldError::ValidationError {
            field: "capacity merkle root".to_string(),
            value: s.to_string(),
            details: format!("invalid capacity merkle root hex: {e}"),
        })?;
        if decoded.len() != Self::LEN {
            return Err(EldError::ValidationError {
                field: "capacity merkle root".to_string(),
                value: s.to_string(),
                details: format!(
                    "capacity merkle root must be {} bytes ({} hex digits), got {} bytes",
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

    /// Raw digest bytes (e.g. before writing to tx / DB / P2P edges).
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.bytes
    }

    /// Lowercase hex without `0x` (64 digits), matching capacity edge string form.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.bytes)
    }
}

impl FromStr for CapacityMerkleRoot {
    type Err = EldError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        CapacityMerkleRoot::parse_hex(s)
    }
}

impl fmt::Display for CapacityMerkleRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl fmt::Debug for CapacityMerkleRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CapacityMerkleRoot({self})")
    }
}

impl Hash for CapacityMerkleRoot {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bytes.hash(state);
    }
}

impl Serialize for CapacityMerkleRoot {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for CapacityMerkleRoot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CapacityMerkleRootVisitor;

        impl<'de> Visitor<'de> for CapacityMerkleRootVisitor {
            type Value = CapacityMerkleRoot;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a 64-digit hex capacity merkle root string (optional 0x prefix)")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: SerdeError,
            {
                CapacityMerkleRoot::parse_hex(value).map_err(SerdeError::custom)
            }
        }

        deserializer.deserialize_str(CapacityMerkleRootVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn parse_hex_accepts_uppercase_and_optional_0x() {
        let upper = "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF";
        let root = CapacityMerkleRoot::parse_hex(upper).expect("valid");
        assert_eq!(root.to_hex(), SAMPLE);
        let prefixed = format!("0x{upper}");
        let root2 = CapacityMerkleRoot::parse_hex(&prefixed).expect("valid with 0x");
        assert_eq!(root2, root);
    }

    #[test]
    fn parse_hex_rejects_wrong_length() {
        assert!(CapacityMerkleRoot::parse_hex("00").is_err());
        assert!(CapacityMerkleRoot::parse_hex("0x00").is_err());
    }

    #[test]
    fn new_roundtrip_bytes() {
        let bytes = [7u8; 32];
        let root = CapacityMerkleRoot::new(bytes);
        assert_eq!(root.as_bytes(), &bytes);
    }

    #[test]
    fn serde_json_roundtrip() {
        let root: CapacityMerkleRoot =
            serde_json::from_str(&format!("\"{SAMPLE}\"")).expect("de");
        assert_eq!(root.to_hex(), SAMPLE);
        let json = serde_json::to_string(&root).expect("ser");
        assert_eq!(json, format!("\"{SAMPLE}\""));
    }

    #[test]
    fn from_str_trait() {
        let root: CapacityMerkleRoot = SAMPLE.parse().expect("parse");
        assert_eq!(root.to_string(), SAMPLE);
    }
}
