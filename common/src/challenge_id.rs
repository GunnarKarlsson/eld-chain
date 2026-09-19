//! Strongly typed capacity challenge identifier (32-byte SHA-256 digest).
//!
//! For internal domain use. P2P, tx, DB, and API edges keep the existing 64-char hex
//! `String` (no `0x`); convert with [`ChallengeId::new`] / [`ChallengeId::to_hex`] /
//! [`ChallengeId::parse_hex`] at those boundaries.
//!
//! See `TYPE_DESIGN.md` for ID representation and edge-stability conventions.

use crate::error::EldError;
use crate::hex_encoding::decode_fixed_hex;
use hex;
use serde::de::{Error as SerdeError, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

/// Capacity challenge id: 32 raw bytes inside; hex text for helpers/logs and wire edges.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChallengeId {
    bytes: [u8; Self::LEN],
}

impl ChallengeId {
    /// Digest length in bytes.
    pub const LEN: usize = 32;

    /// Constructs from raw digest bytes (trusted path, e.g. immediately after SHA-256).
    #[must_use]
    pub fn new(bytes: [u8; Self::LEN]) -> Self {
        Self { bytes }
    }

    /// Parses 64 hexadecimal digits (case-insensitive). Optional `0x` / `0X` prefix is accepted.
    ///
    /// Canonical [`fmt::Display`] / [`Self::to_hex`] output is lowercase hex **without** `0x`,
    /// matching `VerifiedProofTx` / SyncMsg challenge id encoding.
    ///
    /// # Errors
    ///
    /// Returns [`EldError::ValidationError`] if the string is not valid 32-byte hex.
    pub fn parse_hex(s: &str) -> Result<Self, EldError> {
        let bytes = decode_fixed_hex::<{ Self::LEN }>(s, "challenge ID")?;
        Ok(Self { bytes })
    }

    /// Raw digest bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.bytes
    }

    /// Lowercase hex without `0x` (64 digits), matching challenge id edge string form.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.bytes)
    }
}

impl FromStr for ChallengeId {
    type Err = EldError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ChallengeId::parse_hex(s)
    }
}

impl fmt::Display for ChallengeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl fmt::Debug for ChallengeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ChallengeId({self})")
    }
}

impl Hash for ChallengeId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bytes.hash(state);
    }
}

impl Serialize for ChallengeId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for ChallengeId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ChallengeIdVisitor;

        impl<'de> Visitor<'de> for ChallengeIdVisitor {
            type Value = ChallengeId;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a 64-digit hex challenge ID string (optional 0x prefix)")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: SerdeError,
            {
                ChallengeId::parse_hex(value).map_err(SerdeError::custom)
            }
        }

        deserializer.deserialize_str(ChallengeIdVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn parse_hex_accepts_uppercase_and_optional_0x() {
        let upper = "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF";
        let id = ChallengeId::parse_hex(upper).expect("valid");
        assert_eq!(id.to_hex(), SAMPLE);
        let prefixed = format!("0x{upper}");
        let id2 = ChallengeId::parse_hex(&prefixed).expect("valid with 0x");
        assert_eq!(id2, id);
        let id3 = ChallengeId::parse_hex(&format!("0X{upper}")).expect("valid with 0X");
        assert_eq!(id3, id);
        let json = serde_json::to_string(&id2).expect("ser");
        assert_eq!(json, format!("\"{SAMPLE}\""));
    }

    #[test]
    fn parse_hex_rejects_wrong_length() {
        assert!(ChallengeId::parse_hex("00").is_err());
        assert!(ChallengeId::parse_hex("0x00").is_err());
    }

    #[test]
    fn new_roundtrip_bytes() {
        let bytes = [7u8; 32];
        let id = ChallengeId::new(bytes);
        assert_eq!(id.as_bytes(), &bytes);
    }

    #[test]
    fn serde_json_roundtrip() {
        let id: ChallengeId = serde_json::from_str(&format!("\"{SAMPLE}\"")).expect("de");
        assert_eq!(id.to_hex(), SAMPLE);
        let json = serde_json::to_string(&id).expect("ser");
        assert_eq!(json, format!("\"{SAMPLE}\""));
    }

    #[test]
    fn from_str_trait() {
        let id: ChallengeId = SAMPLE.parse().expect("parse");
        assert_eq!(id.to_string(), SAMPLE);
    }
}
