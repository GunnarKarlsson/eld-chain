//! Capacity proof types used in VerifiedProofTx and P2P messages.
//! `SlotState` matches `eld_common::capacity::slot_allocator` so proofs align with on-disk slot maps.

use crate::address::Address;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Slot state for a capacity chunk (matches slot_allocator::SlotState).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlotState {
    Proof,
    Open,
    Content {
        deal_id: String,
        committed_hash: [u8; 32],
    },
}

/// Proof for a single chunk in a capacity challenge (matches capacity_manager::ChunkProof).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChunkProof {
    pub chunk_index: usize,
    pub chunk_data: Vec<u8>,
    pub chunk_hash: [u8; 32],
    pub merkle_proof: Vec<[u8; 32]>,
    pub slot_state: SlotState,
}

/// Complete proof response for a challenge
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeProof {
    pub challenge_id: String,
    pub provider_id: Address,
    pub challenger: Address,
    pub block_height: i64,
    pub proofs: Vec<ChunkProof>,
    pub generated_at: u64, // Unix timestamp
}

#[derive(Serialize)]
struct CapacityChallengeResponseSigningPayload<'a> {
    challenge_id: &'a str,
    provider_id: &'a str,
    challenger: &'a str,
    block_height: i64,
    proofs: &'a [ChunkProof],
    generated_at: u64,
    provider_pubkey: &'a str,
}

/// Canonical signing bytes for a signed [`SyncMsg::CapacityChallengeResponse`](crate::sync_msg::SyncMsg).
pub fn capacity_challenge_response_signing_bytes(
    challenge_id: &str,
    provider_id: &str,
    challenger: &str,
    block_height: i64,
    proofs: &[ChunkProof],
    generated_at: u64,
    provider_pubkey: &str,
) -> Result<Vec<u8>, String> {
    let payload = CapacityChallengeResponseSigningPayload {
        challenge_id,
        provider_id,
        challenger,
        block_height,
        proofs,
        generated_at,
        provider_pubkey,
    };

    serde_json::to_vec(&payload).map_err(|e| {
        format!("failed to serialize capacity challenge response signing payload: {e}")
    })
}

/// Signs a capacity challenge response with the provider's Ed25519 key.
pub fn sign_capacity_challenge_response(
    signing_key: &SigningKey,
    challenge_proof: &ChallengeProof,
) -> Result<(String, String), String> {
    let provider_pubkey = hex::encode(signing_key.verifying_key().to_bytes());
    let signing_bytes = capacity_challenge_response_signing_bytes(
        &challenge_proof.challenge_id,
        &challenge_proof.provider_id.hex_with_prefix(),
        &challenge_proof.challenger.hex_with_prefix(),
        challenge_proof.block_height,
        &challenge_proof.proofs,
        challenge_proof.generated_at,
        &provider_pubkey,
    )?;
    let signature = hex::encode(signing_key.sign(&signing_bytes).to_bytes());
    Ok((provider_pubkey, signature))
}

/// Verifies the provider signature on a P2P capacity challenge response.
#[allow(clippy::too_many_arguments)]
pub fn verify_capacity_challenge_response(
    challenge_id: &str,
    provider_id: &Address,
    challenger: &Address,
    block_height: i64,
    proofs: &[ChunkProof],
    generated_at: u64,
    provider_pubkey: &str,
    provider_signature: &str,
) -> Result<(), String> {
    if provider_pubkey.is_empty() || provider_signature.is_empty() {
        return Err("capacity challenge response missing provider signature".to_string());
    }

    let pubkey_bytes =
        hex::decode(provider_pubkey).map_err(|e| format!("invalid provider_pubkey hex: {e}"))?;
    let verifying_key = VerifyingKey::from_bytes(
        &pubkey_bytes
            .try_into()
            .map_err(|_| "provider_pubkey must be 32 bytes".to_string())?,
    )
    .map_err(|e| format!("invalid provider_pubkey: {e}"))?;

    let derived_provider = Address::from_public_key(&verifying_key)
        .map_err(|e| format!("failed to derive provider address from pubkey: {e}"))?;
    if derived_provider != *provider_id {
        return Err("provider_id does not match provider_pubkey".to_string());
    }

    let signing_bytes = capacity_challenge_response_signing_bytes(
        challenge_id,
        &provider_id.hex_with_prefix(),
        &challenger.hex_with_prefix(),
        block_height,
        proofs,
        generated_at,
        provider_pubkey,
    )?;

    let signature_bytes = hex::decode(provider_signature)
        .map_err(|e| format!("invalid provider_signature hex: {e}"))?;
    let signature = Signature::from_bytes(
        &signature_bytes
            .try_into()
            .map_err(|_| "provider_signature must be 64 bytes".to_string())?,
    );

    verifying_key
        .verify(&signing_bytes, &signature)
        .map_err(|e| format!("provider signature verification failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn sample_challenge_proof(signing_key: &SigningKey) -> ChallengeProof {
        let provider_id = Address::from_public_key(&signing_key.verifying_key())
            .expect("derive provider address");

        ChallengeProof {
            challenge_id: "challenge-1".to_string(),
            provider_id,
            challenger: Address::parse_hex_str("0x4444444444444444444444444444444444444444")
                .expect("challenger"),
            block_height: 42,
            proofs: vec![ChunkProof {
                chunk_index: 0,
                chunk_data: vec![1, 2, 3],
                chunk_hash: [9u8; 32],
                merkle_proof: vec![[8u8; 32]],
                slot_state: SlotState::Proof,
            }],
            generated_at: 1_710_000_000,
        }
    }

    #[test]
    fn challenge_proof_address_fields_round_trip_json() {
        let signing_key = SigningKey::from_bytes(&[9u8; 32]);
        let proof = sample_challenge_proof(&signing_key);
        let json = serde_json::to_string(&proof).expect("serialize");
        assert!(json.contains("0x"));

        let roundtrip: ChallengeProof = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(roundtrip.provider_id, proof.provider_id);
        assert_eq!(roundtrip.challenger, proof.challenger);
    }

    #[test]
    fn capacity_challenge_response_signature_round_trip() {
        let signing_key = SigningKey::from_bytes(&[5u8; 32]);
        let challenge_proof = sample_challenge_proof(&signing_key);
        let (provider_pubkey, provider_signature) =
            sign_capacity_challenge_response(&signing_key, &challenge_proof)
                .expect("sign response");

        verify_capacity_challenge_response(
            &challenge_proof.challenge_id,
            &challenge_proof.provider_id,
            &challenge_proof.challenger,
            challenge_proof.block_height,
            &challenge_proof.proofs,
            challenge_proof.generated_at,
            &provider_pubkey,
            &provider_signature,
        )
        .expect("verify response");
    }

    #[test]
    fn capacity_challenge_response_rejects_tampered_proofs() {
        let signing_key = SigningKey::from_bytes(&[6u8; 32]);
        let challenge_proof = sample_challenge_proof(&signing_key);
        let (provider_pubkey, provider_signature) =
            sign_capacity_challenge_response(&signing_key, &challenge_proof)
                .expect("sign response");

        let mut tampered_proofs = challenge_proof.proofs.clone();
        tampered_proofs[0].chunk_data.push(99);

        let err = verify_capacity_challenge_response(
            &challenge_proof.challenge_id,
            &challenge_proof.provider_id,
            &challenge_proof.challenger,
            challenge_proof.block_height,
            &tampered_proofs,
            challenge_proof.generated_at,
            &provider_pubkey,
            &provider_signature,
        )
        .expect_err("tampered proofs should fail verification");

        assert!(
            err.contains("provider signature verification failed"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn capacity_challenge_response_rejects_wrong_provider_pubkey() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let other_key = SigningKey::from_bytes(&[8u8; 32]);
        let challenge_proof = sample_challenge_proof(&signing_key);
        let (_, provider_signature) =
            sign_capacity_challenge_response(&signing_key, &challenge_proof)
                .expect("sign response");
        let wrong_pubkey = hex::encode(other_key.verifying_key().to_bytes());

        let err = verify_capacity_challenge_response(
            &challenge_proof.challenge_id,
            &challenge_proof.provider_id,
            &challenge_proof.challenger,
            challenge_proof.block_height,
            &challenge_proof.proofs,
            challenge_proof.generated_at,
            &wrong_pubkey,
            &provider_signature,
        )
        .expect_err("wrong pubkey should fail");

        assert!(
            err.contains("provider_id does not match provider_pubkey")
                || err.contains("provider signature verification failed"),
            "unexpected error: {err}"
        );
    }
}
