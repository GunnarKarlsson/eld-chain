use super::{CapacityManager, CapacityProofGenerationParams};
use eld_common::capacity_proof::{ChallengeProof, ChunkProof};
use eld_common::error::EldError;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use tracing::info;

impl CapacityManager {
    /// Generate capacity proof for a challenge
    pub async fn generate_capacity_proof(
        &self,
        params: CapacityProofGenerationParams,
    ) -> Result<ChallengeProof, EldError> {
        let CapacityProofGenerationParams {
            challenge_id,
            challenger,
            provider_id,
            chunk_indices,
            block_height,
            expected_merkle_root,
            expiration_block: _expiration_block,
            timestamp: _timestamp,
        } = params;

        // Verify this challenge is for us
        if provider_id != self.config.provider_id {
            return Err(EldError::ValidationError {
                field: "provider_id".to_string(),
                value: provider_id.to_string(),
                details: "Challenge is not for this provider".to_string(),
            });
        }

        // Verify we're initialized
        let initialized = self.initialized.lock().await;
        if !*initialized {
            return Err(EldError::ValidationError {
                field: "initialized".to_string(),
                value: "false".to_string(),
                details: "Capacity manager not initialized".to_string(),
            });
        }
        drop(initialized);

        // Verify merkle root matches (sanity check)
        let merkle_tree_guard = self.merkle_tree.lock().await;
        let merkle_tree = merkle_tree_guard
            .as_ref()
            .ok_or_else(|| EldError::ValidationError {
                field: "merkle_tree".to_string(),
                value: "none".to_string(),
                details: "Merkle tree not built".to_string(),
            })?;

        let current_root = merkle_tree.root();
        if current_root != expected_merkle_root {
            return Err(EldError::ValidationError {
                field: "merkle_root".to_string(),
                value: expected_merkle_root.to_hex(),
                details: format!(
                    "Expected root {} does not match current root {}",
                    expected_merkle_root.to_hex(),
                    current_root.to_hex()
                ),
            });
        }

        // Get slot allocator to access capacity file
        let slot_allocator = self.slot_allocator.lock().await;
        let capacity_file_path = slot_allocator.capacity_file_path().to_path_buf();
        let slot_map = slot_allocator
            .get_slot_map()
            .ok_or_else(|| EldError::ValidationError {
                field: "slot_map".to_string(),
                value: "none".to_string(),
                details: "Slot map not loaded".to_string(),
            })?
            .clone();
        drop(slot_allocator);

        // Open capacity file for reading
        let mut file = File::open(&capacity_file_path).map_err(|e| EldError::StorageError {
            operation: "open_capacity_file".to_string(),
            details: format!("Failed to open capacity file: {e}"),
        })?;

        // Generate proofs for each requested chunk
        let mut proofs = Vec::new();

        for &chunk_index in &chunk_indices {
            // Get slot info for this chunk
            let slot =
                slot_map
                    .slots
                    .get(chunk_index)
                    .ok_or_else(|| EldError::ValidationError {
                        field: "chunk_index".to_string(),
                        value: chunk_index.to_string(),
                        details: format!("Chunk index {chunk_index} out of bounds"),
                    })?;

            let offset = slot.offset;
            let chunk_size = slot.size;
            let slot_state = slot.state.clone();

            // Read chunk data from file
            file.seek(SeekFrom::Start(offset))
                .map_err(|e| EldError::StorageError {
                    operation: "seek_chunk".to_string(),
                    details: format!("Failed to seek to chunk offset {offset}: {e}"),
                })?;

            let mut chunk_data = vec![0u8; chunk_size];
            file.read_exact(&mut chunk_data)
                .map_err(|e| EldError::StorageError {
                    operation: "read_chunk".to_string(),
                    details: format!("Failed to read chunk data: {e}"),
                })?;

            // Generate Merkle proof for this chunk
            let merkle_proof = merkle_tree.generate_proof(chunk_index);
            let proof_path_length = merkle_proof.len();

            // Calculate chunk hash (SHA256)
            let chunk_hash = Self::hash_chunk(&chunk_data);

            // Clone slot_state for logging
            let slot_state_for_log = slot_state.clone();

            proofs.push(ChunkProof {
                chunk_index,
                chunk_data,
                chunk_hash,
                merkle_proof,
                slot_state,
            });

            info!(
                challenge_id = %challenge_id,
                chunk_index = chunk_index,
                chunk_size = chunk_size,
                proof_path_length = proof_path_length,
                slot_state = ?slot_state_for_log,
                "Generated proof for chunk"
            );
        }

        drop(merkle_tree_guard);

        let generated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let challenge_proof = ChallengeProof {
            challenge_id: challenge_id.to_hex(),
            provider_id,
            challenger,
            block_height,
            proofs,
            generated_at,
        };

        info!(
            challenge_id = %challenge_proof.challenge_id,
            proof_count = challenge_proof.proofs.len(),
            "Successfully generated all proofs for challenge"
        );

        Ok(challenge_proof)
    }

    /// Sign a P2P capacity challenge response with the provider wallet.
    pub async fn sign_capacity_challenge_response(
        &self,
        challenge_proof: &ChallengeProof,
    ) -> Result<(String, String), EldError> {
        let wallet = match self
            .cli
            .get_wallet_by_address(&challenge_proof.provider_id.to_string())
            .await
        {
            Ok(Some(wallet)) => wallet,
            Ok(None) => {
                return Err(EldError::StorageError {
                    operation: "sign_capacity_challenge_response".to_string(),
                    details: format!(
                        "Failed to get wallet for provider_id '{}'",
                        challenge_proof.provider_id
                    ),
                });
            }
            Err(e) => return Err(e),
        };

        wallet
            .sign_capacity_challenge_response(challenge_proof)
            .map_err(|e| EldError::StorageError {
                operation: "sign_capacity_challenge_response".to_string(),
                details: e,
            })
    }
}
