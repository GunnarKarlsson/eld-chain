use crate::coin::Coin;
use crate::error::EldError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Transaction-boundary amount wrapper.
///
/// We intentionally keep tx payload fields as `TxAmount` instead of `Coin`:
/// - `TxAmount` represents wire payload intent only and keeps tx schema stable.
/// - `Coin` is a richer state/economics type with validation and arithmetic semantics.
/// - Conversion to `Coin` is done during tx validation/processing.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TxAmount(pub u128);

impl TxAmount {
    pub fn as_u128(self) -> u128 {
        self.0
    }

    pub fn checked_add(self, rhs: u128) -> Option<Self> {
        self.0.checked_add(rhs).map(Self)
    }

    pub fn saturating_sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }

    pub fn to_coin(self) -> Result<Coin, EldError> {
        Coin::new(self.0)
    }
}

impl std::fmt::Display for TxAmount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u128> for TxAmount {
    fn from(value: u128) -> Self {
        Self(value)
    }
}

impl From<TxAmount> for u128 {
    fn from(value: TxAmount) -> Self {
        value.0
    }
}

impl From<Coin> for TxAmount {
    fn from(value: Coin) -> Self {
        Self(value.amount())
    }
}

impl PartialEq<u128> for TxAmount {
    fn eq(&self, other: &u128) -> bool {
        self.0 == *other
    }
}

impl Serialize for TxAmount {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u128(self.0)
    }
}

impl<'de> Deserialize<'de> for TxAmount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self(u128::deserialize(deserializer)?))
    }
}

/// Serialization helper for TxAmount as string in JSON (for cross-language compatibility)
pub(crate) mod tx_amount_string {
    use super::*;

    pub(crate) fn serialize<S>(value: &TxAmount, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.0.to_string())
    }

    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<TxAmount, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let value = s.parse::<u128>().map_err(serde::de::Error::custom)?;
        Ok(TxAmount(value))
    }
}
