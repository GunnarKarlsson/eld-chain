use super::connection::ConsensusConnection;
use crate::errors::handle_fatal_eld_error;
use crate::storage::traits::ConsensusConnectionStorage;
use abci::types::*;
use ed25519_dalek::VerifyingKey;
use eld_common::account::Account;
use eld_common::address::Address;
use eld_common::cado::{CADOMetadata, CadoBody, CadoPath, CadoPathKey, CadoType};
use eld_common::coin::Coin;
use eld_common::error::{EldError, ErrorBuilder};
use eld_common::nonce::Nonce;
use eld_common::protocol_constants::ProtocolConstants;
use eld_common::validator::ValidatorInfo;
use tracing::{error, info, warn};

impl<S> ConsensusConnection<S>
where
    S: ConsensusConnectionStorage,
{
    pub(crate) async fn init_chain_inner(
        &self,
        init_chain_request: RequestInitChain,
    ) -> ResponseInitChain {
        info!("init_chain_request: {:?}", init_chain_request);
        let consensus_config = match self.consensus_config.lock() {
            Ok(config) => config.clone(),
            Err(e) => {
                info!("error reading 'match self.consensus_config.lock(): {:?}", e);
                handle_fatal_eld_error(e.into());
            }
        };

        info!(
            chain_id = %init_chain_request.chain_id,
            expected_chain_id = %consensus_config.chain_id,
            "Initializing chain"
        );

        if init_chain_request.chain_id != consensus_config.chain_id {
            error!(
                "Chain ID mismatch: expected {}, got {}",
                consensus_config.chain_id, init_chain_request.chain_id
            );
            handle_fatal_eld_error(EldError::InitializationError {
                component: "chain id".into(),
                details: "chain ids don't match".into(),
            });
        }

        install_protocol_constants(self, &init_chain_request.app_state_bytes);

        // Lock committed state to update validators and create accounts
        let mut committed_state = match self.committed_state.lock() {
            Ok(state) => state,
            Err(e) => {
                error!("Failed to lock committed_state in init_chain: {:?}", e);
                handle_fatal_eld_error(e.into());
            }
        };

        // Extract validators from RequestInitChain and convert to ValidatorInfo
        let mut validators = Vec::new();
        for validator_update in init_chain_request.validators {
            // Extract Ed25519 public key from ValidatorUpdate
            let public_key_bytes = match validator_update.pub_key {
                Some(pk) => match pk.sum {
                    Some(Sum::Ed25519(bytes)) => {
                        if bytes.len() != 32 {
                            error!(
                                "Invalid Ed25519 public key length: {} (expected 32)",
                                bytes.len()
                            );
                            continue;
                        }
                        bytes
                    }
                    _ => {
                        warn!("ValidatorUpdate has non-Ed25519 public key, skipping");
                        continue;
                    }
                },
                None => {
                    warn!("ValidatorUpdate has no public key, skipping");
                    continue;
                }
            };

            // Convert public key bytes to VerifyingKey and then to Address
            let public_key_array: [u8; 32] = match public_key_bytes.clone().try_into() {
                Ok(arr) => arr,
                Err(_) => {
                    error!(
                        "Failed to convert public key bytes to array: {}",
                        hex::encode(&public_key_bytes)
                    );
                    continue;
                }
            };
            let verifying_key = match VerifyingKey::from_bytes(&public_key_array) {
                Ok(key) => key,
                Err(e) => {
                    error!("Failed to create VerifyingKey from bytes: {}", e);
                    continue;
                }
            };

            let validator_addr = match Address::from_public_key(&verifying_key) {
                Ok(addr) => addr,
                Err(e) => {
                    error!("Failed to derive address from public key: {}", e);
                    continue;
                }
            };
            let address = validator_addr.to_string();

            info!("validator address: {}", address);

            // Convert power (i64) to stake (u64)
            let stake = if validator_update.power < 0 {
                warn!("Validator has negative power, using 0");
                0
            } else {
                validator_update.power as u64
            };

            // Create ValidatorInfo
            let stake_coin = match Coin::new(stake as u128) {
                Ok(coin) => coin,
                Err(e) => {
                    error!("Failed to create coin for validator stake: {}", e);
                    continue;
                }
            };
            validators.push(ValidatorInfo {
                address: validator_addr,
                stake: stake_coin,
                public_key: public_key_bytes,
            });

            // Create account for validator if it doesn't exist
            let validator_path =
                match CadoPath::new(CadoType::Account, CadoPathKey::Address(validator_addr)) {
                    Ok(path) => path,
                    Err(e) => {
                        error!("Failed to create CadoPath for validator account: {}", e);
                        continue;
                    }
                };

            // Check if account already exists in the working set (no DB read).
            if !committed_state
                .envelope
                .has_cado_in_working_set(&validator_path)
            {
                info!("Staging validator account for first commit: {}", address);
                let validator_balance = match Coin::new(stake as u128) {
                    Ok(coin) => coin,
                    Err(e) => {
                        error!("Failed to create coin for validator stake: {}", e);
                        continue;
                    }
                };
                let validator_account =
                    Account::new(validator_addr, validator_balance, Nonce::new(Nonce::ZERO));

                let serialized = match bincode::serialize(&validator_account) {
                    Ok(data) => data,
                    Err(e) => {
                        error!("Failed to serialize validator account: {}", e);
                        handle_fatal_eld_error(ErrorBuilder::storage_error(
                            "serialize account",
                            &e.to_string(),
                        ));
                    }
                };
                let metadata = CADOMetadata::new(
                    CadoType::Account,
                    validator_account.address().hex_with_prefix(),
                );

                let account_cado = CadoBody::mutable_new(serialized, metadata);
                committed_state
                    .envelope
                    .update_cado_cache(validator_path, account_cado);
            } else {
                info!("Validator account already exists: {}", address);
            }
        }

        // Set validators list in app state
        committed_state.envelope.validators = validators.clone();
        info!(
            "Set validators list in init_chain: {} validators",
            committed_state.envelope.validators.len()
        );

        let validators_per_epoch = self.protocol_constants().validators_per_epoch;
        let mut sorted_validators = validators;
        sorted_validators.sort_by(Self::compare_validator_priority);
        committed_state.envelope.active_validators = sorted_validators
            .into_iter()
            .take(validators_per_epoch)
            .collect();

        info!(
            "Set active validators in init_chain: {} validators",
            committed_state.envelope.active_validators.len()
        );

        // Set active capacity validator for epoch 0
        self.select_active_capacity_validator_for_epoch(&mut committed_state, 0);

        info!(
            "Set active capacity validator in init_chain: {:?}",
            committed_state.envelope.active_capacity_validator
        );

        Default::default()
    }
}

fn install_protocol_constants<S>(connection: &ConsensusConnection<S>, app_state_bytes: &[u8])
where
    S: ConsensusConnectionStorage,
{
    if app_state_bytes.is_empty() {
        if connection.protocol.get().is_some() {
            return;
        }
        handle_fatal_eld_error(EldError::InitializationError {
            component: "protocol_constants".into(),
            details: "genesis app_state is empty".into(),
        });
    }

    let parsed = match ProtocolConstants::from_app_state_bytes(app_state_bytes) {
        Ok(constants) => constants,
        Err(e) => handle_fatal_eld_error(e),
    };

    match connection.protocol.get() {
        Some(existing) if existing == &parsed => return,
        Some(_) => handle_fatal_eld_error(EldError::InitializationError {
            component: "protocol_constants".into(),
            details: "genesis protocol_constants do not match the loaded value".into(),
        }),
        None => {}
    }

    if let Err(e) = connection.storage.put_protocol_constants(app_state_bytes) {
        handle_fatal_eld_error(e);
    }
    if connection.protocol.set(parsed).is_err() {
        handle_fatal_eld_error(EldError::InitializationError {
            component: "protocol_constants".into(),
            details: "protocol constants were set concurrently".into(),
        });
    }
}
