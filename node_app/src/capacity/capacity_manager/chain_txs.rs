use super::CapacityManager;
use crate::capacity::capacity_registration::CapacityRegistrationTxParams;
use eld_common::constants::pinboard::MAX_CHUNK_SIZE;
use eld_common::error::EldError;
use eld_common::{CapacityMerkleRoot, CapacitySeed};
use tracing::{error, info};

impl CapacityManager {
    /// Check if capacity is registered on-chain
    pub async fn is_registered(&self) -> bool {
        self.registration_service.is_registered().await
    }

    /// Register capacity on-chain
    /// This submits a registration transaction after capacity has been allocated
    /// Handles wallet retrieval, chain ID, and chunk count calculation internally
    pub async fn register_capacity_onchain(
        &self,
        capacity_bytes: u64,
        seed: CapacitySeed,
        merkle_root: CapacityMerkleRoot,
    ) -> Result<(), EldError> {
        info!(
            provider_id = %self.config.provider_id,
            capacity_bytes = capacity_bytes,
            "Registering capacity on-chain"
        );

        // Check if already registered
        if self.is_registered().await {
            info!("Capacity already registered, skipping");
            return Ok(());
        }

        // Get CLI and consensus config
        let cli = &self.cli;
        let consensus_config = &self.consensus_config;

        // Get chain ID from consensus config
        let chain_id = consensus_config
            .lock()
            .unwrap_or_else(|e| {
                error!("Failed to acquire consensus config lock: {}", e);
                std::process::exit(1);
            })
            .chain_id
            .clone();

        let wallet_name = self.capacity_validator_wallet_name.as_str();
        let wallet = cli
            .get_wallet_by_name(wallet_name.to_string())
            .await?
            .ok_or_else(|| EldError::StorageError {
                operation: "register_capacity_onchain".to_string(),
                details: format!(
                    "Failed to get capacity-validator wallet '{wallet_name}' for RegisterCapacity"
                ),
            })?;

        // Calculate chunk count
        let chunk_count = capacity_bytes.div_ceil(MAX_CHUNK_SIZE as u64) as u32;

        // On-chain RegisterCapacity sender is the capacity-validator wallet address.
        let provider_address = wallet.address.hex_with_prefix();

        info!(
            capacity_validator_wallet = %wallet_name,
            register_sender = %provider_address,
            local_provider_id = %self.config.provider_id,
            "Submitting RegisterCapacity with capacity-validator wallet"
        );

        // Submit registration transaction
        self.registration_service
            .submit_capacity_registration_tx(CapacityRegistrationTxParams {
                provider_address: &provider_address,
                capacity_bytes,
                seed,
                merkle_root,
                chunk_count,
                wallet: &wallet,
                cli: cli.as_ref(),
                chain_id: &chain_id,
                fee_config: self.protocol_fee_config()?,
            })
            .await?;

        info!("Capacity registration transaction submitted successfully");
        Ok(())
    }

    /// Submit a transaction to update the capacity provider's merkle root on-chain
    pub async fn submit_merkle_root_update(
        &self,
        new_merkle_root: CapacityMerkleRoot,
    ) -> Result<(), EldError> {
        use eld_common::tx::{Payload, Tx, UpdateCapacityMerkleRootTx};

        info!(
            provider_id = %self.config.provider_id,
            merkle_root = %new_merkle_root,
            "Submitting merkle root update transaction"
        );

        // Get wallet by provider_id (address)
        let wallet = match self
            .cli
            .get_wallet_by_address(&self.config.provider_id.to_string())
            .await
        {
            Ok(Some(wallet)) => {
                info!(
                    provider_id = %self.config.provider_id,
                    wallet_address = %wallet.address.hex_with_prefix(),
                    "Found wallet for provider"
                );
                wallet
            }
            Ok(None) => {
                error!(
                    provider_id = %self.config.provider_id,
                    "Wallet not found for provider_id - cannot submit merkle root update"
                );
                return Err(EldError::StorageError {
                    operation: "submit_merkle_root_update".to_string(),
                    details: format!(
                        "Failed to get wallet for provider_id '{}'. Make sure the wallet exists and matches the provider_id.",
                        self.config.provider_id
                    ),
                });
            }
            Err(e) => return Err(e),
        };

        // Get chain ID
        let chain_id = self
            .consensus_config
            .lock()
            .map_err(|e| EldError::StorageError {
                operation: "submit_merkle_root_update".to_string(),
                details: format!("Failed to acquire consensus config lock: {e}"),
            })?
            .chain_id
            .clone();

        // Get next nonce
        let provider_address = self.config.provider_id;
        let next_nonce = match self
            .cli
            .get_next_nonce_for_account(provider_address.to_string())
            .await
        {
            Ok(Some(nonce)) => {
                info!(
                    provider_id = %self.config.provider_id,
                    next_nonce = %nonce,
                    "Retrieved next nonce for account"
                );
                nonce
            }
            Ok(None) => {
                error!(
                    provider_id = %self.config.provider_id,
                    "Failed to get next nonce for account"
                );
                return Err(EldError::StorageError {
                    operation: "submit_merkle_root_update".to_string(),
                    details: format!(
                        "Failed to get next nonce for account '{provider_address}'. Account may not exist on-chain yet."
                    ),
                });
            }
            Err(e) => {
                error!(
                    provider_id = %self.config.provider_id,
                    error = %e,
                    "Failed to get next nonce for account"
                );
                return Err(e);
            }
        };

        let sender_addr = provider_address;

        // Create transaction (edge: raw [u8; 32] field)
        let update_tx = UpdateCapacityMerkleRootTx::new(sender_addr, *new_merkle_root.as_bytes())?;

        let mut tx = Tx::new(next_nonce, Payload::new(update_tx), wallet.verifying_key());

        // Calculate dynamic fee
        let fee_config = self.protocol_fee_config()?;
        let dynamic_fee =
            eld_common::fee::calculate_dynamic_fee(&tx, &fee_config).map_err(|e| {
                EldError::StorageError {
                    operation: "submit_merkle_root_update".to_string(),
                    details: format!("Failed to calculate dynamic fee: {e}"),
                }
            })?;
        tx.fee = dynamic_fee.into();

        // Sign transaction
        wallet.sign(&mut tx, &chain_id)?;

        // Verify transaction before sending
        if !wallet.verify(&tx, &chain_id)? {
            error!("Transaction verification failed before submission");
            return Err(EldError::StorageError {
                operation: "submit_merkle_root_update".to_string(),
                details: "Transaction verification failed".to_string(),
            });
        }

        // Serialize to JSON and hex
        let json = serde_json::to_string(&tx).map_err(|e| EldError::StorageError {
            operation: "submit_merkle_root_update".to_string(),
            details: format!("Failed to serialize transaction: {e}"),
        })?;
        let hex_encoded = hex::encode(&json);

        // Submit transaction via RPC
        info!(
            provider_id = %self.config.provider_id,
            tx_nonce = %tx.nonce,
            tx_fee = %tx.fee,
            "Submitting merkle root update transaction to Tendermint RPC"
        );

        match self.cli.send_tx_rpc(&hex_encoded).await {
            Ok(response) => {
                crate::broadcast_log::log_deliver_tx_events(&response);
                info!(
                    provider_id = %self.config.provider_id,
                    merkle_root = new_merkle_root.to_hex(),
                    tx_nonce = %tx.nonce,
                    "Successfully submitted merkle root update transaction to Tendermint RPC"
                );
            }
            Err(e) => {
                error!(
                    provider_id = %self.config.provider_id,
                    merkle_root = new_merkle_root.to_hex(),
                    tx_nonce = %tx.nonce,
                    error = %e,
                    error_details = format!("{:?}", e),
                    "Failed to submit merkle root update transaction to Tendermint RPC"
                );
                return Err(EldError::StorageError {
                    operation: "submit_merkle_root_update".to_string(),
                    details: format!("Failed to submit transaction: {e}"),
                });
            }
        }

        Ok(())
    }

    /// Get registration info needed for on-chain registration
    /// Returns (capacity_bytes, seed, merkle_root) if capacity is allocated
    pub async fn get_registration_info(
        &self,
    ) -> Result<(u64, CapacitySeed, CapacityMerkleRoot), EldError> {
        let slot_allocator = self.slot_allocator.lock().await;
        let slot_map = slot_allocator
            .get_slot_map()
            .ok_or_else(|| EldError::StorageError {
                operation: "get_registration_info".to_string(),
                details: "Slot map not loaded - capacity not allocated".to_string(),
            })?;

        let capacity_bytes = slot_map.capacity_bytes;
        let seed = CapacitySeed::new(slot_map.seed);
        drop(slot_allocator);

        let merkle_root = self.get_merkle_root().await?;

        Ok((capacity_bytes, seed, merkle_root))
    }
}
