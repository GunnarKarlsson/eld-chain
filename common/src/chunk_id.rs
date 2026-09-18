//! Strongly typed chunk identifier (32-byte SHA-256 digest of chunk bytes).
//!
//! See `TYPE_DESIGN.md` at the workspace root for ID representation conventions.

use crate::error::EldError;
use hex;
use serde::de::{Error as SerdeError, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

/// Chunk content-addressed id: `0x` + 64 lowercase hex digits on the wire; 32 raw bytes inside.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChunkId {
    bytes: [u8; Self::LEN],
}

impl ChunkId {
    /// Digest length in bytes.
    pub const LEN: usize = 32;

    /// Trusted construction from raw digest bytes (e.g. immediately after SHA-256 over chunk data).
    #[must_use]
    pub fn new(bytes: [u8; Self::LEN]) -> Self {
        Self { bytes }
    }

    /// Parses `0x` + 64 hex digits (case-insensitive). [`fmt::Display`] is canonical lowercase.
    ///
    /// Prefer [`str::parse`] or [`FromStr::from_str`].
    ///
    /// # Errors
    ///
    /// Returns [`EldError::ValidationError`] if the string is not valid chunk-id hex.
    pub fn parse_hex(s: &str) -> Result<Self, EldError> {
        if !s.starts_with("0x") {
            return Err(EldError::ValidationError {
                field: "chunk ID".to_string(),
                value: s.to_string(),
                details: "Chunk ID must start with 0x".to_string(),
            });
        }
        let hex_str = &s[2..];
        if hex_str.is_empty() {
            return Err(EldError::ValidationError {
                field: "chunk ID".to_string(),
                value: s.to_string(),
                details: "Chunk ID cannot be empty after 0x prefix".to_string(),
            });
        }
        let decoded = hex::decode(hex_str).map_err(|_| EldError::ValidationError {
            field: "chunk ID".to_string(),
            value: s.to_string(),
            details: "Chunk ID must be valid hex after 0x prefix".to_string(),
        })?;
        if decoded.len() != Self::LEN {
            return Err(EldError::ValidationError {
                field: "chunk ID".to_string(),
                value: s.to_string(),
                details: format!(
                    "Chunk ID must be {} bytes ({} hex digits), got {} bytes",
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

impl FromStr for ChunkId {
    type Err = EldError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ChunkId::parse_hex(s)
    }
}

impl fmt::Display for ChunkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", hex::encode(self.bytes))
    }
}

impl fmt::Debug for ChunkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ChunkId({self})")
    }
}

impl Hash for ChunkId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bytes.hash(state);
    }
}

impl Serialize for ChunkId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.hex_with_prefix())
    }
}

impl<'de> Deserialize<'de> for ChunkId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ChunkIdVisitor;

        impl<'de> Visitor<'de> for ChunkIdVisitor {
            type Value = ChunkId;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a 0x-prefixed 64-digit hex chunk ID string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: SerdeError,
            {
                ChunkId::parse_hex(value).map_err(SerdeError::custom)
            }
        }

        deserializer.deserialize_str(ChunkIdVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn from_str_accepts_uppercase_normalizes_display() {
        let upper = "0x0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF";
        let id = upper.parse::<ChunkId>().expect("valid");
        assert_eq!(id.to_string(), SAMPLE);
    }

    #[test]
    fn from_str_rejects_wrong_length() {
        assert!("0x00".parse::<ChunkId>().is_err());
    }

    #[test]
    fn serde_json_roundtrip() {
        let id: ChunkId = serde_json::from_str(&format!("\"{}\"", SAMPLE)).expect("de");
        assert_eq!(id.to_string(), SAMPLE);
    }
}
