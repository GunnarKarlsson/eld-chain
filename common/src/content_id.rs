//! Strongly typed content identifier (32-byte SHA-256 digest of the chunk summary).
//!
//! See `TYPE_DESIGN.md` at the workspace root for ID representation conventions.

use crate::error::EldError;
use hex;
use serde::de::{Error as SerdeError, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

/// Content-addressed identifier: `0x` + 64 lowercase hex digits on the wire; 32 raw bytes inside.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ContentId {
    bytes: [u8; Self::LEN],
}

impl ContentId {
    /// Digest length in bytes.
    pub const LEN: usize = 32;

    /// Constructs a content id from raw digest bytes (trusted path, e.g. immediately after SHA-256).
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
    /// Returns [`EldError::ValidationError`] if the string is not valid content-id hex.
    pub fn parse_hex(s: &str) -> Result<Self, EldError> {
        if !s.starts_with("0x") {
            return Err(EldError::ValidationError {
                field: "content ID".to_string(),
                value: s.to_string(),
                details: "Content ID must start with 0x".to_string(),
            });
        }
        let hex_str = &s[2..];
        if hex_str.is_empty() {
            return Err(EldError::ValidationError {
                field: "content ID".to_string(),
                value: s.to_string(),
                details: "Content ID cannot be empty after 0x prefix".to_string(),
            });
        }
        let decoded = hex::decode(hex_str).map_err(|_| EldError::ValidationError {
            field: "content ID".to_string(),
            value: s.to_string(),
            details: "Content ID must be valid hex after 0x prefix".to_string(),
        })?;
        if decoded.len() != Self::LEN {
            return Err(EldError::ValidationError {
                field: "content ID".to_string(),
                value: s.to_string(),
                details: format!(
                    "Content ID must be {} bytes ({} hex digits), got {} bytes",
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

impl FromStr for ContentId {
    type Err = EldError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ContentId::parse_hex(s)
    }
}

impl fmt::Display for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", hex::encode(self.bytes))
    }
}

impl fmt::Debug for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentId({self})")
    }
}

impl Hash for ContentId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bytes.hash(state);
    }
}

impl Serialize for ContentId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.hex_with_prefix())
    }
}

impl<'de> Deserialize<'de> for ContentId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ContentIdVisitor;

        impl<'de> Visitor<'de> for ContentIdVisitor {
            type Value = ContentId;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a 0x-prefixed 64-digit hex content ID string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: SerdeError,
            {
                ContentId::parse_hex(value).map_err(SerdeError::custom)
            }
        }

        deserializer.deserialize_str(ContentIdVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn from_str_accepts_uppercase_normalizes_display() {
        let upper = "0x0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF";
        let id = upper.parse::<ContentId>().expect("valid");
        assert_eq!(id.to_string(), SAMPLE);
    }

    #[test]
    fn from_str_rejects_wrong_length() {
        assert!("0x00".parse::<ContentId>().is_err());
    }

    #[test]
    fn new_roundtrip_bytes() {
        let bytes = [7u8; 32];
        let id = ContentId::new(bytes);
        assert_eq!(id.as_bytes(), &bytes);
    }

    #[test]
    fn serde_json_roundtrip() {
        let id: ContentId = serde_json::from_str(&format!("\"{SAMPLE}\"")).expect("de");
        assert_eq!(id.to_string(), SAMPLE);
        let json = serde_json::to_string(&id).expect("ser");
        assert_eq!(json, format!("\"{SAMPLE}\""));
    }

    #[test]
    fn str_parse_uses_from_str_trait() {
        let id: ContentId = SAMPLE.parse().expect("parse");
        assert_eq!(id.to_string(), SAMPLE);
    }
}
