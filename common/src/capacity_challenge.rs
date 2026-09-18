//! Deterministic capacity challenge helpers shared by issuance and `VerifiedProof` validation.
//!
//! Challenge parameters must be recomputable by every consensus node from on-chain inputs
//! (epoch, provider address, challenge block height, active storage validator address,
//! provider `chunk_count`). Wall-clock timestamps must **not** enter `challenge_id`.

use crate::address::Address;
use crate::challenge_id::ChallengeId;
use crate::constants::protocol::CHUNKS_PER_CHALLENGE;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use sha2::{Digest, Sha256};

/// Domain separator for the RNG seed used to pick which chunks to challenge.
const CHALLENGE_CHUNK_SELECTION_DOMAIN: &[u8] = b"CHALLENGE_CHUNK_SELECTION";

/// Deterministically selects up to [`CHUNKS_PER_CHALLENGE`] chunk indices for a capacity challenge.
///
/// Seed preimage (SHA-256 → 32-byte StdRng seed):
/// `CHALLENGE_CHUNK_SELECTION || epoch_be || provider_id || block_height_be || validator_address`
///
/// All nodes with the same inputs get the same shuffled prefix of `[0..chunk_count)`.
pub fn select_challenge_chunk_indices(
    epoch: i64,
    provider_id: &Address,
    block_height: i64,
    validator_address: &Address,
    chunk_count: u32,
) -> Vec<usize> {
    let chunk_count_usize = chunk_count as usize;
    if chunk_count_usize == 0 {
        return Vec::new();
    }

    let mut challenge_seed_bytes = [0u8; 32];
    let mut hasher = Sha256::new();
    hasher.update(CHALLENGE_CHUNK_SELECTION_DOMAIN);
    hasher.update(epoch.to_be_bytes());
    hasher.update(provider_id.canonical_hex_with_prefix().as_bytes());
    hasher.update(block_height.to_be_bytes());
    hasher.update(validator_address.canonical_hex_with_prefix().as_bytes());
    challenge_seed_bytes.copy_from_slice(&hasher.finalize());

    let mut rng = StdRng::from_seed(challenge_seed_bytes);
    let mut chunk_indices: Vec<usize> = (0..chunk_count_usize).collect();
    chunk_indices.shuffle(&mut rng);
    chunk_indices.truncate(CHUNKS_PER_CHALLENGE.min(chunk_count_usize));
    chunk_indices
}

/// Computes the canonical [`ChallengeId`] as SHA-256 of the challenge preimage.
///
/// Preimage (no wall-clock timestamp — every node must recompute this in `deliver_tx`):
/// `validator_address || provider_id || block_height_be || chunk_index_0_be || …`
///
/// Edge surfaces (tx / SyncMsg) still use the 64-char hex string; call [`ChallengeId::to_hex`]
/// when writing those fields.
pub fn compute_challenge_id(
    validator_address: &Address,
    provider_id: &Address,
    block_height: i64,
    chunk_indices: &[usize],
) -> ChallengeId {
    let mut id_hasher = Sha256::new();
    id_hasher.update(validator_address.canonical_hex_with_prefix().as_bytes());
    id_hasher.update(provider_id.canonical_hex_with_prefix().as_bytes());
    id_hasher.update(block_height.to_be_bytes());
    for &idx in chunk_indices {
        id_hasher.update(idx.to_be_bytes());
    }
    let digest = id_hasher.finalize();
    let mut bytes = [0u8; ChallengeId::LEN];
    bytes.copy_from_slice(&digest);
    ChallengeId::new(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(hex: &str) -> Address {
        Address::parse_hex_str(hex).expect("test address")
    }

    const VALIDATOR: &str = "0x1111111111111111111111111111111111111111";
    const PROVIDER: &str = "0x2222222222222222222222222222222222222222";
    const OTHER_PROVIDER: &str = "0x3333333333333333333333333333333333333333";

    #[test]
    fn select_challenge_chunk_indices_is_deterministic() {
        let provider = addr(PROVIDER);
        let validator = addr(VALIDATOR);
        let a = select_challenge_chunk_indices(3, &provider, 100, &validator, 50);
        let b = select_challenge_chunk_indices(3, &provider, 100, &validator, 50);
        assert_eq!(a, b);
        assert_eq!(a.len(), CHUNKS_PER_CHALLENGE);
        assert!(a.iter().all(|&i| i < 50));
    }

    #[test]
    fn select_challenge_chunk_indices_differs_by_provider() {
        let validator = addr(VALIDATOR);
        let a = select_challenge_chunk_indices(3, &addr(PROVIDER), 100, &validator, 50);
        let b = select_challenge_chunk_indices(3, &addr(OTHER_PROVIDER), 100, &validator, 50);
        assert_ne!(a, b);
    }

    #[test]
    fn select_challenge_chunk_indices_empty_when_no_chunks() {
        let indices = select_challenge_chunk_indices(1, &addr(PROVIDER), 1, &addr(VALIDATOR), 0);
        assert!(indices.is_empty());
    }

    #[test]
    fn compute_challenge_id_is_deterministic() {
        let indices = vec![1usize, 7, 9];
        let provider = addr(PROVIDER);
        let validator = addr(VALIDATOR);
        let a = compute_challenge_id(&validator, &provider, 42, &indices);
        let b = compute_challenge_id(&validator, &provider, 42, &indices);
        assert_eq!(a, b);
        assert_eq!(a.to_hex().len(), 64); // hex of 32 bytes
    }

    #[test]
    fn compute_challenge_id_differs_by_provider() {
        let indices = vec![1usize, 7, 9];
        let validator = addr(VALIDATOR);
        let a = compute_challenge_id(&validator, &addr(PROVIDER), 42, &indices);
        let b = compute_challenge_id(&validator, &addr(OTHER_PROVIDER), 42, &indices);
        assert_ne!(a, b);
    }

    #[test]
    fn end_to_end_challenge_id_stable_for_same_inputs() {
        let provider = addr(PROVIDER);
        let validator = addr(VALIDATOR);
        let indices = select_challenge_chunk_indices(2, &provider, 55, &validator, 20);
        let id1 = compute_challenge_id(&validator, &provider, 55, &indices);
        let id2 = compute_challenge_id(
            &validator,
            &provider,
            55,
            &select_challenge_chunk_indices(2, &provider, 55, &validator, 20),
        );
        assert_eq!(id1, id2);
    }
}
