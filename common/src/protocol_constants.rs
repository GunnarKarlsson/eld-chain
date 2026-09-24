//! Chain parameters delivered in Tendermint genesis `app_state` and fixed for the life of the chain.

use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};

use crate::coin::Coin;
use crate::error::EldError;
use crate::fee::FeeConfig;

pub const PARAMS_VERSION: u32 = 1;

/// Fee schedule inside [`ProtocolConstants`]. Multipliers are basis points (10_000 = 1.0).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolFees {
    pub base_fee: Coin,
    pub size_fee_per_kb: Coin,
    pub gas_price: Coin,
    pub transfer_multiplier_bps: u32,
    pub stake_multiplier_bps: u32,
    pub content_manifest_multiplier_bps: u32,
    pub verified_proof_multiplier_bps: u32,
    pub device_operation_multiplier_bps: u32,
}

impl ProtocolFees {
    pub fn to_fee_config(&self) -> FeeConfig {
        FeeConfig {
            base_fee: self.base_fee.amount(),
            size_fee_per_kb: self.size_fee_per_kb.amount(),
            gas_price: self.gas_price.amount(),
            transfer_multiplier: bps_to_multiplier(self.transfer_multiplier_bps),
            stake_multiplier: bps_to_multiplier(self.stake_multiplier_bps),
            content_manifest_multiplier: bps_to_multiplier(self.content_manifest_multiplier_bps),
            verified_proof_multiplier: bps_to_multiplier(self.verified_proof_multiplier_bps),
            device_operation_multiplier: bps_to_multiplier(self.device_operation_multiplier_bps),
        }
    }
}

fn bps_to_multiplier(bps: u32) -> f64 {
    f64::from(bps) / 10_000.0
}

/// Immutable protocol parameters for one chain. Not part of app state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolConstants {
    pub params_version: u32,
    pub min_stake_amount: Coin,
    pub validators_per_epoch: usize,
    pub blocks_per_epoch: i64,
    pub active_storage_validators_per_epoch: usize,
    pub challenges_per_epoch: usize,
    pub chunks_per_challenge: usize,
    #[serde(with = "u64_json_string")]
    pub registration_duration_blocks: u64,
    pub verified_proof_reward_base_amount: Coin,
    pub failed_proofs_before_slash: u32,
    #[serde(with = "u64_json_string")]
    pub default_pinboard_post_ttl_blocks: u64,
    pub max_pinboard_message_bytes: usize,
    pub max_tx_bytes: usize,
    pub fees: ProtocolFees,
}

/// `{ "protocol_constants": ... }` object stored in genesis `app_state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenesisAppState {
    pub protocol_constants: ProtocolConstants,
}

impl ProtocolConstants {
    /// Local-dev numbers. Tests and pre-`InitChain` helpers use this. A running chain uses genesis.
    pub fn local_dev() -> Self {
        Self {
            params_version: PARAMS_VERSION,
            min_stake_amount: Coin::new(5).expect("local dev min stake"),
            validators_per_epoch: 4,
            blocks_per_epoch: 20,
            active_storage_validators_per_epoch: 1,
            challenges_per_epoch: 5,
            chunks_per_challenge: 10,
            registration_duration_blocks: 1_000_000,
            verified_proof_reward_base_amount: Coin::new(1000).expect("local dev reward"),
            failed_proofs_before_slash: 3,
            default_pinboard_post_ttl_blocks: 1000,
            max_pinboard_message_bytes: 1024 * 1024,
            max_tx_bytes: 10 * 1024 * 1024,
            fees: ProtocolFees {
                base_fee: Coin::new(1000).expect("local dev base fee"),
                size_fee_per_kb: Coin::new(100).expect("local dev size fee"),
                gas_price: Coin::new(1).expect("local dev gas price"),
                transfer_multiplier_bps: 10_000,
                stake_multiplier_bps: 15_000,
                content_manifest_multiplier_bps: 25_000,
                verified_proof_multiplier_bps: 20_000,
                device_operation_multiplier_bps: 18_000,
            },
        }
    }

    pub fn fee_config(&self) -> FeeConfig {
        self.fees.to_fee_config()
    }

    pub fn validate(&self) -> Result<(), EldError> {
        if self.params_version != PARAMS_VERSION {
            return EldError::validation_error(
                "params_version",
                &self.params_version.to_string(),
                &format!("unsupported protocol params version (want {PARAMS_VERSION})"),
            );
        }
        require_positive_u128("min_stake_amount", self.min_stake_amount.amount())?;
        require_positive_usize("validators_per_epoch", self.validators_per_epoch)?;
        if self.blocks_per_epoch <= 0 {
            return EldError::validation_error(
                "blocks_per_epoch",
                &self.blocks_per_epoch.to_string(),
                "blocks_per_epoch must be positive",
            );
        }
        require_positive_usize(
            "active_storage_validators_per_epoch",
            self.active_storage_validators_per_epoch,
        )?;
        require_positive_usize("challenges_per_epoch", self.challenges_per_epoch)?;
        require_positive_usize("chunks_per_challenge", self.chunks_per_challenge)?;
        if self.registration_duration_blocks == 0 {
            return EldError::validation_error(
                "registration_duration_blocks",
                "0",
                "registration_duration_blocks must be positive",
            );
        }
        require_positive_u128(
            "verified_proof_reward_base_amount",
            self.verified_proof_reward_base_amount.amount(),
        )?;
        if self.failed_proofs_before_slash == 0 {
            return EldError::validation_error(
                "failed_proofs_before_slash",
                "0",
                "failed_proofs_before_slash must be positive",
            );
        }
        if self.default_pinboard_post_ttl_blocks == 0 {
            return EldError::validation_error(
                "default_pinboard_post_ttl_blocks",
                "0",
                "default_pinboard_post_ttl_blocks must be positive",
            );
        }
        require_positive_usize(
            "max_pinboard_message_bytes",
            self.max_pinboard_message_bytes,
        )?;
        require_positive_usize("max_tx_bytes", self.max_tx_bytes)?;
        self.fee_config().validate()?;
        Ok(())
    }

    /// Parse CometBFT `RequestInitChain.app_state_bytes`.
    pub fn from_app_state_bytes(bytes: &[u8]) -> Result<Self, EldError> {
        if bytes.is_empty() {
            return Err(EldError::InitializationError {
                component: "protocol_constants".into(),
                details: "genesis app_state is empty".into(),
            });
        }
        let genesis: GenesisAppState =
            serde_json::from_slice(bytes).map_err(|e| EldError::InitializationError {
                component: "protocol_constants".into(),
                details: format!("failed to parse genesis app_state: {e}"),
            })?;
        genesis.protocol_constants.validate()?;
        Ok(genesis.protocol_constants)
    }
}

fn require_positive_u128(field: &str, value: u128) -> Result<(), EldError> {
    if value == 0 {
        EldError::validation_error(field, "0", &format!("{field} must be positive"))
    } else {
        Ok(())
    }
}

fn require_positive_usize(field: &str, value: usize) -> Result<(), EldError> {
    if value == 0 {
        EldError::validation_error(field, "0", &format!("{field} must be positive"))
    } else {
        Ok(())
    }
}

/// Shared handle set once from `InitChain` or from the restart key. No mutex.
#[derive(Clone, Debug)]
pub struct ProtocolHandle(Arc<OnceLock<Arc<ProtocolConstants>>>);

impl ProtocolHandle {
    pub fn new() -> Self {
        Self(Arc::new(OnceLock::new()))
    }

    pub fn installed_local_dev() -> Self {
        let handle = Self::new();
        handle
            .set(ProtocolConstants::local_dev())
            .expect("fresh handle");
        handle
    }

    pub fn set(&self, constants: ProtocolConstants) -> Result<(), Arc<ProtocolConstants>> {
        self.0.set(Arc::new(constants))
    }

    pub fn get(&self) -> Option<&ProtocolConstants> {
        self.0.get().map(Arc::as_ref)
    }
}

impl Default for ProtocolHandle {
    fn default() -> Self {
        Self::new()
    }
}

mod u64_json_string {
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        let raw = serde_json::Value::deserialize(deserializer)?;
        match raw {
            serde_json::Value::String(text) => text.parse().map_err(serde::de::Error::custom),
            serde_json::Value::Number(number) => number
                .as_u64()
                .ok_or_else(|| serde::de::Error::custom("expected u64")),
            _ => Err(serde::de::Error::custom("expected string or number")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_dev_round_trips_through_app_state_bytes() {
        let original = ProtocolConstants::local_dev();
        let bytes = serde_json::to_vec(&GenesisAppState {
            protocol_constants: original.clone(),
        })
        .expect("serialize");
        let parsed = ProtocolConstants::from_app_state_bytes(&bytes).expect("parse");
        assert_eq!(parsed, original);
        let fees = parsed.fee_config();
        assert_eq!(fees.base_fee, 1000);
        assert!((fees.stake_multiplier - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn rejects_unknown_params_version() {
        let mut constants = ProtocolConstants::local_dev();
        constants.params_version = 2;
        let bytes = serde_json::to_vec(&GenesisAppState {
            protocol_constants: constants,
        })
        .expect("serialize");
        assert!(ProtocolConstants::from_app_state_bytes(&bytes).is_err());
    }
}
