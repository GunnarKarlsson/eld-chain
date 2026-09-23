use super::CapacityManager;
use eld_common::capacity::{CapacityProofMerkleTree, SlotMap};
use eld_common::capacity_proof::SlotState;
use eld_common::error::EldError;
use eld_common::{CapacityMerkleRoot, CapacitySeed};
use tracing::info;

impl CapacityManager {
    /// Build merkle tree from slot map
    /// This computes leaf hashes for all slots and builds the merkle tree
    pub async fn build_merkle_tree(
        &self,
        slot_map: &SlotMap,
    ) -> Result<CapacityProofMerkleTree, EldError> {
        info!(
            provider_id = %slot_map.provider_id,
            slot_count = slot_map.slots.len(),
            "Building merkle tree from slot map"
        );

        let mut leaf_hashes = Vec::with_capacity(slot_map.slots.len());

        // For each slot, compute its hash based on state
        for (chunk_index, slot) in slot_map.slots.iter().enumerate() {
            let chunk_hash = match &slot.state {
                SlotState::Proof => {
                    // Generate deterministic proof data and hash it
                    let chunk_data = Self::generate_chunk_data(
                        &slot_map.provider_id,
                        &CapacitySeed::new(slot_map.seed),
                        chunk_index,
                    );
                    Self::hash_chunk(&chunk_data[..slot.size])
                }
                SlotState::Open => {
                    // Hash zero chunk
                    let zero_chunk = vec![0u8; slot.size];
                    Self::hash_chunk(&zero_chunk)
                }
                SlotState::Content { .. } => {
                    // For content slots, we need to read from file
                    // This will be handled when content is actually stored
                    // For now, we'll need to read from file if it exists
                    // For now, treat as zero (will be updated when content is stored)
                    let zero_chunk = vec![0u8; slot.size];
                    Self::hash_chunk(&zero_chunk)
                }
            };
            leaf_hashes.push(chunk_hash);
        }

        let merkle_tree = CapacityProofMerkleTree::build(leaf_hashes);
        let merkle_root = merkle_tree.root();

        info!(
            provider_id = %slot_map.provider_id,
            merkle_root = merkle_root.to_hex(),
            "Merkle tree built successfully"
        );

        Ok(merkle_tree)
    }

    /// Update merkle tree when content is stored in slots
    /// This incrementally updates only the affected leaves
    pub async fn update_merkle_tree_for_content(
        &self,
        slot_indices: &[usize],
        content_hashes: Vec<[u8; 32]>,
    ) -> Result<CapacityMerkleRoot, EldError> {
        if slot_indices.len() != content_hashes.len() {
            return Err(EldError::StorageError {
                operation: "update_merkle_tree_for_content".to_string(),
                details: format!(
                    "Mismatch: {} slot indices but {} content hashes",
                    slot_indices.len(),
                    content_hashes.len()
                ),
            });
        }

        let mut merkle_tree_guard = self.merkle_tree.lock().await;
        let merkle_tree = merkle_tree_guard
            .as_mut()
            .ok_or_else(|| EldError::StorageError {
                operation: "update_merkle_tree_for_content".to_string(),
                details: "Merkle tree not initialized".to_string(),
            })?;

        // Validate slot indices are within bounds
        let leaf_count = merkle_tree.leaf_count();
        for &slot_index in slot_indices {
            if slot_index >= leaf_count {
                return Err(EldError::StorageError {
                    operation: "update_merkle_tree_for_content".to_string(),
                    details: format!(
                        "Slot index {} out of bounds (max: {})",
                        slot_index,
                        leaf_count - 1
                    ),
                });
            }
        }

        // Prepare updates: (leaf_index, new_hash)
        let updates: Vec<(usize, [u8; 32])> = slot_indices
            .iter()
            .zip(content_hashes.iter())
            .map(|(&idx, &hash)| (idx, hash))
            .collect();

        merkle_tree.update_leaves(updates);
        let new_root = merkle_tree.root();

        info!(
            slots_updated = slot_indices.len(),
            new_merkle_root = new_root.to_hex(),
            "Updated merkle tree for content storage"
        );

        Ok(new_root)
    }

    /// Get current merkle root
    pub async fn get_merkle_root(&self) -> Result<CapacityMerkleRoot, EldError> {
        let merkle_tree_guard = self.merkle_tree.lock().await;
        let merkle_tree = merkle_tree_guard
            .as_ref()
            .ok_or_else(|| EldError::StorageError {
                operation: "get_merkle_root".to_string(),
                details: "Merkle tree not initialized".to_string(),
            })?;
        Ok(merkle_tree.root())
    }

    /// Set merkle tree (typically after building from slot map)
    pub async fn set_merkle_tree(&self, merkle_tree: CapacityProofMerkleTree) {
        *self.merkle_tree.lock().await = Some(merkle_tree);
    }
}
