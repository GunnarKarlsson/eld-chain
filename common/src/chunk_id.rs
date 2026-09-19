//! Strongly typed chunk identifier (32-byte SHA-256 digest of chunk bytes).
//!
//! See `TYPE_DESIGN.md` at the workspace root for ID representation conventions.

use crate::error::EldError;
use crate::hex_encoding::decode_fixed_hex;
use hex;
use serde::de::{Error as SerdeError, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

/// Chunk content-addressed id: 32 raw bytes inside.
///
/// Parsing accepts an optional `0x` / `0X` prefix. Canonical [`fmt::Display`] / serde output is
/// `0x` + 64 lowercase hex digits so existing clients keep the same wire form.
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

    /// Parses 64 hexadecimal digits (case-insensitive). Optional `0x` / `0X` prefix is accepted.
    ///
    /// Canonical [`fmt::Display`] output is `0x` + lowercase.
    /// Prefer [`str::parse`] or [`FromStr::from_str`].
    ///
    /// # Errors
    ///
    /// Returns [`EldError::ValidationError`] if the string is not valid chunk-id hex.
    pub fn parse_hex(s: &str) -> Result<Self, EldError> {
        let bytes = decode_fixed_hex::<{ Self::LEN }>(s, "chunk ID")?;
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
                formatter.write_str("a 64-digit hex chunk ID string (optional 0x prefix)")
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
        assert!(SAMPLE[2..].parse::<ChunkId>().is_ok());
    }

    #[test]
    fn parse_hex_accepts_optional_prefix_canonical_display_keeps_0x() {
        let bare = &SAMPLE[2..];
        let from_bare: ChunkId = bare.parse().expect("bare hex");
        let from_0x: ChunkId = SAMPLE.parse().expect("0x hex");
        let from_0x_upper = format!("0X{}", bare.to_ascii_uppercase())
            .parse::<ChunkId>()
            .expect("0X hex");
        assert_eq!(from_bare, from_0x);
        assert_eq!(from_bare, from_0x_upper);
        assert_eq!(from_bare.to_string(), SAMPLE);
        let json = serde_json::to_string(&from_bare).expect("ser");
        assert_eq!(json, format!("\"{SAMPLE}\""));
        let from_json: ChunkId =
            serde_json::from_str(&format!("\"{bare}\"")).expect("de without prefix");
        assert_eq!(from_json, from_bare);
    }

    #[test]
    fn serde_json_roundtrip() {
        let id: ChunkId = serde_json::from_str(&format!("\"{SAMPLE}\"")).expect("de");
        assert_eq!(id.to_string(), SAMPLE);
    }
}
