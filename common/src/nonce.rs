//! Account nonce: sequential counter for replay protection.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Account nonce. Wraps a `u32`; use [`Nonce::new`](Self::new) with [`Nonce::ZERO`](Self::ZERO) for a zero nonce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Nonce(u32);

impl Nonce {
    /// Zero value for nonce (alias for `0u32`). Use `Nonce::new(Nonce::ZERO)` for the zero nonce.
    pub const ZERO: u32 = 0;

    /// Creates a nonce with the given value.
    #[inline]
    pub fn new(value: u32) -> Self {
        Nonce(value)
    }

    /// Returns the raw `u32` value.
    #[inline]
    pub fn value(&self) -> u32 {
        self.0
    }

    /// Returns the next nonce (value + 1). Use when building the next transaction.
    #[inline]
    pub fn next(&self) -> Self {
        Nonce(self.0 + 1)
    }

    /// Converts to the transaction wire format (`u32`).
    #[inline]
    pub fn to_tx_nonce(&self) -> u32 {
        self.0
    }
}

impl fmt::Display for Nonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u32> for Nonce {
    fn from(value: u32) -> Self {
        Nonce(value)
    }
}
