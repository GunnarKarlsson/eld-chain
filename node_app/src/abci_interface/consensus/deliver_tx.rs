use super::connection::ConsensusConnection;
use super::tx_deliver::ConsensusTxDeliver;
use crate::errors::handle_recoverable_eld_error;
use crate::errors::{
    response_deliver_tx_error_fee_calculation_failed,
    response_deliver_tx_error_fee_validation_failed,
    response_deliver_tx_error_hex_validation_failed, response_deliver_tx_error_insufficient_fee,
    response_deliver_tx_error_insufficient_funds, response_deliver_tx_error_internal_lock_failed,
    response_deliver_tx_error_invalid_coin_amount, response_deliver_tx_error_invalid_hex_encoding,
    response_deliver_tx_error_invalid_path_format, response_deliver_tx_error_invalid_utf8_encoding,
    response_deliver_tx_error_json_parsing_failed,
    response_deliver_tx_error_json_validation_failed,
    response_deliver_tx_error_nonce_not_sequential, response_deliver_tx_error_nonce_overflow,
    response_deliver_tx_error_sender_doesnt_exist,
    response_deliver_tx_error_transaction_structure_invalid,
    response_deliver_tx_error_tx_too_large, response_deliver_tx_error_verification_failed,
};
use crate::storage::traits::ConsensusConnectionStorage;
use abci::types::*;
use eld_common::account::Account;
use eld_common::cado::{CADOMetadata, CadoBody, CadoPath, CadoPathKey, CadoType};
use eld_common::error::EldError;
use eld_common::tx::HasSender;
use eld_common::tx::Tx;
use eld_common::validation::{
    self, validate_hex_string, validate_json_string, validate_transaction_structure,
};
use tracing::{error, info, warn};

impl<S> ConsensusConnection<S>
where
    S: ConsensusConnectionStorage,
{
    pub(crate) async fn deliver_tx_inner(
        &self,
        deliver_tx_request: RequestDeliverTx,
    ) -> ResponseDeliverTx {
        let tx_bytes = deliver_tx_request.tx;

        let max_tx_bytes = match self.consensus_config.lock() {
            Ok(config) => config.max_tx_bytes,
            Err(e) => {
                handle_recoverable_eld_error(e.into());
                return response_deliver_tx_error_internal_lock_failed(
                    "consensus config lock error for reading max_tx_bytes".to_string(),
                );
            }
        };
        if tx_bytes.len() > max_tx_bytes {
            warn!(
                tx_size = tx_bytes.len(),
                max_size = max_tx_bytes,
                "Transaction too large in deliver_tx"
            );
            return response_deliver_tx_error_tx_too_large(max_tx_bytes, tx_bytes.len());
        }

        // Step 1: Convert bytes to UTF-8 string with validation
        let hex_str = match String::from_utf8(tx_bytes) {
            Ok(s) => s,
            Err(e) => {
                handle_recoverable_eld_error(EldError::TransactionError {
                    tx_type: "UTF-8 validation".to_string(),
                    details: e.to_string(),
                });
                return response_deliver_tx_error_invalid_utf8_encoding(e.to_string());
            }
        };

        // Step 2: Validate hex string format and size before decoding
        const MAX_HEX_BYTES: usize = 1024 * 1024; // 1MB
        if let Err(e) = validate_hex_string(&hex_str, MAX_HEX_BYTES) {
            handle_recoverable_eld_error(EldError::ValidationError {
                field: "hex_string".to_string(),
                value: "transaction_hex".to_string(),
                details: e.to_string(),
            });
            return response_deliver_tx_error_hex_validation_failed(e.to_string());
        }

        // Step 3: Decode hex with size validation
        let decoded_bytes = match hex::decode(&hex_str) {
            Ok(bytes) => bytes,
            Err(e) => {
                handle_recoverable_eld_error(EldError::ValidationError {
                    field: "hex_encoding".to_string(),
                    value: "transaction_hex".to_string(),
                    details: e.to_string(),
                });
                return response_deliver_tx_error_invalid_hex_encoding(e.to_string());
            }
        };

        // Step 4: Convert decoded bytes to UTF-8 string
        let decoded_json = match String::from_utf8(decoded_bytes) {
            Ok(s) => s,
            Err(e) => {
                handle_recoverable_eld_error(EldError::TransactionError {
                    tx_type: "hex to UTF-8 conversion".to_string(),
                    details: e.to_string(),
                });
                return response_deliver_tx_error_invalid_utf8_encoding(e.to_string());
            }
        };

        // Step 5: Validate JSON string before parsing
        // Use reasonable limits: max 10MB JSON, max 10 levels of nesting
        const MAX_JSON_SIZE: usize = 10 * 1024 * 1024; // 10MB
        const MAX_JSON_DEPTH: usize = 10;
        if let Err(e) = validate_json_string(&decoded_json, MAX_JSON_SIZE, MAX_JSON_DEPTH) {
            info!("JSON too large. Error");
            handle_recoverable_eld_error(e.clone());
            return response_deliver_tx_error_json_validation_failed(e.to_string());
        }

        // Step 6: Parse JSON to Value first for structure validation
        let json_value: serde_json::Value = match serde_json::from_str(&decoded_json) {
            Ok(value) => value,
            Err(e) => {
                handle_recoverable_eld_error(EldError::TransactionError {
                    tx_type: "JSON parsing".to_string(),
                    details: e.to_string(),
                });
                return response_deliver_tx_error_json_parsing_failed(e.to_string());
            }
        };

        // Step 7: Validate transaction structure before deserialization
        if let Err(e) = validate_transaction_structure(&json_value) {
            handle_recoverable_eld_error(e.clone());
            return response_deliver_tx_error_transaction_structure_invalid(e.to_string());
        }

        // Step 9: Deserialize to Tx struct (now safe after validation)
        let tx: Tx = match serde_json::from_str(&decoded_json) {
            Ok(tx) => tx,
            Err(e) => {
                handle_recoverable_eld_error(EldError::TransactionError {
                    tx_type: "JSON deserialization".to_string(),
                    details: e.to_string(),
                });
                return response_deliver_tx_error_json_parsing_failed(e.to_string());
            }
        };

        // Get chain_id from committed state for transaction verification (not current state)
        let chain_id = match self.committed_state.lock() {
            Ok(state) => state.chain_id.clone(),
            Err(e) => {
                handle_recoverable_eld_error(e.into());
                return response_deliver_tx_error_internal_lock_failed(
                    "committed state lock error".to_string(),
                );
            }
        };

        // Verify transaction signature immediately after deserialization but before any changes to current state
        if !tx.verify(&chain_id).unwrap_or(false) {
            error!("Transaction verification failed in deliver_tx - signature invalid");
            return response_deliver_tx_error_verification_failed();
        }

        // Validate dynamic fee
        let fee_config = match self.consensus_config.lock() {
            Ok(config) => config.fee_config.clone(),
            Err(e) => {
                handle_recoverable_eld_error(e.into());
                return response_deliver_tx_error_internal_lock_failed(
                    "consensus config lock for fee validation".to_string(),
                );
            }
        };

        let required_fee = match eld_common::fee::calculate_dynamic_fee(&tx, &fee_config) {
            Ok(fee) => fee,
            Err(e) => {
                handle_recoverable_eld_error(EldError::TransactionError {
                    tx_type: "fee calculation".to_string(),
                    details: e.to_string(),
                });
                return response_deliver_tx_error_fee_calculation_failed(e.to_string());
            }
        };

        let provided_fee = match tx.fee.to_coin() {
            Ok(coin) => coin,
            Err(e) => {
                let error_string = e.to_string();
                handle_recoverable_eld_error(e);
                return response_deliver_tx_error_invalid_coin_amount(error_string);
            }
        };

        if provided_fee < required_fee {
            warn!(
                required_fee = %required_fee,
                provided_fee = %provided_fee,
                "Insufficient fee in deliver_tx"
            );
            return response_deliver_tx_error_insufficient_fee(
                required_fee.amount(),
                tx.fee.as_u128(),
            );
        }

        {
            let sender_addr = tx.payload.inner.sender();
            let sender_address = sender_addr.to_string();

            let mut current_state_lock = match self.current_state.lock() {
                Ok(lock) => lock,
                Err(e) => {
                    handle_recoverable_eld_error(e.into());
                    return response_deliver_tx_error_internal_lock_failed(
                        "current state lock error".to_string(),
                    );
                }
            };
            let current_state = match current_state_lock.as_mut() {
                Some(state) => state,
                None => {
                    handle_recoverable_eld_error(EldError::InitializationError {
                        component: "current state".to_string(),
                        details: "state is None".to_string(),
                    });
                    return response_deliver_tx_error_internal_lock_failed(
                        "current state is None".to_string(),
                    );
                }
            };

            let sender_path =
                match CadoPath::new(CadoType::Account, CadoPathKey::Address(sender_addr)) {
                    Ok(path) => path,
                    Err(e) => {
                        handle_recoverable_eld_error(EldError::ValidationError {
                            field: "account_path".to_string(),
                            value: "sender_account_path".to_string(),
                            details: e.to_string(),
                        });
                        return response_deliver_tx_error_invalid_path_format(e.to_string());
                    }
                };

            let sender_account_with_hash = match current_state
                .envelope
                .get_account_from_cado(&*self.storage, &sender_path)
            {
                Some(account_with_hash) => account_with_hash,
                None => {
                    error!("Sender account not found: {}", sender_address);
                    return response_deliver_tx_error_sender_doesnt_exist(sender_address.clone());
                }
            };
            let sender_account = sender_account_with_hash.account;
            let expected_nonce = match sender_account.nonce().next() {
                Some(nonce) => nonce,
                None => {
                    warn!(
                        account_nonce = sender_account.nonce().value(),
                        tx_nonce = tx.nonce.value(),
                        "Account nonce overflow"
                    );
                    return response_deliver_tx_error_nonce_overflow(
                        sender_account.address().hex_with_prefix(),
                    );
                }
            };
            if expected_nonce != tx.nonce {
                warn!(
                    account_nonce = sender_account.nonce().value(),
                    tx_nonce = tx.nonce.value(),
                    "Nonce is not sequential"
                );
                return response_deliver_tx_error_nonce_not_sequential(
                    expected_nonce.value(),
                    tx.nonce.value(),
                    sender_account.address().hex_with_prefix(),
                );
            }

            let fee = match tx.fee.to_coin() {
                Ok(coin) => coin,
                Err(e) => {
                    let error_string = e.to_string();
                    handle_recoverable_eld_error(e);
                    return response_deliver_tx_error_invalid_coin_amount(error_string);
                }
            };

            // Validate fee amount
            if let Err(e) = validation::validate_fee_amount(&fee) {
                error!(fee = %fee, error = %e, "Fee validation failed");
                return response_deliver_tx_error_fee_validation_failed(e.to_string());
            }

            // Check sender has sufficient funds
            if sender_account.balance() < fee {
                warn!(
                    sender_balance = %sender_account.balance(),
                    required_fee = %fee,
                    "Sender has insufficient balance"
                );
                return response_deliver_tx_error_insufficient_funds();
            }

            // Update sender account nonce and deduct fee
            let updated_balance = match sender_account.balance() - fee {
                Ok(balance) => balance,
                Err(e) => {
                    let error_string = e.to_string();
                    handle_recoverable_eld_error(e);
                    return response_deliver_tx_error_invalid_coin_amount(error_string);
                }
            };

            let updated_sender = Account::new(sender_addr, updated_balance, expected_nonce);

            let sender_serialized = match bincode::serialize(&updated_sender) {
                Ok(serialized) => serialized,
                Err(e) => {
                    handle_recoverable_eld_error(EldError::StorageError {
                        operation: "serialization".to_string(),
                        details: e.to_string(),
                    });
                    return response_deliver_tx_error_internal_lock_failed(
                        "serialization failed".to_string(),
                    );
                }
            };
            let sender_meta = CADOMetadata::new(CadoType::Account, &sender_address);
            let sender_cado = CadoBody::mutable_updated(
                sender_account_with_hash.hash,
                sender_serialized,
                sender_meta,
            );

            current_state
                .envelope
                .update_cado_cache(sender_path, sender_cado);
        }

        tx.process(self).await
    }
}
