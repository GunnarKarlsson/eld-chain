use crate::abci_interface::ConsensusConnection;
use crate::errors::response_deliver_tx_error_validation_failed;
use crate::storage::traits::ConsensusConnectionStorage;
use abci::types::{Event, ResponseDeliverTx};
use eld_common::{
    address::Address,
    capacity_challenge::{compute_challenge_id, select_challenge_chunk_indices},
    capacity_merkle_root::CapacityMerkleRoot,
    challenge_id::ChallengeId,
    coin::Coin,
    constants::{
        protocol::{DEFAULT_REGISTRATION_DURATION_BLOCKS, VERIFIED_PROOF_REWARD_BASE_AMOUNT},
        tx_type,
    },
    tx::{create_event_attribute, VerifiedProofTx},
    validator::{ActiveCapacityValidator, CapacityValidatorInfo},
};
use tracing::warn;

/// On-chain inputs resolved by binding gates (sender / challenged set / challenge_id).
#[derive(Debug)]
pub(crate) struct VerifiedProofBinding {
    pub challenged_provider_id: Address,
    pub validator_address: Address,
    pub expected_merkle_root: CapacityMerkleRoot,
    pub expected_indices: Vec<usize>,
    #[allow(dead_code)] // asserted in unit tests; kept for bind completeness
    pub expected_challenge_id: ChallengeId,
}

/// Gates 1–4: sender must be active V, provider challenged, challenge_id recomputable.
///
/// Pure / sync so unit tests can cover fabricated-tx rejection without spinning ABCI.
pub(crate) fn validate_verified_proof_binding(
    active_sv: Option<&ActiveCapacityValidator>,
    capacity_validators: &[CapacityValidatorInfo],
    sender: Address,
    provider: Address,
    challenge_id: &str,
    block_height: i64,
) -> Result<VerifiedProofBinding, String> {
    // Gate 1: rewards only from the epoch's active capacity validator.
    let active_sv = active_sv.ok_or_else(|| {
        "VerifiedProof rejected: no active_capacity_validator for current epoch".to_string()
    })?;
    // TODO: Handle case if epoch change while tx is in mempool (handle in Mempool)
    if sender != active_sv.validator_address {
        return Err(format!(
            "VerifiedProof rejected: sender {sender} is not active_capacity_validator {}",
            active_sv.validator_address
        ));
    }

    // Gate 2: capacity_provider must be one of the providers selected for this epoch.
    if !active_sv.challenged_providers.contains(&provider) {
        return Err(format!(
            "VerifiedProof rejected: capacity_provider {provider} is not in challenged_providers"
        ));
    }
    let challenged_provider_id = provider;

    // Gate 3: provider must have on-chain chunk_count + merkle_root.
    let provider_info = capacity_validators
        .iter()
        .find(|p| p.address == provider)
        .ok_or_else(|| {
            format!(
                "VerifiedProof rejected: capacity_provider {provider} not in capacity_validators"
            )
        })?;
    let chunk_count = match provider_info.chunk_count {
        Some(c) if c > 0 => c,
        _ => {
            return Err(format!(
                "VerifiedProof rejected: capacity_provider {provider} has no chunk_count"
            ));
        }
    };
    let expected_merkle_root =
        CapacityMerkleRoot::new(provider_info.merkle_root.ok_or_else(|| {
            format!("VerifiedProof rejected: capacity_provider {provider} has no merkle_root")
        })?);
    let validator_address = active_sv.validator_address;

    // Gate 4: recompute challenge_id from on-chain inputs; reject fabricated IDs.
    let expected_indices = select_challenge_chunk_indices(
        active_sv.epoch,
        &challenged_provider_id,
        block_height,
        &validator_address,
        chunk_count,
    );
    let parsed_challenge_id = ChallengeId::parse_hex(challenge_id)
        .map_err(|e| format!("VerifiedProof rejected: invalid challenge_id hex: {e}"))?;
    let expected_challenge_id = compute_challenge_id(
        &validator_address,
        &challenged_provider_id,
        block_height,
        &expected_indices,
    );
    if parsed_challenge_id != expected_challenge_id {
        return Err(format!(
            "VerifiedProof rejected: challenge_id mismatch (got {challenge_id}, expected {})",
            expected_challenge_id.to_hex()
        ));
    }

    Ok(VerifiedProofBinding {
        challenged_provider_id,
        validator_address,
        expected_merkle_root,
        expected_indices,
        expected_challenge_id,
    })
}

pub async fn process_verified_proof_tx<S>(
    connection: &ConsensusConnection<S>,
    verified_proof_tx: VerifiedProofTx,
) -> ResponseDeliverTx
where
    S: ConsensusConnectionStorage,
{
    let reward_coin = match Coin::new(VERIFIED_PROOF_REWARD_BASE_AMOUNT) {
        Ok(coin) => coin,
        Err(e) => {
            return response_deliver_tx_error_validation_failed(e.to_string());
        }
    };

    let provider = verified_proof_tx.capacity_provider;
    let sender = verified_proof_tx.sender;
    let challenge_id = verified_proof_tx.challenge_id.clone();

    let mut current_state_lock = connection
        .current_state
        .lock()
        .expect("Failed to acquire current_state lock");
    let current_state = current_state_lock
        .as_mut()
        .expect("current_state lock is None");

    // Gate 0: once-per-challenge_id (in-block cache + committed RocksDB).
    match current_state
        .envelope
        .is_verified_proof_challenge_rewarded(&*connection.storage, &challenge_id)
    {
        Ok(true) => {
            warn!(
                challenge_id = %challenge_id,
                "VerifiedProof rejected: challenge_id already rewarded"
            );
            return response_deliver_tx_error_validation_failed(format!(
                "VerifiedProof challenge_id already rewarded: {challenge_id}"
            ));
        }
        Ok(false) => {}
        Err(e) => {
            return response_deliver_tx_error_validation_failed(e.to_string());
        }
    }

    let binding = match validate_verified_proof_binding(
        current_state.envelope.active_capacity_validator.as_ref(),
        &current_state.envelope.capacity_validators,
        sender,
        provider,
        &challenge_id,
        verified_proof_tx.block_height,
    ) {
        Ok(b) => b,
        Err(e) => {
            warn!(reason = %e, "VerifiedProof rejected: binding validation failed");
            return response_deliver_tx_error_validation_failed(e);
        }
    };

    // Gate 5: CP signature over the challenge response (same preimage as P2P).
    if let Err(e) = eld_common::capacity_proof::verify_capacity_challenge_response(
        &challenge_id,
        &binding.challenged_provider_id,
        &binding.validator_address,
        verified_proof_tx.block_height,
        &verified_proof_tx.proofs,
        verified_proof_tx.generated_at,
        &verified_proof_tx.provider_pubkey,
        &verified_proof_tx.provider_signature,
    ) {
        warn!(
            challenge_id = %challenge_id,
            reason = %e,
            "VerifiedProof rejected: CP signature invalid"
        );
        return response_deliver_tx_error_validation_failed(format!(
            "VerifiedProof rejected: CP signature invalid: {e}"
        ));
    }

    // Gate 6: merkle / chunk proofs vs registered root and recomputed challenge indices.
    use crate::capacity::challenge_validator::{validate_challenge_proof, ProofValidationResult};
    let proof_check = validate_challenge_proof(
        &verified_proof_tx.proofs,
        &binding.expected_merkle_root,
        &binding.expected_indices,
    );
    if let ProofValidationResult {
        is_valid: false,
        errors,
    } = proof_check
    {
        warn!(
            challenge_id = %challenge_id,
            error_count = errors.len(),
            "VerifiedProof rejected: proof validation failed"
        );
        return response_deliver_tx_error_validation_failed(format!(
            "VerifiedProof rejected: proof validation failed ({} error(s))",
            errors.len()
        ));
    }

    // Mint the VerifiedProof reward onto the provider account. Same tx on every node.
    if let Err(e) = connection.reward_manager.credit_verified_proof_reward(
        current_state,
        verified_proof_tx.capacity_provider,
        reward_coin,
    ) {
        return response_deliver_tx_error_validation_failed(e.to_string());
    }

    // Extend the lease on a successful proof: duration = (current_block - registered_block) + DEFAULT_REGISTRATION_DURATION_BLOCKS.
    let current_block = (current_state.envelope.block_height + 1) as u64;
    for cv in current_state.envelope.capacity_validators.iter_mut() {
        if cv.address == provider && cv.registered_block != 0 {
            let new_duration = current_block.saturating_sub(cv.registered_block)
                + DEFAULT_REGISTRATION_DURATION_BLOCKS;
            cv.registration_duration = new_duration;
            break;
        }
    }

    // Create events: VerifiedProof + Transfer (mint from rewards_pool to provider)
    let events = vec![
        Event {
            r#type: tx_type::TX_TYPE_VERIFIED_PROOF.into(),
            attributes: vec![
                create_event_attribute("sender".into(), verified_proof_tx.sender.to_string()),
                create_event_attribute(
                    "capacity_provider".into(),
                    verified_proof_tx.capacity_provider.to_string(),
                ),
                create_event_attribute("challenge_id".into(), challenge_id.clone()),
                create_event_attribute(
                    "block_height".into(),
                    verified_proof_tx.block_height.to_string(),
                ),
                create_event_attribute(
                    "verified_at_block".into(),
                    verified_proof_tx.verified_at_block.to_string(),
                ),
                create_event_attribute(
                    "verified_at_timestamp".into(),
                    verified_proof_tx.verified_at_timestamp.to_string(),
                ),
                create_event_attribute(
                    "reward".into(),
                    VERIFIED_PROOF_REWARD_BASE_AMOUNT.to_string(),
                ),
            ],
        },
        Event {
            r#type: tx_type::TX_TYPE_TRANSFER.into(),
            attributes: vec![
                create_event_attribute("from".into(), "rewards_pool".into()),
                create_event_attribute("to".into(), provider.to_string()),
                create_event_attribute(
                    "amount".into(),
                    VERIFIED_PROOF_REWARD_BASE_AMOUNT.to_string(),
                ),
            ],
        },
    ];

    current_state
        .envelope
        .verified_proof_rewarded_cache
        .insert(challenge_id);

    ResponseDeliverTx {
        code: 0,
        log: "VerifiedProof transaction processed successfully; minted constant reward (1000 ELD) to provider"
            .to_string(),
        events,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use eld_common::address::Address;
    use eld_common::public_key::PublicKey;

    fn addr(hex: &str) -> Address {
        Address::parse_hex_str(hex).expect("test address")
    }

    fn test_public_key(seed: u8) -> PublicKey {
        PublicKey::from(SigningKey::from_bytes(&[seed; 32]).verifying_key())
    }

    const VALIDATOR: &str = "0x1111111111111111111111111111111111111111";
    const PROVIDER: &str = "0x2222222222222222222222222222222222222222";
    const OTHER: &str = "0x3333333333333333333333333333333333333333";
    const EPOCH: i64 = 3;
    const BLOCK_HEIGHT: i64 = 100;
    const CHUNK_COUNT: u32 = 50;

    fn active_sv(challenged: Vec<&str>) -> ActiveCapacityValidator {
        ActiveCapacityValidator {
            validator_address: addr(VALIDATOR),
            epoch: EPOCH,
            challenged_providers: challenged.iter().map(|s| addr(s)).collect(),
            subscribed_topics: vec![],
        }
    }

    fn provider_info(address: &str) -> CapacityValidatorInfo {
        CapacityValidatorInfo {
            address: addr(address),
            stake: Coin::zero(),
            public_key: test_public_key(7),
            storage_capacity: 1_000,
            merkle_root: Some([9u8; 32]),
            seed: Some([8u8; 32]),
            chunk_count: Some(CHUNK_COUNT),
            registered_at: Some(1),
            last_merkle_root_update: Some(1),
            registered_block: 1,
            registration_duration: 1000,
        }
    }

    fn expected_challenge_id_for(provider: &str) -> ChallengeId {
        let indices = select_challenge_chunk_indices(
            EPOCH,
            &addr(provider),
            BLOCK_HEIGHT,
            &addr(VALIDATOR),
            CHUNK_COUNT,
        );
        compute_challenge_id(&addr(VALIDATOR), &addr(provider), BLOCK_HEIGHT, &indices)
    }

    #[test]
    fn binding_accepts_matching_sender_provider_and_challenge_id() {
        let challenge_id = expected_challenge_id_for(PROVIDER);
        let binding = validate_verified_proof_binding(
            Some(&active_sv(vec![PROVIDER])),
            &[provider_info(PROVIDER)],
            addr(VALIDATOR),
            addr(PROVIDER),
            &challenge_id.to_hex(),
            BLOCK_HEIGHT,
        )
        .expect("binding should succeed");

        assert_eq!(binding.expected_challenge_id, challenge_id);
        assert_eq!(binding.challenged_provider_id, addr(PROVIDER));
        assert_eq!(binding.validator_address, addr(VALIDATOR));
        assert_eq!(
            binding.expected_merkle_root,
            CapacityMerkleRoot::new([9u8; 32])
        );
        assert!(!binding.expected_indices.is_empty());
    }

    #[test]
    fn binding_rejects_wrong_sender() {
        let challenge_id = expected_challenge_id_for(PROVIDER);
        let err = validate_verified_proof_binding(
            Some(&active_sv(vec![PROVIDER])),
            &[provider_info(PROVIDER)],
            addr(OTHER),
            addr(PROVIDER),
            &challenge_id.to_hex(),
            BLOCK_HEIGHT,
        )
        .expect_err("wrong sender");
        assert!(err.contains("not active_capacity_validator"), "{err}");
    }

    #[test]
    fn binding_rejects_provider_not_challenged() {
        let challenge_id = expected_challenge_id_for(PROVIDER);
        let err = validate_verified_proof_binding(
            Some(&active_sv(vec![OTHER])),
            &[provider_info(PROVIDER)],
            addr(VALIDATOR),
            addr(PROVIDER),
            &challenge_id.to_hex(),
            BLOCK_HEIGHT,
        )
        .expect_err("provider not challenged");
        assert!(err.contains("not in challenged_providers"), "{err}");
    }

    #[test]
    fn binding_rejects_fabricated_challenge_id() {
        let err = validate_verified_proof_binding(
            Some(&active_sv(vec![PROVIDER])),
            &[provider_info(PROVIDER)],
            addr(VALIDATOR),
            addr(PROVIDER),
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
            BLOCK_HEIGHT,
        )
        .expect_err("fabricated challenge_id");
        assert!(err.contains("challenge_id mismatch"), "{err}");
    }

    #[test]
    fn binding_rejects_missing_active_capacity_validator() {
        let challenge_id = expected_challenge_id_for(PROVIDER);
        let err = validate_verified_proof_binding(
            None,
            &[provider_info(PROVIDER)],
            addr(VALIDATOR),
            addr(PROVIDER),
            &challenge_id.to_hex(),
            BLOCK_HEIGHT,
        )
        .expect_err("missing active SV");
        assert!(err.contains("no active_capacity_validator"), "{err}");
    }

    #[test]
    fn binding_challenge_id_matches_consensus_issuance_inputs() {
        let provider = addr(PROVIDER);
        let validator = addr(VALIDATOR);
        let indices =
            select_challenge_chunk_indices(EPOCH, &provider, BLOCK_HEIGHT, &validator, CHUNK_COUNT);
        let issuance_challenge_id =
            compute_challenge_id(&validator, &provider, BLOCK_HEIGHT, &indices);

        let binding = validate_verified_proof_binding(
            Some(&active_sv(vec![PROVIDER])),
            &[provider_info(PROVIDER)],
            validator,
            provider,
            &issuance_challenge_id.to_hex(),
            BLOCK_HEIGHT,
        )
        .expect("binding should accept consensus-issued challenge_id");

        assert_eq!(binding.expected_challenge_id, issuance_challenge_id);
        assert_eq!(binding.expected_indices, indices);
        assert_eq!(binding.challenged_provider_id, provider);
        assert_eq!(binding.validator_address, validator);
    }
}
