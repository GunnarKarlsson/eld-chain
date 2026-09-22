use super::CapacityManager;
use eld_common::error::EldError;
use sha2::{Digest, Sha256};
use tracing::{error, info};

impl CapacityManager {
    /// Store content chunks in open slots
    /// Finds open slots, writes chunks, marks slots as Content, and updates Merkle tree
    /// Returns the slot indices used for the chunks
    pub async fn store_content_chunks(
        &self,
        content_id: String,
        chunks: Vec<Vec<u8>>,
    ) -> Result<Vec<usize>, EldError> {
        use eld_common::constants::pinboard::MAX_CHUNK_SIZE;

        info!(
            provider_id = %self.config.provider_id,
            content_id = %content_id,
            chunk_count = chunks.len(),
            "Storing content chunks in slots"
        );

        // Find open slots for all chunks
        let mut slot_allocator = self.slot_allocator.lock().await;
        let num_slots_needed = chunks.len();
        let slot_indices = slot_allocator.find_open_slots(num_slots_needed)?;

        // Write each chunk to its slot and collect hashes and actual sizes
        let mut content_hashes = Vec::with_capacity(chunks.len());
        let mut slot_sizes = Vec::with_capacity(chunks.len());
        for (chunk_idx, chunk_data) in chunks.iter().enumerate() {
            let slot_index = slot_indices[chunk_idx];
            let actual_size = chunk_data.len();

            // Ensure chunk data fits in slot (pad if needed)
            let mut padded_chunk = chunk_data.clone();
            if padded_chunk.len() < MAX_CHUNK_SIZE {
                padded_chunk.resize(MAX_CHUNK_SIZE, 0);
            } else if padded_chunk.len() > MAX_CHUNK_SIZE {
                return Err(EldError::StorageError {
                    operation: "store_content_chunks".to_string(),
                    details: format!(
                        "Chunk {} size {} exceeds slot size {}",
                        chunk_idx,
                        padded_chunk.len(),
                        MAX_CHUNK_SIZE
                    ),
                });
            }

            // Write chunk to slot and get hash
            let chunk_hash = slot_allocator.write_chunk_to_slot(slot_index, &padded_chunk)?;
            content_hashes.push(chunk_hash);
            slot_sizes.push((slot_index, actual_size));
        }

        // Calculate committed hash (hash of all chunk hashes)
        let mut committed_hasher = Sha256::new();
        committed_hasher.update(b"CONTENT_COMMIT");
        for hash in &content_hashes {
            committed_hasher.update(hash);
        }
        let committed_hash: [u8; 32] = committed_hasher.finalize().into();

        // Mark slots as Content (with actual sizes)
        slot_allocator.mark_slots_as_content(&slot_sizes, content_id.clone(), committed_hash)?;

        // Save slot map to disk
        slot_allocator.save_slot_map()?;

        // Extract slot indices for Merkle tree update
        let slot_indices: Vec<usize> = slot_sizes.iter().map(|(idx, _)| *idx).collect();
        drop(slot_allocator);

        // Update Merkle tree incrementally
        let new_merkle_root = self
            .update_merkle_tree_for_content(&slot_indices, content_hashes)
            .await?;

        info!(
            provider_id = %self.config.provider_id,
            content_id = %content_id,
            slots_used = slot_indices.len(),
            merkle_root = new_merkle_root.to_hex(),
            "Successfully stored content chunks in slots, submitting merkle root update"
        );

        // Submit transaction to update merkle root on-chain
        match self.submit_merkle_root_update(new_merkle_root).await {
            Ok(()) => {
                info!(
                    provider_id = %self.config.provider_id,
                    content_id = %content_id,
                    merkle_root = new_merkle_root.to_hex(),
                    "Successfully submitted merkle root update transaction"
                );
            }
            Err(e) => {
                // Log error but don't fail the content storage
                // The merkle root update can be retried later if needed
                error!(
                    provider_id = %self.config.provider_id,
                    content_id = %content_id,
                    merkle_root = new_merkle_root.to_hex(),
                    error = %e,
                    error_details = format!("{:?}", e),
                    "Failed to submit merkle root update transaction (content storage succeeded)"
                );
            }
        }

        Ok(slot_indices)
    }

    /// Retrieve content from slots by content_id
    /// Reads chunks from slots and reconstructs the original content
    pub async fn get_content_from_slots(&self, content_id: &str) -> Result<Vec<u8>, EldError> {
        info!(
            provider_id = %self.config.provider_id,
            content_id = %content_id,
            "Retrieving content from slots"
        );

        // Find slot indices and sizes for this content
        let slot_allocator = self.slot_allocator.lock().await;
        let slot_sizes = slot_allocator.find_slots_by_content_id(content_id)?;

        info!(
            provider_id = %self.config.provider_id,
            content_id = %content_id,
            slot_count = slot_sizes.len(),
            "Found slots for content"
        );

        // Read chunks from slots in order and trim padding using actual sizes
        let mut content = Vec::new();
        for (slot_index, actual_size) in &slot_sizes {
            let chunk_data = slot_allocator.read_chunk_from_slot(*slot_index)?;

            // Trim padding to actual size
            if chunk_data.len() >= *actual_size {
                content.extend_from_slice(&chunk_data[..*actual_size]);
            } else {
                // If actual_size is larger than what we read, something is wrong
                return Err(EldError::StorageError {
                    operation: "get_content_from_slots".to_string(),
                    details: format!(
                        "Chunk size mismatch: expected {}, got {}",
                        actual_size,
                        chunk_data.len()
                    ),
                });
            }
        }

        drop(slot_allocator);

        info!(
            provider_id = %self.config.provider_id,
            content_id = %content_id,
            content_size = content.len(),
            "Successfully retrieved content from slots"
        );

        Ok(content)
    }

    /// Delete content from slots, release the occupied slots, and update merkle root on-chain.
    pub async fn delete_content_from_slots(&self, content_id: &str) -> Result<(), EldError> {
        info!(
            provider_id = %self.config.provider_id,
            content_id = %content_id,
            "Deleting content from slots"
        );

        let mut slot_allocator = self.slot_allocator.lock().await;
        let slot_sizes = slot_allocator.find_slots_by_content_id(content_id)?;

        let slot_zero_hashes: Vec<[u8; 32]> = {
            let slot_map = slot_allocator.slot_map_ref()?;
            let mut hashes = Vec::with_capacity(slot_sizes.len());
            for (slot_index, _) in &slot_sizes {
                let slot_size = slot_map
                    .slots
                    .get(*slot_index)
                    .ok_or_else(|| EldError::StorageError {
                        operation: "delete_content_from_slots".to_string(),
                        details: format!("Slot index {slot_index} out of bounds"),
                    })?
                    .size;
                hashes.push(Self::hash_chunk(&vec![0u8; slot_size]));
            }
            hashes
        };

        for (slot_index, _) in &slot_sizes {
            slot_allocator.write_chunk_to_slot(*slot_index, &[])?;
        }

        let released = slot_allocator.remove_content_by_id(content_id)?;
        slot_allocator.save_slot_map()?;
        drop(slot_allocator);

        let released_indices: Vec<usize> = released.iter().map(|(idx, _)| *idx).collect();
        let previous_merkle_root = self.get_merkle_root().await?;
        let new_merkle_root = self
            .update_merkle_tree_for_content(&released_indices, slot_zero_hashes)
            .await?;

        if previous_merkle_root == new_merkle_root {
            info!(
                provider_id = %self.config.provider_id,
                content_id = %content_id,
                released_slots = released_indices.len(),
                merkle_root = new_merkle_root.to_hex(),
                "Deleted content slots; merkle root unchanged, skipping merkle root update tx"
            );
            return Ok(());
        }

        match self.submit_merkle_root_update(new_merkle_root).await {
            Ok(()) => {
                info!(
                    provider_id = %self.config.provider_id,
                    content_id = %content_id,
                    released_slots = released_indices.len(),
                    previous_merkle_root = previous_merkle_root.to_hex(),
                    merkle_root = new_merkle_root.to_hex(),
                    "Deleted content slots and submitted merkle root update"
                );
            }
            Err(e) => {
                error!(
                    provider_id = %self.config.provider_id,
                    content_id = %content_id,
                    released_slots = released_indices.len(),
                    previous_merkle_root = previous_merkle_root.to_hex(),
                    merkle_root = new_merkle_root.to_hex(),
                    error = %e,
                    "Deleted content slots but failed to submit merkle root update"
                );
            }
        }

        Ok(())
    }
}
