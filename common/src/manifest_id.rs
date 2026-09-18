//! Strongly typed content manifest identifier (32-byte SHA-256 digest over content id, metadata, chunks).
//!
//! See `TYPE_DESIGN.md` at the workspace root for ID representation conventions.

use crate::error::EldError;
use hex;
use serde::de::{Error as SerdeError, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

/// Manifest identifier: `0x` + 64 lowercase hex digits on the wire; 32 raw bytes inside.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ManifestId {
    bytes: [u8; Self::LEN],
}

impl ManifestId {
    /// Digest length in bytes.
    pub const LEN: usize = 32;

    /// Constructs a manifest id from raw digest bytes (trusted path, e.g. after `ContentManifest::calculate_id` hashing).
    #[must_use]
    pub fn new(bytes: [u8; Self::LEN]) -> Self {
        Self { bytes }
    }

    /// Parses `0x` followed by 64 hexadecimal digits (case-insensitive). Canonical [`fmt::Display`] output is lowercase.
    ///
    /// Prefer [`str::parse`] or [`FromStr::from_str`].
    ///
    /// # Errors
    ///
    /// Returns [`EldError::ValidationError`] if the string is not valid manifest-id hex.
    pub fn parse_hex(s: &str) -> Result<Self, EldError> {
        if !s.starts_with("0x") {
            return Err(EldError::ValidationError {
                field: "manifest ID".to_string(),
                value: s.to_string(),
                details: "Manifest ID must start with 0x".to_string(),
            });
        }
        let hex_str = &s[2..];
        if hex_str.is_empty() {
            return Err(EldError::ValidationError {
                field: "manifest ID".to_string(),
                value: s.to_string(),
                details: "Manifest ID cannot be empty after 0x prefix".to_string(),
            });
        }
        let decoded = hex::decode(hex_str).map_err(|_| EldError::ValidationError {
            field: "manifest ID".to_string(),
            value: s.to_string(),
            details: "Manifest ID must be valid hex after 0x prefix".to_string(),
        })?;
        if decoded.len() != Self::LEN {
            return Err(EldError::ValidationError {
                field: "manifest ID".to_string(),
                value: s.to_string(),
                details: format!(
                    "Manifest ID must be {} bytes ({} hex digits), got {} bytes",
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

    /// Raw digest bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.bytes
    }

    /// Canonical string: `0x` + lowercase hex (64 digits).
    #[must_use]
    pub fn hex_with_prefix(&self) -> String {
        format!("0x{}", hex::encode(self.bytes))
    }
}

impl FromStr for ManifestId {
    type Err = EldError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ManifestId::parse_hex(s)
    }
}

impl fmt::Display for ManifestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", hex::encode(self.bytes))
    }
}

impl fmt::Debug for ManifestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ManifestId({self})")
    }
}

impl Hash for ManifestId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bytes.hash(state);
    }
}

impl Serialize for ManifestId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.hex_with_prefix())
    }
}

impl<'de> Deserialize<'de> for ManifestId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ManifestIdVisitor;

        impl<'de> Visitor<'de> for ManifestIdVisitor {
            type Value = ManifestId;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a 0x-prefixed 64-digit hex manifest ID string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: SerdeError,
            {
                ManifestId::parse_hex(value).map_err(SerdeError::custom)
            }
        }

        deserializer.deserialize_str(ManifestIdVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "0xfedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

    #[test]
    fn from_str_accepts_uppercase_normalizes_display() {
        let upper = "0xFEDCBA9876543210FEDCBA9876543210FEDCBA9876543210FEDCBA9876543210";
        let id = upper.parse::<ManifestId>().expect("valid");
        assert_eq!(id.to_string(), SAMPLE);
    }

    #[test]
    fn from_str_rejects_wrong_length() {
        assert!("0x00".parse::<ManifestId>().is_err());
    }

    #[test]
    fn new_roundtrip_bytes() {
        let bytes = [9u8; 32];
        let id = ManifestId::new(bytes);
        assert_eq!(id.as_bytes(), &bytes);
    }

    #[test]
    fn serde_json_roundtrip() {
        let id: ManifestId = serde_json::from_str(&format!("\"{}\"", SAMPLE)).expect("de");
        assert_eq!(id.to_string(), SAMPLE);
        let json = serde_json::to_string(&id).expect("ser");
        assert_eq!(json, format!("\"{}\"", SAMPLE));
    }

    #[test]
    fn str_parse_uses_from_str_trait() {
        let id: ManifestId = SAMPLE.parse().expect("parse");
        assert_eq!(id.to_string(), SAMPLE);
    }
}
