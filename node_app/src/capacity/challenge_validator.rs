use eld_common::capacity::CapacityProofMerkleTree;
use eld_common::capacity_merkle_root::CapacityMerkleRoot;
use eld_common::capacity_proof::{ChunkProof, SlotState};

/// Validation result for a challenge proof
#[derive(Debug, Clone)]
pub struct ProofValidationResult {
    pub is_valid: bool,
    pub errors: Vec<String>,
}

impl ProofValidationResult {
    /// Create a valid result
    pub fn valid() -> Self {
        Self {
            is_valid: true,
            errors: Vec::new(),
        }
    }

    /// Create an invalid result with errors
    pub fn invalid(errors: Vec<String>) -> Self {
        Self {
            is_valid: false,
            errors,
        }
    }
}

/// Validate a complete challenge proof
pub fn validate_challenge_proof(
    proofs: &[ChunkProof],
    expected_merkle_root: &CapacityMerkleRoot,
    expected_chunk_indices: &[usize],
) -> ProofValidationResult {
    let mut errors = Vec::new();

    // Check that we have proofs for all requested chunks
    if proofs.len() != expected_chunk_indices.len() {
        errors.push(format!(
            "Proof count mismatch: expected {}, got {}",
            expected_chunk_indices.len(),
            proofs.len()
        ));
        return ProofValidationResult::invalid(errors);
    }

    // Validate each proof
    for (i, proof) in proofs.iter().enumerate() {
        let expected_index = expected_chunk_indices[i];

        // Check chunk index matches
        if proof.chunk_index != expected_index {
            errors.push(format!(
                "Proof {}: chunk index mismatch: expected {}, got {}",
                i, expected_index, proof.chunk_index
            ));
            continue;
        }

        // Verify chunk hash matches chunk data
        let calculated_hash =
            crate::capacity::capacity_manager::CapacityManager::hash_chunk(&proof.chunk_data);
        if calculated_hash != proof.chunk_hash {
            errors.push(format!(
                "Proof {}: chunk hash mismatch for chunk {}",
                i, proof.chunk_index
            ));
            continue;
        }

        // Verify Merkle proof
        let merkle_valid = CapacityProofMerkleTree::verify_proof(
            &proof.chunk_hash,
            &proof.merkle_proof,
            expected_merkle_root,
            proof.chunk_index,
        );

        if !merkle_valid {
            errors.push(format!(
                "Proof {}: Merkle proof verification failed for chunk {}",
                i, proof.chunk_index
            ));
            continue;
        }

        // Verify slot state consistency (optional but recommended)
        // For Proof slots, data should match deterministic generation
        // For Open slots, data should be zeros
        // For Content slots, data should match stored content
        match &proof.slot_state {
            SlotState::Open => {
                // Open slots should contain zeros
                if !proof.chunk_data.iter().all(|&b| b == 0) {
                    errors.push(format!(
                        "Proof {}: Open slot chunk {} contains non-zero data",
                        i, proof.chunk_index
                    ));
                }
            }
            SlotState::Proof => {
                // Proof slots should match deterministic generation
                // This would require provider_id and seed, so we skip for now
                // Could be added if needed
            }
            SlotState::Content { .. } => {
                // Content slots: data should match the content hash
                // This would require checking against stored content manifests
                // Could be added if needed
            }
        }
    }

    if errors.is_empty() {
        ProofValidationResult::valid()
    } else {
        ProofValidationResult::invalid(errors)
    }
}
