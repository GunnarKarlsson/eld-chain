use super::CapacityManager;
use blake3::Hasher as Blake3Hasher;
use eld_common::address::Address;
use eld_common::constants::pinboard::MAX_CHUNK_SIZE;
use eld_common::CapacitySeed;
use sha2::{Digest, Sha256};

impl CapacityManager {
    /// Hash a chunk using SHA256 (matching eld_proof_access)
    pub fn hash_chunk(chunk_data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"CHUNK_HASH");
        hasher.update(chunk_data);
        hasher.finalize().into()
    }

    /// Generate deterministic chunk data for Proof slots
    pub fn generate_chunk_data(
        provider_id: &Address,
        seed: &CapacitySeed,
        chunk_index: usize,
    ) -> Vec<u8> {
        let mut hasher = Self::create_deterministic_hasher(b"CAPACITY_PROOF", seed, provider_id);
        hasher.update(&chunk_index.to_le_bytes());

        // Use BLAKE3 XOF to generate exactly MAX_CHUNK_SIZE bytes
        let mut output = vec![0u8; MAX_CHUNK_SIZE];
        hasher.finalize_xof().fill(&mut output);
        output
    }

    /// Create a deterministic BLAKE3 hasher with a prefix, seed, and provider_id
    pub(super) fn create_deterministic_hasher(
        prefix: &[u8],
        seed: &CapacitySeed,
        provider_id: &Address,
    ) -> Blake3Hasher {
        let mut hasher = Blake3Hasher::new();
        hasher.update(prefix);
        hasher.update(seed.as_bytes());
        hasher.update(provider_id.canonical_hex_with_prefix().as_bytes());
        hasher
    }
}
