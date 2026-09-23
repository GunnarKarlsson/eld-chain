use super::coordinator::P2pSyncCoordinator;
use super::trait_impl::P2pCoordinatorTrait;
use super::SyncMsg;
use crate::app_state::AppState;
use crate::capacity::capacity_manager::{CapacityManager, CapacityProofGenerationParams};
use eld_common::address::Address;
use eld_common::capacity_proof::ChunkProof;
use eld_common::CapacityMerkleRoot;
use eld_common::ChallengeId;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

impl P2pSyncCoordinator {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn handle_capacity_challenge(
        &self,
        capacity_manager: &CapacityManager,
        challenge_id_str: String,
        challenger: Address,
        provider: Address,
        chunk_indices: Vec<usize>,
        block_height: i64,
        merkle_root: [u8; 32],
        seed: [u8; 32],
        expiration_block: i64,
        timestamp: u64,
    ) {
        let _ = seed;
        let challenge_id = match ChallengeId::parse_hex(&challenge_id_str) {
            Ok(id) => id,
            Err(e) => {
                error!(
                    challenge_id = %challenge_id_str,
                    error = %e,
                    "Rejected capacity challenge: invalid challenge_id hex"
                );
                return;
            }
        };

        info!(
            challenge_id = %challenge_id,
            challenger = %challenger,
            capacity_provider = %provider,
            chunk_count = chunk_indices.len(),
            block_height = block_height,
            expiration_block = expiration_block,
            merkle_root = hex::encode(merkle_root),
            timestamp = timestamp,
            "Received capacity challenge via P2P"
        );

        // Generate proofs for the challenge
        match capacity_manager
            .generate_capacity_proof(CapacityProofGenerationParams {
                challenge_id,
                challenger,
                provider_id: provider,
                chunk_indices: chunk_indices.clone(),
                block_height,
                expected_merkle_root: CapacityMerkleRoot::new(merkle_root),
                expiration_block,
                timestamp,
            })
            .await
        {
            Ok(challenge_proof) => {
                info!(
                    challenge_id = %challenge_proof.challenge_id,
                    proof_count = challenge_proof.proofs.len(),
                    "Successfully generated proofs for challenge"
                );

                // Send proof response back to validator via P2P
                use eld_common::constants::p2p::ELD_STORAGE_PROOF_TOPIC_PREFIX;

                // Create proof topic: `{ELD_STORAGE_PROOF_TOPIC_PREFIX}{capacity_provider}`
                let proof_topic = format!("{ELD_STORAGE_PROOF_TOPIC_PREFIX}{provider}");

                // Create response message
                let (provider_pubkey, provider_signature) = match capacity_manager
                    .sign_capacity_challenge_response(&challenge_proof)
                    .await
                {
                    Ok(signed) => signed,
                    Err(e) => {
                        error!(
                            challenge_id = %challenge_proof.challenge_id,
                            error = %e,
                            "Failed to sign capacity challenge response"
                        );
                        return;
                    }
                };

                let proof_response = SyncMsg::CapacityChallengeResponse {
                    challenge_id: challenge_proof.challenge_id.clone(),
                    provider_id: challenge_proof.provider_id,
                    challenger: challenge_proof.challenger,
                    block_height: challenge_proof.block_height,
                    proofs: challenge_proof.proofs,
                    generated_at: challenge_proof.generated_at,
                    provider_pubkey,
                    provider_signature,
                };

                // Publish proof response to topic
                match self.publish_to_topic(&proof_topic, proof_response) {
                    Ok(()) => {
                        info!(
                            challenge_id = %challenge_proof.challenge_id,
                            proof_topic = %proof_topic,
                            "Successfully published proof response to P2P topic"
                        );
                    }
                    Err(e) => {
                        error!(
                            challenge_id = %challenge_proof.challenge_id,
                            proof_topic = %proof_topic,
                            error = %e,
                            "Failed to publish proof response to P2P topic"
                        );
                        // Note: InsufficientPeers error is already handled in publish_to_topic
                        // which will manually inject the message locally if we're subscribed
                    }
                }
            }
            Err(e) => {
                error!(
                    challenge_id = %challenge_id,
                    error = %e,
                    "Failed to generate proofs for challenge"
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn handle_capacity_challenge_response(
        &self,
        committed_state: Option<&Arc<std::sync::Mutex<AppState>>>,
        challenge_id: String,
        provider: Address,
        challenger: Address,
        block_height: i64,
        proofs: Vec<ChunkProof>,
        generated_at: u64,
        provider_pubkey: String,
        provider_signature: String,
    ) {
        // Step 6: only the selected capacity-validator wallet evaluates responses
        // and submits VerifiedProof (not the TM consensus address).
        let is_local_capacity_validator = self
            .local_identity
            .read()
            .map(|identity| identity.matches_capacity_validator_wallet(&challenger))
            .unwrap_or(false);

        if !is_local_capacity_validator {
            debug!(
                challenge_id = %challenge_id,
                challenger = %challenger,
                "Ignoring proof response: not the local capacity validator"
            );
            return;
        }

        let is_active_capacity_validator = match committed_state {
            Some(state_lock) => match state_lock.lock() {
                Ok(state) => state
                    .envelope
                    .active_capacity_validator
                    .as_ref()
                    .map(|sv| sv.validator_address == challenger)
                    .unwrap_or(false),
                Err(e) => {
                    error!(
                        challenge_id = %challenge_id,
                        error = %e,
                        "Failed to lock committed_state for active capacity validator check"
                    );
                    return;
                }
            },
            None => {
                warn!(
                    challenge_id = %challenge_id,
                    "Ignoring proof response: committed state unavailable"
                );
                false
            }
        };

        if !is_active_capacity_validator {
            debug!(
                challenge_id = %challenge_id,
                challenger = %challenger,
                "Ignoring proof response: challenger is not active_capacity_validator"
            );
            return;
        }

        info!(
            challenge_id = %challenge_id,
            capacity_provider = %provider,
            challenger = %challenger,
            block_height = block_height,
            proof_count = proofs.len(),
            "Active capacity validator: Received capacity challenge proof response via P2P"
        );

        if let Err(e) = eld_common::capacity_proof::verify_capacity_challenge_response(
            &challenge_id,
            &provider,
            &challenger,
            block_height,
            &proofs,
            generated_at,
            &provider_pubkey,
            &provider_signature,
        ) {
            error!(
                challenge_id = %challenge_id,
                capacity_provider = %provider,
                error = %e,
                "Rejected capacity challenge response: invalid provider signature"
            );
            return;
        }

        // Validate proof
        use crate::capacity::challenge_validator::{
            validate_challenge_proof, ProofValidationResult,
        };

        // Extract chunk indices from proofs
        let chunk_indices: Vec<usize> = proofs.iter().map(|p| p.chunk_index).collect();

        // Get expected merkle root from app_state.envelope.capacity_validators
        let expected_merkle_root = match committed_state {
            Some(state_lock) => {
                let state = match state_lock.lock() {
                    Ok(state) => state,
                    Err(e) => {
                        error!(
                            challenge_id = %challenge_id,
                            capacity_provider = %provider,
                            error = %e,
                            "Failed to acquire committed_state lock"
                        );
                        return;
                    }
                };

                // Find the capacity provider in the list
                match state
                    .envelope
                    .capacity_validators
                    .iter()
                    .find(|sp| sp.address == provider)
                {
                    Some(provider_info) => match provider_info.merkle_root {
                        Some(root) => CapacityMerkleRoot::new(root),
                        None => {
                            error!(
                                challenge_id = %challenge_id,
                                capacity_provider = %provider,
                                "Capacity provider has no merkle root on-chain"
                            );
                            return;
                        }
                    },
                    None => {
                        error!(
                            challenge_id = %challenge_id,
                            capacity_provider = %provider,
                            "Capacity provider not found in app_state"
                        );
                        return;
                    }
                }
            }
            None => {
                error!(
                    challenge_id = %challenge_id,
                    capacity_provider = %provider,
                    "Committed state not available for proof validation"
                );
                return;
            }
        };

        // Validate the proof
        let validation_result =
            validate_challenge_proof(&proofs, &expected_merkle_root, &chunk_indices);

        match validation_result {
            ProofValidationResult { is_valid: true, .. } => {
                info!(
                    challenge_id = %challenge_id,
                    capacity_provider = %provider,
                    proof_count = proofs.len(),
                    merkle_root = %expected_merkle_root,
                    "Proof validation succeeded"
                );

                let submitter = self.verified_proof_submitter.clone();
                let challenge_id_clone = challenge_id.clone();
                let capacity_provider_clone = provider.to_string();
                let block_height_clone = block_height;
                let proof_fields =
                    crate::wallet::verified_proof_chain_submitter::VerifiedProofSubmissionProofs {
                        proofs: proofs.clone(),
                        generated_at,
                        provider_pubkey: provider_pubkey.clone(),
                        provider_signature: provider_signature.clone(),
                    };
                tokio::spawn(async move {
                    if let Err(e) = submitter
                        .submit_verified_proof(
                            &capacity_provider_clone,
                            &challenge_id_clone,
                            block_height_clone,
                            proof_fields,
                        )
                        .await
                    {
                        error!(
                            challenge_id = %challenge_id_clone,
                            error = %e,
                            "Failed to submit proof confirmation transaction"
                        );
                    } else {
                        info!(
                            challenge_id = %challenge_id_clone,
                            "Successfully submitted proof confirmation transaction"
                        );
                    }
                });
            }
            ProofValidationResult {
                is_valid: false,
                errors,
            } => {
                error!(
                    challenge_id = %challenge_id,
                    capacity_provider = %provider,
                    proof_count = proofs.len(),
                    error_count = errors.len(),
                    errors = ?errors,
                    "Proof validation failed"
                );
            }
        }
    }
}
