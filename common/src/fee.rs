use crate::coin::Coin;
use crate::error::EldError;
use crate::tx::{PayloadInner, Tx};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Configuration for fee calculations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeConfig {
    /// Base fee for any transaction (minimum fee)
    pub base_fee: u128,
    /// Fee per KB of transaction data
    pub size_fee_per_kb: u128,
    /// Gas price in fee units
    pub gas_price: u128,
    /// Complexity multipliers for different transaction types
    pub transfer_multiplier: f64,
    pub stake_multiplier: f64,
    pub content_manifest_multiplier: f64,
    /// Fee multiplier for [`PayloadInner::VerifiedProof`](crate::tx::PayloadInner::VerifiedProof).
    #[serde(alias = "chunk_proof_multiplier")]
    pub verified_proof_multiplier: f64,
    pub device_operation_multiplier: f64,
}

impl Default for FeeConfig {
    fn default() -> Self {
        Self {
            base_fee: 1000,
            size_fee_per_kb: 100,
            gas_price: 1,
            transfer_multiplier: 1.0,
            stake_multiplier: 1.5,
            content_manifest_multiplier: 2.5,
            verified_proof_multiplier: 2.0,
            device_operation_multiplier: 1.8,
        }
    }
}

impl FeeConfig {
    /// Validates fee configuration parameters
    pub fn validate(&self) -> Result<(), EldError> {
        if self.base_fee == 0 {
            return EldError::validation_error(
                "base_fee",
                &self.base_fee.to_string(),
                "Base fee cannot be zero",
            );
        }
        if self.size_fee_per_kb == 0 {
            return EldError::validation_error(
                "size_fee_per_kb",
                &self.size_fee_per_kb.to_string(),
                "Size fee per KB cannot be zero",
            );
        }
        if self.gas_price == 0 {
            return EldError::validation_error(
                "gas_price",
                &self.gas_price.to_string(),
                "Gas price cannot be zero",
            );
        }
        if self.transfer_multiplier <= 0.0 {
            return EldError::validation_error(
                "transfer_multiplier",
                &self.transfer_multiplier.to_string(),
                "Transfer multiplier must be positive",
            );
        }
        if self.stake_multiplier <= 0.0 {
            return EldError::validation_error(
                "stake_multiplier",
                &self.stake_multiplier.to_string(),
                "Stake multiplier must be positive",
            );
        }
        if self.content_manifest_multiplier <= 0.0 {
            return EldError::validation_error(
                "content_manifest_multiplier",
                &self.content_manifest_multiplier.to_string(),
                "Content manifest multiplier must be positive",
            );
        }
        if self.verified_proof_multiplier <= 0.0 {
            return EldError::validation_error(
                "verified_proof_multiplier",
                &self.verified_proof_multiplier.to_string(),
                "Verified proof multiplier must be positive",
            );
        }
        if self.device_operation_multiplier <= 0.0 {
            return EldError::validation_error(
                "device_operation_multiplier",
                &self.device_operation_multiplier.to_string(),
                "Device operation multiplier must be positive",
            );
        }
        Ok(())
    }
}

/// Maximum allowed transaction size in bytes to prevent overflow attacks
pub const MAX_TRANSACTION_SIZE_BYTES: u64 = 10 * 1024 * 1024; // 10MB
/// Maximum allowed gas usage to prevent overflow attacks
pub const MAX_GAS_USAGE: u64 = 110_000_000; // 110M gas
/// Maximum allowed fee to prevent overflow attacks
pub const MAX_FEE: u128 = u128::MAX / 4; // Quarter of max u128 to leave room for calculations

/// Calculates dynamic fee for a transaction with overflow protection
pub fn calculate_dynamic_fee(tx: &Tx, fee_config: &FeeConfig) -> Result<Coin, EldError> {
    // Add detailed logging of inputs
    info!("Fee calculation - tx type: {}", tx.payload.r#type);
    info!(
        "Fee calculation - config: base_fee={}, size_fee_per_kb={}, content_manifest_multiplier={}",
        fee_config.base_fee, fee_config.size_fee_per_kb, fee_config.content_manifest_multiplier
    );

    // 1. Base fee (minimum for any transaction)
    let base_fee = fee_config.base_fee;
    info!("Fee calculation - base_fee: {}", base_fee);

    // 2. Size-based fee (pay for bandwidth)
    let tx_size_bytes = estimate_transaction_size(tx)?;
    let size_fee = calculate_size_fee(tx_size_bytes, fee_config.size_fee_per_kb)?;

    // 3. Complexity fee (pay for computational cost)
    let complexity_multiplier = get_complexity_multiplier(&tx.payload.inner, fee_config);
    let complexity_fee = calculate_complexity_fee(base_fee, complexity_multiplier)?;

    // 4. Gas fee (pay for execution cost)
    let estimated_gas = estimate_gas_usage(&tx.payload.inner)?;
    let gas_fee = calculate_gas_fee(estimated_gas, fee_config.gas_price)?;

    // 5. Total fee with overflow protection
    let total_fee = base_fee
        .checked_add(size_fee)
        .ok_or_else(|| EldError::FeeError {
            operation: "fee_calculation".to_string(),
            details: "Fee calculation overflow: base_fee + size_fee".to_string(),
        })?
        .checked_add(complexity_fee)
        .ok_or_else(|| EldError::FeeError {
            operation: "fee_calculation".to_string(),
            details: "Fee calculation overflow: adding complexity_fee".to_string(),
        })?
        .checked_add(gas_fee)
        .ok_or_else(|| EldError::FeeError {
            operation: "fee_calculation".to_string(),
            details: "Fee calculation overflow: adding gas_fee".to_string(),
        })?;

    // Validate final fee is within reasonable bounds
    if total_fee > MAX_FEE {
        return Err(EldError::FeeError {
            operation: "fee_validation".to_string(),
            details: format!("Calculated fee {total_fee} exceeds maximum allowed fee {MAX_FEE}"),
        });
    }

    info!(
        "Fee calculation - TOTAL: {} (base={}, size={}, complexity={}, gas={})",
        total_fee, base_fee, size_fee, complexity_fee, gas_fee
    );

    Coin::new(total_fee)
}

/// Estimates transaction size with validation
fn estimate_transaction_size(tx: &Tx) -> Result<u64, EldError> {
    // Serialize transaction to estimate size
    let json_str = serde_json::to_string(tx).map_err(|e| EldError::FeeError {
        operation: "size_estimation".to_string(),
        details: format!("Failed to serialize transaction for size estimation: {e}"),
    })?;

    let size = json_str.len() as u64;

    // Validate transaction size
    if size > MAX_TRANSACTION_SIZE_BYTES {
        warn!(
            "Transaction size {} bytes exceeds maximum allowed size {} bytes",
            size, MAX_TRANSACTION_SIZE_BYTES
        );
        return Err(EldError::FeeError {
            operation: "size_validation".to_string(),
            details: format!(
                "Transaction size {size} bytes exceeds maximum allowed size {MAX_TRANSACTION_SIZE_BYTES} bytes"
            ),
        });
    }

    if size == 0 {
        warn!("Transaction serialized to zero bytes, using fallback size");
        return Ok(1024); // Fallback size if serialization results in empty string
    }

    Ok(size)
}

/// Calculates size-based fee with overflow protection
fn calculate_size_fee(tx_size_bytes: u64, size_fee_per_kb: u128) -> Result<u128, EldError> {
    // Calculate KB (with rounding up)
    let kb = tx_size_bytes.div_ceil(1024); // Round up to nearest KB

    // Calculate size fee with overflow protection
    let size_fee = (kb as u128)
        .checked_mul(size_fee_per_kb)
        .ok_or_else(|| EldError::FeeError {
            operation: "size_fee_calculation".to_string(),
            details: format!("Size fee calculation overflow: {kb} KB * {size_fee_per_kb} per KB"),
        })?;

    Ok(size_fee)
}

/// Calculates complexity fee with overflow protection
fn calculate_complexity_fee(base_fee: u128, complexity_multiplier: f64) -> Result<u128, EldError> {
    // Validate multiplier
    if complexity_multiplier <= 0.0 {
        return Err(EldError::FeeError {
            operation: "complexity_multiplier".to_string(),
            details: "Complexity multiplier must be positive".to_string(),
        });
    }

    // Calculate complexity fee using floating point arithmetic
    // Note: For very large u128 values, we need to be careful with f64 precision
    // For now, we'll use a safe conversion that works for reasonable fee values
    let total_complexity_fee = if base_fee <= u64::MAX as u128 {
        (base_fee as u64 as f64 * complexity_multiplier) as u128
    } else {
        // For very large values, use integer arithmetic with scaling
        // This is a simplified approach - in production you might want more sophisticated handling
        base_fee + ((base_fee as f64 * (complexity_multiplier - 1.0)) as u128)
    };

    // Subtract base fee with overflow protection
    let complexity_fee =
        total_complexity_fee
            .checked_sub(base_fee)
            .ok_or_else(|| EldError::FeeError {
                operation: "complexity_fee_calculation".to_string(),
                details: format!(
                    "Complexity fee calculation underflow: {total_complexity_fee} - {base_fee}"
                ),
            })?;

    Ok(complexity_fee)
}

/// Calculates gas fee with overflow protection
fn calculate_gas_fee(estimated_gas: u64, gas_price: u128) -> Result<u128, EldError> {
    // Validate gas usage
    if estimated_gas > MAX_GAS_USAGE {
        warn!(
            "Estimated gas usage {} exceeds maximum allowed gas usage {}",
            estimated_gas, MAX_GAS_USAGE
        );
        return Err(EldError::FeeError {
            operation: "gas_validation".to_string(),
            details: format!(
                "Estimated gas usage {estimated_gas} exceeds maximum allowed gas usage {MAX_GAS_USAGE}"
            ),
        });
    }

    // Calculate gas fee with overflow protection
    let gas_fee = (estimated_gas as u128)
        .checked_mul(gas_price)
        .ok_or_else(|| EldError::FeeError {
            operation: "gas_fee_calculation".to_string(),
            details: format!(
                "Gas fee calculation overflow: {estimated_gas} gas * {gas_price} price"
            ),
        })?;

    Ok(gas_fee)
}

/// Gets complexity multiplier for different transaction types
fn get_complexity_multiplier(payload: &PayloadInner, fee_config: &FeeConfig) -> f64 {
    match payload {
        // Simple operations
        PayloadInner::Transfer(_) => fee_config.transfer_multiplier,

        // Moderate complexity
        PayloadInner::Stake(_) => fee_config.stake_multiplier,
        PayloadInner::Unstake(_) => fee_config.stake_multiplier,
        PayloadInner::RegisterCapacity(_) => fee_config.device_operation_multiplier,
        PayloadInner::UnregisterCapacity(_) => fee_config.device_operation_multiplier,
        PayloadInner::UpdateCapacityMerkleRoot(_) => fee_config.device_operation_multiplier,

        // High complexity
        PayloadInner::VerifiedProof(_) => fee_config.verified_proof_multiplier,
        PayloadInner::PostMessage(_) => fee_config.content_manifest_multiplier,
        PayloadInner::AddNamespace(_) => fee_config.transfer_multiplier,
    }
}

/// Estimates gas usage for different transaction types with validation
fn estimate_gas_usage(payload: &PayloadInner) -> Result<u64, EldError> {
    let gas_usage = match payload {
        // Base gas costs
        PayloadInner::Transfer(_) => 21_000,
        PayloadInner::Stake(_) => 50_000,
        PayloadInner::Unstake(_) => 50_000,

        // TODO: Should depend on message size
        PayloadInner::PostMessage(_) => 120_000,

        PayloadInner::VerifiedProof(_) => 50_000, // Simpler than chunk proof, just recording verification

        // Device operations
        PayloadInner::RegisterCapacity(_) => 60_000,
        PayloadInner::UnregisterCapacity(_) => 40_000,
        PayloadInner::UpdateCapacityMerkleRoot(_) => 40_000, // Simpler than registration, just updating a field

        PayloadInner::AddNamespace(_) => 40_000,
    };

    // Validate gas usage
    if gas_usage > MAX_GAS_USAGE {
        warn!(
            "Estimated gas usage {} exceeds maximum allowed gas usage {}",
            gas_usage, MAX_GAS_USAGE
        );
        return Err(EldError::FeeError {
            operation: "gas_estimation".to_string(),
            details: format!(
                "Estimated gas usage {gas_usage} exceeds maximum allowed gas usage {MAX_GAS_USAGE}"
            ),
        });
    }

    Ok(gas_usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::Address;
    use crate::tx::{Payload, PayloadInner, TransferTx, Tx, TxPublicKey, TxSig};
    use ed25519_dalek::SigningKey;

    fn create_test_fee_config() -> FeeConfig {
        FeeConfig {
            base_fee: 1000u128,
            size_fee_per_kb: 100u128,
            gas_price: 10u128,
            transfer_multiplier: 1.0,
            stake_multiplier: 1.5,
            content_manifest_multiplier: 2.0,
            verified_proof_multiplier: 2.5,
            device_operation_multiplier: 1.8,
        }
    }

    fn create_test_transfer_tx() -> Tx {
        let signing_key = SigningKey::from_bytes(&[1u8; 32]);
        Tx {
            sig: TxSig::new("0".repeat(128)).expect("64-byte zero signature hex"),
            nonce: 1u32.into(),
            fee: 1000.into(),
            payload: Payload::new(
                TransferTx::new(
                    Address::parse_hex_str("0x1234567890123456789012345678901234567890")
                        .expect("test sender"),
                    Address::parse_hex_str("0xfedcba0987654321fedcba0987654321fedcba09")
                        .expect("test recipient"),
                    1000.into(),
                )
                .expect("valid test transfer"),
            ),
            public_key: TxPublicKey::from(signing_key.verifying_key()),
        }
    }

    #[test]
    fn test_fee_config_validation() {
        let config = create_test_fee_config();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_fee_config_validation_zero_base_fee() {
        let mut config = create_test_fee_config();
        config.base_fee = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_calculate_dynamic_fee_success() {
        let config = create_test_fee_config();
        let tx = create_test_transfer_tx();
        let result = calculate_dynamic_fee(&tx, &config);
        assert!(result.is_ok());
        let fee = result.unwrap();
        assert!(fee > Coin::zero());
    }

    #[test]
    fn test_estimate_transaction_size_success() {
        let tx = create_test_transfer_tx();
        let result = estimate_transaction_size(&tx);
        assert!(result.is_ok());
        let size = result.unwrap();
        assert!(size > 0);
        assert!(size <= MAX_TRANSACTION_SIZE_BYTES);
    }

    #[test]
    fn test_calculate_size_fee_success() {
        let result = calculate_size_fee(2048, 100); // 2KB * 100 per KB
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 200); // 2KB * 100 = 200
    }

    #[test]
    fn test_calculate_complexity_fee_success() {
        let result = calculate_complexity_fee(1000, 1.5);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 500); // (1000 * 1.5) - 1000 = 500
    }

    #[test]
    fn test_calculate_gas_fee_success() {
        let result = calculate_gas_fee(21000, 10);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 210000); // 21000 * 10 = 210000
    }

    #[test]
    fn test_estimate_gas_usage_success() {
        let payload = PayloadInner::Transfer(
            TransferTx::new(
                Address::parse_hex_str("0x1234567890123456789012345678901234567890")
                    .expect("test sender"),
                Address::parse_hex_str("0xfedcba0987654321fedcba0987654321fedcba09")
                    .expect("test recipient"),
                1000.into(),
            )
            .expect("valid test transfer"),
        );
        let result = estimate_gas_usage(&payload);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 21000);
    }

    #[test]
    fn test_overflow_protection() {
        // Test with maximum values that could cause overflow
        let config = FeeConfig {
            base_fee: u128::MAX / 2,
            size_fee_per_kb: u128::MAX / 2,
            gas_price: u128::MAX / 2,
            transfer_multiplier: 1.0,
            stake_multiplier: 1.5,
            content_manifest_multiplier: 2.0,
            verified_proof_multiplier: 2.5,
            device_operation_multiplier: 1.8,
        };
        let tx = create_test_transfer_tx();
        let result = calculate_dynamic_fee(&tx, &config);
        assert!(result.is_err()); // Should fail due to overflow protection
    }
}
