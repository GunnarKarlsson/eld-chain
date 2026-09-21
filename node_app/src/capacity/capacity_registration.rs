use eld_client::facade::ChainClient;
use eld_common::error::EldError;
use eld_common::fee::calculate_dynamic_fee;
use eld_common::tx::{Payload, RegisterCapacityTx, Tx};
use eld_common::wallet::Wallet;
use eld_common::{Address, CapacityMerkleRoot, CapacitySeed};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info};

/// Registration status for capacity proof
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapacityRegistrationStatus {
    /// Not yet registered on-chain
    NotRegistered,
    /// Registration transaction submitted, waiting for confirmation
    Pending,
    /// Registration failed
    Failed { error: String },
}

/// Service for handling on-chain capacity registration
pub struct CapacityRegistrationService {
    status: Arc<Mutex<CapacityRegistrationStatus>>,
}

/// Inputs for [`CapacityRegistrationService::submit_capacity_registration_tx`].
pub struct CapacityRegistrationTxParams<'a> {
    pub provider_address: &'a str,
    pub capacity_bytes: u64,
    pub seed: CapacitySeed,
    pub merkle_root: CapacityMerkleRoot,
    pub chunk_count: u32,
    pub wallet: &'a Wallet,
    pub cli: &'a ChainClient,
    pub chain_id: &'a str,
}

impl Default for CapacityRegistrationService {
    fn default() -> Self {
        Self::new()
    }
}

impl CapacityRegistrationService {
    /// Create a new RegistrationService
    pub fn new() -> Self {
        Self {
            status: Arc::new(Mutex::new(CapacityRegistrationStatus::NotRegistered)),
        }
    }

    /// On-chain confirmation is not yet tracked in status; always returns false.
    pub async fn is_registered(&self) -> bool {
        false
    }

    /// Submit registration transaction to Tendermint
    pub async fn submit_capacity_registration_tx(
        &self,
        params: CapacityRegistrationTxParams<'_>,
    ) -> Result<(), EldError> {
        let CapacityRegistrationTxParams {
            provider_address,
            capacity_bytes,
            seed,
            merkle_root,
            chunk_count,
            wallet,
            cli,
            chain_id,
        } = params;

        info!(
            provider_address = %provider_address,
            capacity_bytes = capacity_bytes,
            chunk_count = chunk_count,
            merkle_root = %merkle_root,
            signer_address = %wallet.address.hex_with_prefix(),
            "Submitting capacity registration transaction"
        );

        // Update status to pending
        *self.status.lock().await = CapacityRegistrationStatus::Pending;

        info!("provider_address: {}", provider_address.to_string());

        // Get next nonce for the account
        let next_nonce = cli
            .get_next_nonce_for_account(provider_address.to_string())
            .await?
            .ok_or_else(|| EldError::StorageError {
                operation: "submit_capacity_registration_tx".to_string(),
                details: "Failed to get next nonce for account".to_string(),
            })?;

        let sender_addr = Address::parse_hex_str(provider_address)?;

        // Create RegisterCapacity transaction (edge: raw [u8; 32] fields)
        let register_capacity_tx = RegisterCapacityTx::new(
            sender_addr,
            capacity_bytes,
            *merkle_root.as_bytes(),
            *seed.as_bytes(),
            chunk_count,
        )?;

        let mut tx = Tx::new(
            next_nonce,
            Payload::new(register_capacity_tx),
            wallet.verifying_key(),
        );

        // Calculate dynamic fee
        let fee_config = cli.get_fee_config();
        let dynamic_fee =
            calculate_dynamic_fee(&tx, fee_config).map_err(|e| EldError::StorageError {
                operation: "submit_capacity_registration_tx".to_string(),
                details: format!("Failed to calculate dynamic fee: {e}"),
            })?;
        tx.fee = dynamic_fee.into();

        info!(
            provider_address = %provider_address,
            calculated_fee = %dynamic_fee,
            "Calculated dynamic fee for capacity registration"
        );

        // Sign transaction
        wallet.sign(&mut tx, chain_id)?;

        // Serialize to JSON and hex
        let json = serde_json::to_string(&tx).map_err(|e| EldError::StorageError {
            operation: "submit_capacity_registration_tx".to_string(),
            details: format!("Failed to serialize transaction: {e}"),
        })?;
        let hex_encoded = hex::encode(&json);

        // Verify transaction before sending
        if !wallet.verify(&tx, chain_id)? {
            error!("Transaction verification failed before submission");
            *self.status.lock().await = CapacityRegistrationStatus::Failed {
                error: "Transaction verification failed".to_string(),
            };
            return Err(EldError::StorageError {
                operation: "submit_capacity_registration_tx".to_string(),
                details: "Transaction verification failed".to_string(),
            });
        }

        // Submit transaction via RPC
        match cli.send_tx_rpc(&hex_encoded).await {
            Ok(response) => {
                crate::broadcast_log::log_deliver_tx_events(&response);
                info!(
                    provider_address = %provider_address,
                    "Capacity registration transaction submitted successfully"
                );
                // Status remains Pending until confirmed on-chain
                Ok(())
            }
            Err(e) => {
                error!(
                    provider_address = %provider_address,
                    error = %e,
                    "Failed to submit capacity registration transaction"
                );
                *self.status.lock().await = CapacityRegistrationStatus::Failed {
                    error: format!("Transaction submission failed: {e}"),
                };
                Err(EldError::StorageError {
                    operation: "submit_capacity_registration_tx".to_string(),
                    details: format!("Failed to submit transaction: {e}"),
                })
            }
        }
    }
}

#[cfg(test)]
impl CapacityRegistrationService {
    pub async fn get_status(&self) -> CapacityRegistrationStatus {
        self.status.lock().await.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_submit_registration_tx_sets_pending() {
        use ed25519_dalek::SigningKey;
        use eld_client::facade::ChainClient;
        use eld_common::wallet::Wallet;
        use std::sync::Arc;

        let service = Arc::new(CapacityRegistrationService::new());

        // Create mock wallet and cli for test
        let signing_key = SigningKey::from_bytes(&[0u8; 32]);
        let wallet = Wallet::from_signing_key("test_wallet".to_string(), signing_key);
        let cli_config = eld_client::config::ClientConfig {
            node_host: "127.0.0.1".to_string(),
            node_port: "26657".to_string(),
            chain_id: "test-chain".to_string(),
            faucet_host: "127.0.0.1".to_string(),
            faucet_port: "8080".to_string(),
            faucet_end_point: "/faucet/request".to_string(),
            faucet_url: None,
            app_port: "9001".to_string(),
            node_url: None,
            app_url: None,
        };
        let cli = Arc::new(ChainClient::new(
            cli_config,
            eld_common::fee::FeeConfig::default(),
        ));

        // Submit registration (will fail due to account not existing)
        // The status is set to Pending at the start of the method (line 77)
        // Even if the call fails, the status should remain Pending
        let service_clone = service.clone();
        let handle = tokio::spawn(async move {
            service_clone
                .submit_capacity_registration_tx(CapacityRegistrationTxParams {
                    provider_address: "0x1234",
                    capacity_bytes: 1000,
                    seed: CapacitySeed::new([0x42; 32]),
                    merkle_root: CapacityMerkleRoot::new([0xAA; 32]),
                    chunk_count: 10,
                    wallet: &wallet,
                    cli: &cli,
                    chain_id: "test-chain",
                })
                .await
        });

        let result = handle.await;

        // Should fail (account doesn't exist on test chain; nonce lookup returns None)
        match result {
            Ok(Err(_)) => {}
            Ok(Ok(_)) => {
                panic!("Expected submit_capacity_registration_tx to fail, but it succeeded");
            }
            Err(e) => {
                panic!("submit_capacity_registration_tx task failed unexpectedly: {e}");
            }
        }

        // Status should be Pending because it's set at the start of the method
        // before any operations that might fail
        assert_eq!(
            service.get_status().await,
            CapacityRegistrationStatus::Pending
        );
    }
}
