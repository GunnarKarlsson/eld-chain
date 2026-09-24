use crate::capacity_proof::ChunkProof;
use crate::error::EldError;
use crate::Address;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
#[serde(try_from = "VerifiedProofTxUnchecked")]
pub struct VerifiedProofTx {
    pub sender: Address,            // Storage validator (challenger)
    pub capacity_provider: Address, // Capacity provider that passed
    pub challenge_id: String,       // Challenge ID that was verified
    pub block_height: i64,          // Block height when challenge was issued
    pub verified_at_block: i64,     // Current block height
    pub verified_at_timestamp: u64, // Unix timestamp
    /// Chunk proofs from the CP response (same as P2P `CapacityChallengeResponse.proofs`).
    pub proofs: Vec<ChunkProof>,
    /// When the CP generated/signed the response (part of the CP signature preimage).
    pub generated_at: u64,
    /// Hex Ed25519 pubkey of the capacity provider (must derive to `capacity_provider`).
    pub provider_pubkey: String,
    /// Hex Ed25519 signature over the capacity challenge response signing preimage.
    pub provider_signature: String,
    /// `true` when the active capacity validator submits this challenge as a failed proof.
    /// Absent on older payloads; those deserialize as `false` (reward path).
    #[serde(default)]
    pub failed: bool,
}

#[derive(Deserialize)]
struct VerifiedProofTxUnchecked {
    sender: Address,
    capacity_provider: Address,
    challenge_id: String,
    block_height: i64,
    verified_at_block: i64,
    verified_at_timestamp: u64,
    proofs: Vec<ChunkProof>,
    generated_at: u64,
    provider_pubkey: String,
    provider_signature: String,
    #[serde(default)]
    failed: bool,
}

impl TryFrom<VerifiedProofTxUnchecked> for VerifiedProofTx {
    type Error = EldError;

    fn try_from(unchecked: VerifiedProofTxUnchecked) -> Result<Self, Self::Error> {
        let mut tx = VerifiedProofTx::new(
            unchecked.sender,
            unchecked.capacity_provider,
            unchecked.challenge_id,
            unchecked.block_height,
            unchecked.verified_at_block,
            unchecked.verified_at_timestamp,
            unchecked.proofs,
            unchecked.generated_at,
            unchecked.provider_pubkey,
            unchecked.provider_signature,
        )?;
        tx.failed = unchecked.failed;
        Ok(tx)
    }
}

/// Same rules as [`crate::validation::validate_safe_string`] for `challenge_id` (max 100).
fn validate_verified_proof_challenge_id(challenge_id: &str) -> Result<(), EldError> {
    if challenge_id.is_empty() {
        return Err(EldError::ValidationError {
            field: "string".to_string(),
            value: challenge_id.to_string(),
            details: "String cannot be empty".to_string(),
        });
    }

    const MAX_LEN: usize = 100;
    if challenge_id.len() > MAX_LEN {
        return Err(EldError::ValidationError {
            field: "string".to_string(),
            value: challenge_id.to_string(),
            details: format!("String exceeds maximum length of {MAX_LEN}"),
        });
    }

    let dangerous_chars = [
        '<', '>', '"', '\'', '&', ';', '|', '`', '$', '(', ')', '{', '}',
    ];
    for &ch in &dangerous_chars {
        if challenge_id.contains(ch) {
            return Err(EldError::ValidationError {
                field: "string".to_string(),
                value: challenge_id.to_string(),
                details: format!("String contains potentially dangerous character: {ch}"),
            });
        }
    }

    if challenge_id.chars().any(|c| c.is_control()) {
        return Err(EldError::ValidationError {
            field: "string".to_string(),
            value: challenge_id.to_string(),
            details: "String contains control characters".to_string(),
        });
    }

    Ok(())
}

impl VerifiedProofTx {
    /// Builds a verified-proof payload after validating challenge id, block heights, proof fields,
    /// and timestamp bounds.
    ///
    /// `proofs` / `provider_pubkey` / `provider_signature` / `generated_at` are the same fields the
    /// CP puts on a signed P2P `CapacityChallengeResponse` (verified again in consensus).
    /// Field list matches that wire payload.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sender: Address,
        capacity_provider: Address,
        challenge_id: String,
        block_height: i64,
        verified_at_block: i64,
        verified_at_timestamp: u64,
        proofs: Vec<ChunkProof>,
        generated_at: u64,
        provider_pubkey: String,
        provider_signature: String,
    ) -> Result<Self, EldError> {
        validate_verified_proof_challenge_id(&challenge_id)?;

        if block_height <= 0 {
            EldError::validation_error(
                "block_height",
                &block_height.to_string(),
                "Block height must be positive",
            )?;
        }

        if verified_at_block <= 0 {
            EldError::validation_error(
                "verified_at_block",
                &verified_at_block.to_string(),
                "Verified at block must be positive",
            )?;
        }

        if verified_at_block < block_height {
            EldError::validation_error(
                "verified_at_block",
                &verified_at_block.to_string(),
                "Verified at block must be >= challenge block height",
            )?;
        }

        const MIN_TIMESTAMP: u64 = 1577836800;
        const MAX_TIMESTAMP: u64 = 4102444800;
        if !(MIN_TIMESTAMP..=MAX_TIMESTAMP).contains(&verified_at_timestamp) {
            EldError::validation_error(
                "verified_at_timestamp",
                &verified_at_timestamp.to_string(),
                &format!(
                    "Verified at timestamp must be between {MIN_TIMESTAMP} and {MAX_TIMESTAMP}"
                ),
            )?;
        }

        if proofs.is_empty() {
            EldError::validation_error("proofs", "[]", "VerifiedProof proofs must be non-empty")?;
        }

        if !(MIN_TIMESTAMP..=MAX_TIMESTAMP).contains(&generated_at) {
            EldError::validation_error(
                "generated_at",
                &generated_at.to_string(),
                &format!("generated_at must be between {MIN_TIMESTAMP} and {MAX_TIMESTAMP}"),
            )?;
        }

        if provider_pubkey.is_empty() {
            EldError::validation_error(
                "provider_pubkey",
                &provider_pubkey,
                "provider_pubkey must be non-empty hex",
            )?;
        }
        if provider_signature.is_empty() {
            EldError::validation_error(
                "provider_signature",
                &provider_signature,
                "provider_signature must be non-empty hex",
            )?;
        }

        Ok(Self {
            sender,
            capacity_provider,
            challenge_id,
            block_height,
            verified_at_block,
            verified_at_timestamp,
            proofs,
            generated_at,
            provider_pubkey,
            provider_signature,
            failed: false,
        })
    }
}

impl std::fmt::Display for VerifiedProofTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "VerifiedProofTx {{\n sender: {}\n capacity_provider: {}\n challenge_id: {}\n block_height: {}\n verified_at_block: {}\n verified_at_timestamp: {}\n proofs: {}\n generated_at: {}\n failed: {}\n }}",
            self.sender,
            self.capacity_provider,
            self.challenge_id,
            self.block_height,
            self.verified_at_block,
            self.verified_at_timestamp,
            self.proofs.len(),
            self.generated_at,
            self.failed
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capacity_proof::{ChunkProof, SlotState};

    fn sample() -> VerifiedProofTx {
        VerifiedProofTx::new(
            Address::parse_hex_str("0x1111111111111111111111111111111111111111").unwrap(),
            Address::parse_hex_str("0x2222222222222222222222222222222222222222").unwrap(),
            "abc".to_string(),
            1,
            2,
            1_600_000_000,
            vec![ChunkProof {
                chunk_index: 0,
                chunk_data: vec![1],
                chunk_hash: [2u8; 32],
                merkle_proof: vec![],
                slot_state: SlotState::Proof,
            }],
            1_600_000_000,
            "03".repeat(32),
            "04".repeat(64),
        )
        .unwrap()
    }

    #[test]
    fn missing_failed_field_deserializes_as_false() {
        let mut value = serde_json::to_value(sample()).unwrap();
        value.as_object_mut().unwrap().remove("failed");
        let tx: VerifiedProofTx = serde_json::from_value(value).unwrap();
        assert!(!tx.failed);
    }

    #[test]
    fn failed_true_round_trips() {
        let mut tx = sample();
        tx.failed = true;
        let json = serde_json::to_string(&tx).unwrap();
        let again: VerifiedProofTx = serde_json::from_str(&json).unwrap();
        assert!(again.failed);
    }
}
