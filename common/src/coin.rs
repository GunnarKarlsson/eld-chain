//! #Value with associated properties (e.g. min/max bounds)
//! adapted from https://github.com/input-output-hk/rust-cardano (Cardano Rust)
//! Copyright (c) 2018, Input Output HK (licensed under the MIT License)
//! Modifications Copyright (c) 2018-2020 Crypto.com (licensed under the Apache License, Version 2.0)

use crate::constants::token::{MAX_COIN, MAX_COIN_DECIMALS};
use crate::error::EldError;
use parity_scale_codec::{Decode, Encode, EncodeLike, Error as ScaleError, Input, Output};

use serde::de::{Error, Visitor};
use serde::{Deserialize, Serialize, Serializer};

use std::prelude::v1::Vec;
use std::{fmt, ops, result};

/// represets the base unit amount bounded by the maximum / total supply
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Default, Hash)]
pub struct Coin(u128);

/// result type relating to `Coin` operations
pub type CoinResult = Result<Coin, EldError>;

impl Serialize for Coin {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let coin_string = self.0.to_string();
        serializer.serialize_str(&coin_string[..])
    }
}

impl<'de> Deserialize<'de> for Coin {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct StrVisitor;

        impl<'de> Visitor<'de> for StrVisitor {
            type Value = Coin;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("the coin amount in a range (0..total supply")
            }

            #[inline]
            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                let amount = value
                    .parse::<u128>()
                    .map_err(|e| E::custom(format!("{e}")))?;
                Coin::new(amount).map_err(|e| E::custom(format!("{e}")))
            }
        }

        deserializer.deserialize_str(StrVisitor)
    }
}

impl Coin {
    /// Create a coin with the maximum allowed value
    pub fn max() -> Self {
        Coin(MAX_COIN)
    }

    /// Create a coin with zero value
    pub fn zero() -> Self {
        Coin(0)
    }

    /// Create a coin of value `1`
    pub fn unit() -> Self {
        Coin(1)
    }

    /// Create a non-base coin of value 1 (assuming 8 decimals)
    pub fn one() -> Self {
        Coin(MAX_COIN_DECIMALS)
    }

    /// Check if the coin has zero value
    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }

    /// Get the amount as a u128
    pub fn amount(&self) -> u128 {
        self.0
    }

    /// Checked addition that returns None on overflow or invalid result.
    pub fn checked_add(self, other: Coin) -> Option<Coin> {
        self.0.checked_add(other.0).and_then(|v| Coin::new(v).ok())
    }

    /// Calculate the ratio of this Coin to another Coin as f64
    /// Useful for percentage calculations
    /// Returns an error if dividing by zero
    pub fn ratio(&self, other: Coin) -> Result<f64, EldError> {
        if other.0 == 0 {
            Err(EldError::CoinError {
                details: "Coin divide by zero".to_string(),
            })
        } else {
            Ok(self.0 as f64 / other.0 as f64)
        }
    }

    /// Convert to a string representation with proper decimal formatting
    pub fn to_decimal_string(&self) -> String {
        let amount = self.0;
        let whole = amount / MAX_COIN_DECIMALS;
        let decimal = amount % MAX_COIN_DECIMALS;

        if decimal == 0 {
            whole.to_string()
        } else {
            format!("{whole}.{decimal:0>6}")
        }
    }

    /// Parse a string representation back to a coin
    pub fn from_string(s: &str) -> Result<Self, EldError> {
        let parts: Vec<&str> = s.split('.').collect();
        match parts.as_slice() {
            [whole] => {
                let whole_val: u128 = whole.parse().map_err(|_| EldError::CoinError {
                    details: format!("Invalid whole number: {whole}"),
                })?;
                Coin::new(whole_val * MAX_COIN_DECIMALS)
            }
            [whole, decimal] => {
                let whole_val: u128 = whole.parse().map_err(|_| EldError::CoinError {
                    details: format!("Invalid whole number: {whole}"),
                })?;
                let decimal_str = if decimal.len() > 9 {
                    &decimal[..9]
                } else {
                    decimal
                };
                let decimal_val: u128 = decimal_str.parse().map_err(|_| EldError::CoinError {
                    details: format!("Invalid decimal number: {decimal_str}"),
                })?;
                let multiplier = 10u128.pow(9 - decimal_str.len() as u32);
                Coin::new(whole_val * MAX_COIN_DECIMALS + decimal_val * multiplier)
            }
            _ => Err(EldError::CoinError {
                details: format!("Invalid coin format: {s}"),
            }),
        }
    }

    /// create a coin of the given value
    pub fn new(amount: u128) -> Result<Self, EldError> {
        if amount > MAX_COIN {
            Err(EldError::CoinError {
                details: format!("Amount {amount} exceeds maximum coin value {MAX_COIN}"),
            })
        } else {
            Ok(Coin(amount))
        }
    }
}

impl fmt::Display for Coin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 6 decimals (matching MAX_COIN_DECIMALS = 1_000_000)
        write!(
            f,
            "{}.{:06}",
            self.0 / MAX_COIN_DECIMALS,
            self.0 % MAX_COIN_DECIMALS
        )
    }
}

impl ::std::str::FromStr for Coin {
    type Err = EldError;
    fn from_str(s: &str) -> result::Result<Self, Self::Err> {
        let v: u128 = match s.parse() {
            Err(_) => {
                return Err(EldError::CoinError {
                    details: "Cannot parse a valid integer".to_string(),
                })
            }
            Ok(v) => v,
        };
        Coin::new(v)
    }
}

impl ops::Add for Coin {
    type Output = CoinResult;
    fn add(self, other: Coin) -> Self::Output {
        let sum = self.0.checked_add(other.0);
        match sum {
            None => Err(EldError::CoinError {
                details: "Integer overflow during coin addition".to_string(),
            }),
            Some(v) => Coin::new(v),
        }
    }
}

impl<'a> ops::Add<&'a Coin> for Coin {
    type Output = CoinResult;
    fn add(self, other: &'a Coin) -> Self::Output {
        let sum = self.0.checked_add(other.0);
        match sum {
            None => Err(EldError::CoinError {
                details: "Integer overflow during coin addition".to_string(),
            }),
            Some(v) => Coin::new(v),
        }
    }
}

impl ops::Sub for Coin {
    type Output = CoinResult;
    fn sub(self, other: Coin) -> Self::Output {
        let sub = self.0.checked_sub(other.0);
        match sub {
            None => Err(EldError::CoinError {
                details: "Coin cannot hold a negative value".to_string(),
            }),
            Some(v) => Coin::new(v),
        }
    }
}
impl<'a> ops::Sub<&'a Coin> for Coin {
    type Output = CoinResult;
    fn sub(self, other: &'a Coin) -> Self::Output {
        let sub = self.0.checked_sub(other.0);
        match sub {
            None => Err(EldError::CoinError {
                details: "Coin cannot hold a negative value".to_string(),
            }),
            Some(v) => Coin::new(v),
        }
    }
}

// i.e. `coin1 - coin2 - coin3`
impl ops::Sub<Coin> for CoinResult {
    type Output = CoinResult;
    fn sub(self, other: Coin) -> Self::Output {
        let coin = self?;
        if other.0 > coin.0 {
            Err(EldError::CoinError {
                details: "Coin cannot hold a negative value".to_string(),
            })
        } else {
            Ok(Coin(coin.0 - other.0))
        }
    }
}

impl ops::Mul<u128> for Coin {
    type Output = CoinResult;
    fn mul(self, mul: u128) -> CoinResult {
        self.0
            .checked_mul(mul)
            .ok_or(EldError::CoinError {
                details: "Integer overflow during coin multiplication".to_string(),
            })
            .and_then(Coin::new)
    }
}

impl ops::Mul<Coin> for Coin {
    type Output = CoinResult;
    fn mul(self, other: Coin) -> Self::Output {
        self.0
            .checked_mul(other.0)
            .ok_or(EldError::CoinError {
                details: "Integer overflow during coin multiplication".to_string(),
            })
            .and_then(Coin::new)
    }
}

impl ops::Div<u128> for Coin {
    type Output = CoinResult;
    fn div(self, modulus: u128) -> CoinResult {
        self.0
            .checked_div(modulus)
            .ok_or(EldError::CoinError {
                details: "Coin divide by zero".to_string(),
            })
            .map(Coin)
    }
}

impl ops::Div<Coin> for Coin {
    type Output = CoinResult;
    /// Divide two Coins, returning integer division result as Coin
    /// Returns an error if dividing by zero
    fn div(self, other: Coin) -> Self::Output {
        if other.0 == 0 {
            Err(EldError::CoinError {
                details: "Coin divide by zero".to_string(),
            })
        } else {
            Ok(Coin(self.0 / other.0))
        }
    }
}

impl ops::Rem<u128> for Coin {
    type Output = CoinResult;
    fn rem(self, modulus: u128) -> CoinResult {
        self.0
            .checked_rem(modulus)
            .ok_or(EldError::CoinError {
                details: "Coin divide by zero".to_string(),
            })
            .map(Coin)
    }
}

impl From<Coin> for u128 {
    fn from(c: Coin) -> u128 {
        c.0
    }
}

impl Decode for Coin {
    fn decode<I: Input>(input: &mut I) -> Result<Self, ScaleError> {
        let num = u128::decode(input)?;

        if num > MAX_COIN {
            Err(ScaleError::from("Value greater than maximum allowed"))
        } else {
            Ok(Coin(num))
        }
    }
}

impl Encode for Coin {
    fn encode_to<EncOut: Output>(&self, dest: &mut EncOut) {
        self.0.encode_to(dest)
    }
    fn encode(&self) -> Vec<u8> {
        self.0.encode()
    }
    fn using_encoded<R, F: FnOnce(&[u8]) -> R>(&self, f: F) -> R {
        self.0.using_encoded(f)
    }
    fn size_hint(&self) -> usize {
        self.0.size_hint()
    }
}
impl EncodeLike<u128> for Coin {}

/// helper for summing coins in some iterable structure
pub fn sum_coins(mut coins: impl Iterator<Item = Coin>) -> Result<Coin, EldError> {
    coins.try_fold(Coin::zero(), |acc, coin| acc + coin)
}
