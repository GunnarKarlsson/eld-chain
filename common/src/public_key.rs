//! Canonical Ed25519 verifying key for on-chain capacity-validator registry entries.

use crate::error::EldError;
use crate::tx::TxPublicKey;
use ed25519_dalek::VerifyingKey;
use serde::de::{Error, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::hash::{Hash, Hasher};

/// 32-byte Ed25519 verifying key stored on capacity-validator registry entries.
///
/// There is no [`Default`]: the verifying key of the all-zero Ed25519 seed is a
/// valid key, so a default would look real and is a footgun. Construct from
/// validated bytes or an Ed25519 verifying key.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PublicKey {
    bytes: [u8; 32],
}

impl PublicKey {
    pub const LEN: usize = 32;

    /// Builds a public key from exactly 32 bytes, validating Ed25519 curve membership.
    pub fn new(bytes: [u8; 32]) -> Result<Self, EldError> {
        VerifyingKey::from_bytes(&bytes).map_err(|e| EldError::ValidationError {
            field: "public_key".to_string(),
            value: hex::encode(bytes),
            details: format!("invalid ed25519 public_key: {e}"),
        })?;
        Ok(Self { bytes })
    }

    /// Parses a hex-encoded Ed25519 public key (optional `0x` prefix).
    pub fn from_hex(hex_str: &str) -> Result<Self, EldError> {
        let cleaned = hex_str.strip_prefix("0x").unwrap_or(hex_str);
        let decoded = hex::decode(cleaned).map_err(|e| EldError::ValidationError {
            field: "public_key".to_string(),
            value: hex_str.to_string(),
            details: format!("invalid public_key hex: {e}"),
        })?;
        let array: [u8; 32] =
            decoded
                .as_slice()
                .try_into()
                .map_err(|_| EldError::ValidationError {
                    field: "public_key".to_string(),
                    value: hex_str.to_string(),
                    details: format!(
                        "invalid public_key length: {}, expected {}",
                        decoded.len(),
                        Self::LEN
                    ),
                })?;
        Self::new(array)
    }

    /// Raw verifying key bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }

    /// Returns the Ed25519 verifying key.
    pub fn to_verifying_key(&self) -> VerifyingKey {
        VerifyingKey::from_bytes(&self.bytes)
            .expect("PublicKey invariant: bytes are valid Ed25519 verifying key")
    }
}

impl Hash for PublicKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bytes.hash(state);
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({})", hex::encode(self.bytes))
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.bytes))
    }
}

impl Serialize for PublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(self.bytes))
    }
}

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PublicKeyVisitor;

        impl<'de> Visitor<'de> for PublicKeyVisitor {
            type Value = PublicKey;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a hex-encoded Ed25519 public key string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                PublicKey::from_hex(value).map_err(Error::custom)
            }
        }

        deserializer.deserialize_str(PublicKeyVisitor)
    }
}

impl From<VerifyingKey> for PublicKey {
    fn from(value: VerifyingKey) -> Self {
        Self {
            bytes: value.to_bytes(),
        }
    }
}

impl From<&VerifyingKey> for PublicKey {
    fn from(value: &VerifyingKey) -> Self {
        Self {
            bytes: value.to_bytes(),
        }
    }
}

impl TryFrom<TxPublicKey> for PublicKey {
    type Error = EldError;

    fn try_from(value: TxPublicKey) -> Result<Self, Self::Error> {
        PublicKey::try_from(&value)
    }
}

impl TryFrom<&TxPublicKey> for PublicKey {
    type Error = EldError;

    fn try_from(value: &TxPublicKey) -> Result<Self, Self::Error> {
        let verifying_key = value
            .to_verifying_key()
            .map_err(|e| EldError::ValidationError {
                field: "public_key".to_string(),
                value: value.as_str().to_string(),
                details: e,
            })?;
        Ok(PublicKey::from(verifying_key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    #[test]
    fn from_tx_public_key_matches_wallet_bytes() {
        let sk = signing_key(9);
        let tx_pk = TxPublicKey::from(sk.verifying_key());
        let registry_pk = PublicKey::try_from(&tx_pk).expect("convert");
        assert_eq!(registry_pk.as_bytes(), &sk.verifying_key().to_bytes());
    }

    #[test]
    fn serde_round_trip_hex_string() {
        let pk = PublicKey::from(signing_key(3).verifying_key());
        let json = serde_json::to_string(&pk).expect("serialize");
        assert_eq!(json, format!("\"{}\"", hex::encode(pk.as_bytes())));
        let restored: PublicKey = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(pk, restored);
    }
}
