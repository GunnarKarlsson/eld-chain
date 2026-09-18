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

    /// Returns the next nonce (`value + 1`), or `None` if this nonce is `u32::MAX`.
    ///
    /// Wrap-around would reuse spent nonces and is a consensus bug; callers must treat
    /// overflow as failure rather than wrapping.
    #[inline]
    pub fn next(&self) -> Option<Self> {
        self.0.checked_add(1).map(Nonce)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_increments() {
        assert_eq!(Nonce::new(Nonce::ZERO).next(), Some(Nonce::new(1)));
        assert_eq!(Nonce::new(41).next(), Some(Nonce::new(42)));
    }

    #[test]
    fn next_does_not_wrap_at_max() {
        assert_eq!(Nonce::new(u32::MAX).next(), None);
    }
}
