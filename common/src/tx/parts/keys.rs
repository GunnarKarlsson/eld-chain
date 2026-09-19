use crate::error::EldError;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TxPublicKey(String);

impl TxPublicKey {
    pub const LEN: usize = 32;

    pub fn new(value: String) -> Result<Self, EldError> {
        let bytes = hex::decode(&value).map_err(|e| {
            EldError::make_validation_error(
                "public_key",
                &value,
                format!("invalid public_key hex: {e}"),
            )
        })?;
        if bytes.len() != Self::LEN {
            return Err(EldError::make_validation_error(
                "public_key",
                &value,
                format!(
                    "public_key must be {} bytes, got {} bytes",
                    Self::LEN,
                    bytes.len()
                ),
            ));
        }
        let array: [u8; Self::LEN] = bytes.try_into().map_err(|_| {
            EldError::make_validation_error(
                "public_key",
                &value,
                format!("public_key must be {} bytes", Self::LEN),
            )
        })?;
        VerifyingKey::from_bytes(&array).map_err(|e| {
            EldError::make_validation_error(
                "public_key",
                &value,
                format!("invalid ed25519 public_key: {e}"),
            )
        })?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn to_verifying_key(&self) -> Result<VerifyingKey, EldError> {
        let bytes = hex::decode(&self.0).map_err(|e| {
            EldError::make_validation_error(
                "public_key",
                &self.0,
                format!("invalid public_key hex: {e}"),
            )
        })?;
        let array: [u8; Self::LEN] = bytes.try_into().map_err(|_| {
            EldError::make_validation_error(
                "public_key",
                &self.0,
                format!("public_key must be {} bytes", Self::LEN),
            )
        })?;
        VerifyingKey::from_bytes(&array).map_err(|e| {
            EldError::make_validation_error(
                "public_key",
                &self.0,
                format!("invalid ed25519 public_key: {e}"),
            )
        })
    }
}

impl std::fmt::Display for TxPublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Serialize for TxPublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TxPublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        TxPublicKey::new(value).map_err(serde::de::Error::custom)
    }
}

impl From<VerifyingKey> for TxPublicKey {
    fn from(value: VerifyingKey) -> Self {
        Self(hex::encode(value.to_bytes()))
    }
}

impl From<&VerifyingKey> for TxPublicKey {
    fn from(value: &VerifyingKey) -> Self {
        Self(hex::encode(value.to_bytes()))
    }
}

impl From<TxPublicKey> for String {
    fn from(value: TxPublicKey) -> Self {
        value.0
    }
}

impl TryFrom<String> for TxPublicKey {
    type Error = EldError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for TxPublicKey {
    type Error = EldError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct TxSig(String);

impl TxSig {
    pub const LEN: usize = 64;

    pub fn new(value: String) -> Result<Self, EldError> {
        let bytes = hex::decode(&value).map_err(|e| {
            EldError::make_validation_error("sig", &value, format!("invalid signature hex: {e}"))
        })?;
        if bytes.len() != Self::LEN {
            return Err(EldError::make_validation_error(
                "sig",
                &value,
                format!(
                    "signature must be {} bytes, got {} bytes",
                    Self::LEN,
                    bytes.len()
                ),
            ));
        }
        let array: [u8; Self::LEN] = bytes.try_into().map_err(|_| {
            EldError::make_validation_error(
                "sig",
                &value,
                format!("signature must be {} bytes", Self::LEN),
            )
        })?;
        let _ = Signature::from_bytes(&array);
        Ok(Self(value))
    }

    pub fn empty() -> Self {
        Self(String::new())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn to_signature(&self) -> Result<Signature, EldError> {
        let bytes = hex::decode(&self.0).map_err(|e| {
            EldError::make_validation_error("sig", &self.0, format!("invalid signature hex: {e}"))
        })?;
        let array: [u8; Self::LEN] = bytes.try_into().map_err(|_| {
            EldError::make_validation_error(
                "sig",
                &self.0,
                format!("signature must be {} bytes", Self::LEN),
            )
        })?;
        Ok(Signature::from_bytes(&array))
    }
}

impl std::fmt::Display for TxSig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Serialize for TxSig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TxSig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.is_empty() {
            return Ok(TxSig::empty());
        }
        TxSig::new(value).map_err(serde::de::Error::custom)
    }
}

impl From<Signature> for TxSig {
    fn from(value: Signature) -> Self {
        Self(hex::encode(value.to_bytes()))
    }
}

impl From<TxSig> for String {
    fn from(value: TxSig) -> Self {
        value.0
    }
}

impl TryFrom<String> for TxSig {
    type Error = EldError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() {
            return Ok(Self::empty());
        }
        Self::new(value)
    }
}

impl TryFrom<&str> for TxSig {
    type Error = EldError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_string())
    }
}
