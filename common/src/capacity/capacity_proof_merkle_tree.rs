use crate::capacity_merkle_root::CapacityMerkleRoot;
use blake3::Hasher as Blake3Hasher;

// ============================================================================
// CAPACITY PROOF MERKLE TREE
// ============================================================================

pub struct CapacityProofMerkleTree {
    leaves: Vec<[u8; 32]>,
    root: CapacityMerkleRoot,
    tree: Vec<Vec<[u8; 32]>>,
}

impl CapacityProofMerkleTree {
    /// Build a Merkle tree from leaf hashes
    pub fn build(leaves: Vec<[u8; 32]>) -> Self {
        if leaves.is_empty() {
            panic!("Cannot build Merkle tree with empty leaves");
        }

        let mut tree = vec![leaves.clone()];
        let mut current_level = leaves.clone();

        // Build tree bottom-up
        while current_level.len() > 1 {
            let mut next_level = Vec::new();
            for chunk in current_level.chunks(2) {
                if chunk.len() == 2 {
                    let hash = Self::hash_pair(&chunk[0], &chunk[1]);
                    next_level.push(hash);
                } else {
                    // Odd number of nodes, hash with itself
                    let hash = Self::hash_pair(&chunk[0], &chunk[0]);
                    next_level.push(hash);
                }
            }
            tree.push(next_level.clone());
            current_level = next_level;
        }

        let root = CapacityMerkleRoot::new(current_level[0]);
        Self { leaves, root, tree }
    }

    /// Get the Merkle root
    pub fn root(&self) -> CapacityMerkleRoot {
        self.root
    }

    /// Get the number of leaves in the tree
    pub fn leaf_count(&self) -> usize {
        self.leaves.len()
    }

    /// Incrementally update specific leaf hashes and recompute only the affected path to root.
    /// This is much faster than rebuilding the entire tree (O(k * log n) vs O(n)) when only k
    /// leaves change, which is the common case when storing content in slots.
    ///
    /// Performance: For storing 1 chunk in a 5000-slot tree, this is ~13 operations vs ~5000.
    ///
    /// # Arguments
    /// * `updates` - Vector of (leaf_index, new_hash) pairs for leaves that changed
    pub fn update_leaves(&mut self, updates: Vec<(usize, [u8; 32])>) {
        if updates.is_empty() {
            return;
        }

        // Update leaf hashes in both leaves array and tree[0]
        for (leaf_index, new_hash) in &updates {
            if *leaf_index >= self.leaves.len() {
                panic!(
                    "Leaf index {} out of bounds (max: {})",
                    leaf_index,
                    self.leaves.len() - 1
                );
            }
            self.leaves[*leaf_index] = *new_hash;
            self.tree[0][*leaf_index] = *new_hash;
        }

        // Track which nodes need recomputation at each level
        let mut nodes_to_update: std::collections::HashSet<usize> =
            updates.iter().map(|(idx, _)| *idx).collect();

        // Walk up the tree level by level, updating only affected parent nodes
        let mut current_level = 0;
        while current_level < self.tree.len() - 1 {
            let mut next_level_nodes = std::collections::HashSet::new();
            let mut parent_updates: Vec<(usize, [u8; 32])> = Vec::new();

            // First pass: read from current level and compute parent hashes
            // We do this in two passes to avoid borrow checker issues
            for &node_index in &nodes_to_update {
                let parent_index = node_index / 2;
                next_level_nodes.insert(parent_index);

                // Get sibling index (left if node is even, right if odd)
                let sibling_index = if node_index % 2 == 0 {
                    node_index + 1
                } else {
                    node_index - 1
                };

                // Read hashes from current level (immutable borrow)
                // For parent at index parent_index, children are at 2*parent_index (left) and 2*parent_index+1 (right)
                // So if node_index is even, it's the left child; if odd, it's the right child
                let current_level_nodes = &self.tree[current_level];
                let left_hash = if node_index % 2 == 0 {
                    // Even index = left child
                    current_level_nodes[node_index]
                } else {
                    // Odd index = right child, sibling (left) is at node_index - 1
                    current_level_nodes[sibling_index]
                };

                let right_hash = if node_index % 2 == 0 {
                    // Even index = left child, sibling (right) is at node_index + 1
                    if sibling_index < current_level_nodes.len() {
                        current_level_nodes[sibling_index]
                    } else {
                        // Odd number of nodes at end, hash with itself
                        current_level_nodes[node_index]
                    }
                } else {
                    // Odd index = right child
                    current_level_nodes[node_index]
                };

                // Compute parent hash: always hash_pair(left, right) for consistency
                let parent_hash = Self::hash_pair(&left_hash, &right_hash);
                parent_updates.push((parent_index, parent_hash));
            }

            // Second pass: write to next level (mutable borrow)
            let next_level_nodes_mut = &mut self.tree[current_level + 1];
            for (parent_index, parent_hash) in parent_updates {
                if parent_index < next_level_nodes_mut.len() {
                    next_level_nodes_mut[parent_index] = parent_hash;
                }
            }

            nodes_to_update = next_level_nodes;
            current_level += 1;
        }

        // Update root from the top level of the tree
        if !self.tree.is_empty() {
            let root_level = self.tree.len() - 1;
            if !self.tree[root_level].is_empty() {
                self.root = CapacityMerkleRoot::new(self.tree[root_level][0]);
            }
        }
    }

    /// Generate a Merkle proof for a leaf at the given index
    pub fn generate_proof(&self, leaf_index: usize) -> Vec<[u8; 32]> {
        if leaf_index >= self.leaves.len() {
            panic!("Leaf index out of bounds");
        }

        let mut proof = Vec::new();
        let mut current_index = leaf_index;
        let mut level = 0;

        while level < self.tree.len() - 1 {
            let current_level = &self.tree[level];
            let sibling_index = if current_index % 2 == 0 {
                current_index + 1
            } else {
                current_index - 1
            };

            if sibling_index < current_level.len() {
                proof.push(current_level[sibling_index]);
            } else {
                // Odd node at end, hash with itself (already in proof)
                proof.push(current_level[current_index]);
            }

            current_index /= 2;
            level += 1;
        }

        proof
    }

    /// Verify a Merkle proof
    pub fn verify_proof(
        leaf_hash: &[u8; 32],
        proof: &[[u8; 32]],
        root: &CapacityMerkleRoot,
        leaf_index: usize,
    ) -> bool {
        let mut current_hash = *leaf_hash;
        let mut current_index = leaf_index;

        for sibling_hash in proof {
            current_hash = if current_index % 2 == 0 {
                Self::hash_pair(&current_hash, sibling_hash)
            } else {
                Self::hash_pair(sibling_hash, &current_hash)
            };
            current_index /= 2;
        }

        current_hash == *root.as_bytes()
    }

    fn hash_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        let mut hasher = Blake3Hasher::new();
        hasher.update(b"Merkle");
        hasher.update(left);
        hasher.update(right);
        *hasher.finalize().as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merkle_tree_build_single_leaf() {
        let leaves = vec![[0x01; 32]];
        let tree = CapacityProofMerkleTree::build(leaves.clone());
        let root = tree.root();

        // Single leaf tree: root should be the leaf itself
        assert_eq!(*root.as_bytes(), leaves[0]);
    }

    #[test]
    fn test_merkle_tree_build_two_leaves() {
        let leaves = vec![[0x01; 32], [0x02; 32]];
        let tree = CapacityProofMerkleTree::build(leaves.clone());
        let root = tree.root();

        // Root should be hash of the two leaves
        assert_ne!(*root.as_bytes(), leaves[0]);
        assert_ne!(*root.as_bytes(), leaves[1]);
        assert_ne!(*root.as_bytes(), [0u8; 32]);
    }

    #[test]
    fn test_merkle_proof_generation() {
        let leaves = vec![[0x01; 32], [0x02; 32], [0x03; 32], [0x04; 32]];

        let tree = CapacityProofMerkleTree::build(leaves.clone());

        // Generate proof for each leaf
        for i in 0..leaves.len() {
            let proof = tree.generate_proof(i);
            assert!(!proof.is_empty(), "Proof should not be empty for leaf {i}");
        }
    }

    #[test]
    fn test_merkle_proof_verification_valid() {
        let leaves = vec![[0x01; 32], [0x02; 32], [0x03; 32], [0x04; 32]];

        let tree = CapacityProofMerkleTree::build(leaves.clone());
        let root = tree.root();

        // Verify proofs for all leaves
        for (i, leaf) in leaves.iter().enumerate() {
            let proof = tree.generate_proof(i);
            let is_valid = CapacityProofMerkleTree::verify_proof(leaf, &proof, &root, i);
            assert!(is_valid, "Merkle proof for leaf {i} should be valid");
        }
    }

    #[test]
    fn test_merkle_tree_build_four_leaves() {
        let leaves = vec![[0x01; 32], [0x02; 32], [0x03; 32], [0x04; 32]];

        let tree = CapacityProofMerkleTree::build(leaves.clone());
        let root = tree.root();

        // Verify root is not all zeros
        assert_ne!(*root.as_bytes(), [0u8; 32]);

        // Verify root is deterministic
        let tree2 = CapacityProofMerkleTree::build(leaves.clone());
        assert_eq!(tree2.root(), root, "Merkle root should be deterministic");
    }

    #[test]
    fn test_merkle_tree_build_odd_number_of_leaves() {
        let leaves = vec![[0x01; 32], [0x02; 32], [0x03; 32]];

        let tree = CapacityProofMerkleTree::build(leaves.clone());
        let root = tree.root();

        // Should handle odd number of leaves
        assert_ne!(*root.as_bytes(), [0u8; 32]);
    }

    #[test]
    fn test_merkle_proof_verification_invalid_proof() {
        let leaves = vec![[0x01; 32], [0x02; 32], [0x03; 32], [0x04; 32]];

        let tree = CapacityProofMerkleTree::build(leaves.clone());
        let root = tree.root();

        // Use invalid proof
        let invalid_proof = vec![[0xff; 32]];
        let is_valid = CapacityProofMerkleTree::verify_proof(&leaves[0], &invalid_proof, &root, 0);
        assert!(!is_valid, "Invalid Merkle proof should fail");
    }

    #[test]
    fn test_merkle_proof_verification_wrong_leaf() {
        let leaves = vec![[0x01; 32], [0x02; 32], [0x03; 32], [0x04; 32]];

        let tree = CapacityProofMerkleTree::build(leaves.clone());
        let root = tree.root();

        // Generate proof for leaf 0 but verify with wrong leaf
        let proof = tree.generate_proof(0);
        let is_valid = CapacityProofMerkleTree::verify_proof(&leaves[1], &proof, &root, 0);
        assert!(!is_valid, "Proof with wrong leaf should fail");
    }

    #[test]
    fn test_merkle_proof_verification_wrong_root() {
        let leaves = vec![[0x01; 32], [0x02; 32], [0x03; 32], [0x04; 32]];

        let tree = CapacityProofMerkleTree::build(leaves.clone());

        // Use wrong root
        let proof = tree.generate_proof(0);
        let wrong_root = CapacityMerkleRoot::new([0xff; 32]);
        let is_valid = CapacityProofMerkleTree::verify_proof(&leaves[0], &proof, &wrong_root, 0);
        assert!(!is_valid, "Proof with wrong root should fail");
    }

    #[test]
    fn test_merkle_tree_large_number_of_leaves() {
        // Test with 100 leaves (similar to our capacity proof scenario)
        let mut leaves = Vec::new();
        for i in 0..100 {
            let mut leaf = [0u8; 32];
            leaf[0] = i as u8;
            leaves.push(leaf);
        }

        let tree = CapacityProofMerkleTree::build(leaves.clone());
        let root = tree.root();

        assert_ne!(*root.as_bytes(), [0u8; 32]);

        // Verify a few random proofs
        for &idx in &[0, 10, 50, 99] {
            let proof = tree.generate_proof(idx);
            let is_valid = CapacityProofMerkleTree::verify_proof(&leaves[idx], &proof, &root, idx);
            assert!(is_valid, "Proof for leaf {idx} should be valid");
        }
    }

    #[test]
    fn test_merkle_tree_deterministic() {
        let leaves = vec![[0x01; 32], [0x02; 32], [0x03; 32], [0x04; 32]];

        // Build tree twice with same leaves
        let tree1 = CapacityProofMerkleTree::build(leaves.clone());
        let tree2 = CapacityProofMerkleTree::build(leaves.clone());

        // Roots should be identical
        assert_eq!(
            tree1.root(),
            tree2.root(),
            "Merkle trees should be deterministic"
        );

        // Proofs should be identical
        for i in 0..leaves.len() {
            let proof1 = tree1.generate_proof(i);
            let proof2 = tree2.generate_proof(i);
            assert_eq!(
                proof1, proof2,
                "Proofs should be deterministic for leaf {i}"
            );
        }
    }

    #[test]
    fn test_hash_pair_commutative_order() {
        let left = [0x01; 32];
        let right = [0x02; 32];

        // hash_pair should produce same result regardless of order in the tree structure
        // (but the tree structure itself determines left/right)
        let hash1 = CapacityProofMerkleTree::hash_pair(&left, &right);
        let hash2 = CapacityProofMerkleTree::hash_pair(&left, &right);

        assert_eq!(hash1, hash2, "hash_pair should be deterministic");
        assert_ne!(hash1, [0u8; 32], "Hash should not be zero");
    }

    #[test]
    fn test_incremental_update_single_leaf() {
        // Build initial tree with 4 leaves
        let mut leaves = vec![[0x01; 32], [0x02; 32], [0x03; 32], [0x04; 32]];
        let mut tree = CapacityProofMerkleTree::build(leaves.clone());
        let original_root = tree.root();

        // Update one leaf
        let new_hash = [0xFF; 32];
        leaves[1] = new_hash;
        tree.update_leaves(vec![(1, new_hash)]);
        let updated_root = tree.root();

        // Root should have changed
        assert_ne!(original_root, updated_root);

        // Rebuild tree from scratch and verify root matches
        let rebuilt_tree = CapacityProofMerkleTree::build(leaves);
        assert_eq!(
            updated_root,
            rebuilt_tree.root(),
            "Incremental update should match full rebuild"
        );
    }

    #[test]
    fn test_incremental_update_multiple_leaves() {
        // Build initial tree with 8 leaves
        let mut leaves: Vec<[u8; 32]> = (0..8)
            .map(|i| {
                let mut hash = [0u8; 32];
                hash[0] = i as u8;
                hash
            })
            .collect();
        let mut tree = CapacityProofMerkleTree::build(leaves.clone());
        let original_root = tree.root();

        // Update multiple leaves (indices 0, 3, 7)
        let updates = vec![(0, [0xAA; 32]), (3, [0xBB; 32]), (7, [0xCC; 32])];
        leaves[0] = [0xAA; 32];
        leaves[3] = [0xBB; 32];
        leaves[7] = [0xCC; 32];

        tree.update_leaves(updates);
        let updated_root = tree.root();

        // Root should have changed
        assert_ne!(original_root, updated_root);

        // Rebuild tree from scratch and verify root matches
        let rebuilt_tree = CapacityProofMerkleTree::build(leaves);
        assert_eq!(
            updated_root,
            rebuilt_tree.root(),
            "Incremental update should match full rebuild"
        );
    }

    #[test]
    fn test_incremental_update_preserves_proofs() {
        // Build initial tree
        let mut leaves: Vec<[u8; 32]> = (0..16)
            .map(|i| {
                let mut hash = [0u8; 32];
                hash[0] = i as u8;
                hash
            })
            .collect();
        let mut tree = CapacityProofMerkleTree::build(leaves.clone());

        // Get proof for leaf 5 before update
        let proof_before = tree.generate_proof(5);

        // Update a different leaf (shouldn't affect leaf 5's proof)
        tree.update_leaves(vec![(10, [0xFF; 32])]);
        leaves[10] = [0xFF; 32];

        // Get proof for leaf 5 after update
        let proof_after = tree.generate_proof(5);

        // Proofs should be different (because root changed), but verification should still work
        assert_ne!(
            proof_before, proof_after,
            "Proofs should differ after tree update"
        );

        // Verify proof still works with new root
        let new_root = tree.root();
        let leaf5_hash = leaves[5];
        assert!(
            CapacityProofMerkleTree::verify_proof(&leaf5_hash, &proof_after, &new_root, 5),
            "Proof should verify with updated root"
        );
    }
}
