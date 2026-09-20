//! ABCI `Mempool` connection (`check_tx`).

use crate::api::lookup_cado_by_path_excluding_current_cache;
use crate::app_state::AppState;
use crate::config::FeeConfig;
use crate::errors::{
    response_check_tx_error_fee_calculation_failed, response_check_tx_error_insufficient_fee,
    response_check_tx_error_invalid_hex_encoding, response_check_tx_error_invalid_utf8_encoding,
    response_check_tx_error_json_parsing_failed, response_check_tx_error_nonce_not_sequential,
    response_check_tx_error_nonce_overflow, response_check_tx_error_sender_doesnt_exist,
    response_check_tx_error_tx_too_large, response_check_tx_error_verification_failed,
};
use crate::storage::traits::ConsensusConnectionStorage;
use abci::{async_api::Mempool, async_trait, types::*};
use eld_common::account::Account;
use eld_common::cado::{CadoBody, CadoPath, CadoPathKey, CadoType};
use eld_common::tx::HasSender;
use eld_common::tx::Tx;
use eld_common::validation::safe_deserialize_account_data;
use std::sync::{Arc, Mutex};
use tracing::warn;

#[derive(Debug)]
pub struct MempoolConnection<S>
where
    S: ConsensusConnectionStorage,
{
    chain_id: String,
    max_tx_bytes: usize,
    fee_config: FeeConfig,
    committed_state: Arc<Mutex<AppState>>,
    storage: Arc<S>,
}

impl<S> MempoolConnection<S>
where
    S: ConsensusConnectionStorage,
{
    pub fn new(
        chain_id: String,
        max_tx_bytes: usize,
        fee_config: FeeConfig,
        committed_state: Arc<Mutex<AppState>>,
        storage: Arc<S>,
    ) -> Self {
        Self {
            chain_id,
            max_tx_bytes,
            fee_config,
            committed_state,
            storage,
        }
    }
}

#[async_trait]
impl<S> Mempool for MempoolConnection<S>
where
    S: ConsensusConnectionStorage,
{
    // Doesn't set app state current or committed - only validates against committed head.
    // If tx passes check_tx, it is included in the mempool
    async fn check_tx(&self, check_tx_request: RequestCheckTx) -> ResponseCheckTx {
        let tx_bytes = check_tx_request.tx;

        // Check transaction size first (before expensive JSON parsing)
        if tx_bytes.len() > self.max_tx_bytes {
            warn!(
                tx_bytes = tx_bytes.len(),
                max_tx_bytes = self.max_tx_bytes,
                "mempool check_tx rejected: tx too large"
            );
            return response_check_tx_error_tx_too_large(self.max_tx_bytes, tx_bytes.len());
        }

        let hex_str = match String::from_utf8(tx_bytes) {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    error = %e,
                    "mempool check_tx rejected: invalid UTF-8 encoding"
                );
                return response_check_tx_error_invalid_utf8_encoding(e.to_string());
            }
        };

        let decoded_bytes = match hex::decode(&hex_str) {
            Ok(bytes) => bytes,
            Err(e) => {
                warn!(
                    error = %e,
                    "mempool check_tx rejected: invalid hex encoding"
                );
                return response_check_tx_error_invalid_hex_encoding(e.to_string());
            }
        };

        let decoded_json = match String::from_utf8(decoded_bytes) {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    error = %e,
                    "mempool check_tx rejected: decoded bytes are not valid UTF-8"
                );
                return response_check_tx_error_invalid_utf8_encoding(e.to_string());
            }
        };

        let tx: Tx = match serde_json::from_str(&decoded_json) {
            Ok(tx) => tx,
            Err(e) => {
                warn!(
                    error = %e,
                    "mempool check_tx rejected: failed to parse transaction JSON"
                );
                return response_check_tx_error_json_parsing_failed(e.to_string());
            }
        };

        // Verify transaction signature
        let verification_result = tx.verify(&self.chain_id).unwrap_or(false);
        if !verification_result {
            warn!(
                nonce = %tx.nonce,
                tx_type = %tx.payload.r#type,
                sender = %tx.payload.inner.sender(),
                "mempool check_tx rejected: signature verification failed"
            );
            return response_check_tx_error_verification_failed();
        }

        let sender_addr = tx.payload.inner.sender();
        let sender_address = sender_addr.to_string();
        let sender_path = match CadoPath::new(CadoType::Account, CadoPathKey::Address(sender_addr))
        {
            Ok(path) => path,
            Err(e) => {
                warn!(
                    nonce = %tx.nonce,
                    tx_type = %tx.payload.r#type,
                    sender = %sender_address,
                    error = %e,
                    "mempool check_tx rejected: invalid sender account path"
                );
                return response_check_tx_error_verification_failed();
            }
        };

        let sender_account = match lookup_cado_by_path_excluding_current_cache(
            &self.committed_state,
            self.storage.as_ref(),
            &sender_path,
        ) {
            Ok(Some(CadoBody::Mutable(cado_mut))) => {
                match safe_deserialize_account_data::<Account>(
                    cado_mut.data(),
                    &format!("account at {}", sender_path.as_str()),
                ) {
                    Ok(account) => account,
                    Err(e) => {
                        warn!(
                            nonce = %tx.nonce,
                            tx_type = %tx.payload.r#type,
                            sender = %sender_address,
                            error = %e,
                            "mempool check_tx rejected: failed to deserialize sender account"
                        );
                        return response_check_tx_error_verification_failed();
                    }
                }
            }
            Ok(Some(_)) => {
                warn!(
                    nonce = %tx.nonce,
                    tx_type = %tx.payload.r#type,
                    sender = %sender_address,
                    "mempool check_tx rejected: sender account CADO is not mutable"
                );
                return response_check_tx_error_verification_failed();
            }
            Ok(None) => {
                warn!(
                    nonce = %tx.nonce,
                    tx_type = %tx.payload.r#type,
                    sender = %sender_address,
                    "mempool check_tx rejected: sender account not found"
                );
                return response_check_tx_error_sender_doesnt_exist(sender_address);
            }
            Err(e) => {
                warn!(
                    nonce = %tx.nonce,
                    tx_type = %tx.payload.r#type,
                    sender = %sender_address,
                    error = %e,
                    "mempool check_tx rejected: sender account lookup failed"
                );
                return response_check_tx_error_verification_failed();
            }
        };

        // Must match deliver_tx: exact next nonce only (`account.nonce().next()`).
        let expected_nonce = match sender_account.nonce().next() {
            Some(nonce) => nonce,
            None => {
                warn!(
                    nonce = %tx.nonce,
                    tx_type = %tx.payload.r#type,
                    sender = %sender_address,
                    "mempool check_tx rejected: account nonce overflow"
                );
                return response_check_tx_error_nonce_overflow(
                    sender_account.address().hex_with_prefix(),
                );
            }
        };
        if expected_nonce != tx.nonce {
            warn!(
                nonce = %tx.nonce,
                tx_type = %tx.payload.r#type,
                sender = %sender_address,
                expected = expected_nonce.value(),
                "mempool check_tx rejected: nonce not sequential"
            );
            return response_check_tx_error_nonce_not_sequential(
                expected_nonce.value(),
                tx.nonce.value(),
                sender_account.address().hex_with_prefix(),
            );
        }

        // Validate dynamic fee
        let required_fee = match eld_common::fee::calculate_dynamic_fee(&tx, &self.fee_config) {
            Ok(fee) => fee,
            Err(e) => {
                warn!(
                    nonce = %tx.nonce,
                    tx_type = %tx.payload.r#type,
                    sender = %sender_address,
                    error = %e,
                    "mempool check_tx rejected: fee calculation failed"
                );
                return response_check_tx_error_fee_calculation_failed(e.to_string());
            }
        };

        let provided_fee = match tx.fee.to_coin() {
            Ok(coin) => coin,
            Err(e) => {
                warn!(
                    nonce = %tx.nonce,
                    tx_type = %tx.payload.r#type,
                    sender = %sender_address,
                    error = %e,
                    "mempool check_tx rejected: provided fee is not a valid coin"
                );
                return response_check_tx_error_insufficient_fee(
                    required_fee.amount(),
                    tx.fee.as_u128(),
                );
            }
        };

        // Validate fee against required dynamic fee.
        if provided_fee < required_fee {
            warn!(
                nonce = %tx.nonce,
                tx_type = %tx.payload.r#type,
                sender = %sender_address,
                required = %required_fee.amount(),
                provided = %provided_fee.amount(),
                "mempool check_tx rejected: insufficient fee"
            );
            return response_check_tx_error_insufficient_fee(
                required_fee.amount(),
                tx.fee.as_u128(),
            );
        }

        ResponseCheckTx {
            data: vec![],
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FeeConfig;
    use crate::storage::hybrid_storage::HybridStorage;
    use crate::storage::rocksdb::RocksDBStorage;
    use ed25519_dalek::SigningKey;
    use eld_common::account::Account;
    use eld_common::address::Address;
    use eld_common::cado::{CADOMetadata, CadoBody, CadoPath, CadoPathKey, CadoType};
    use eld_common::coin::Coin;
    use eld_common::fee::calculate_dynamic_fee;
    use eld_common::nonce::Nonce;
    use eld_common::tx::{Payload, TransferTx, Tx, TxPublicKey, TxSig};
    use sha2::{Digest, Sha256};
    use std::sync::Arc;
    use tempfile::TempDir;

    fn seed_committed_account(
        state: &mut AppState,
        signing_key: &SigningKey,
        balance: u128,
        nonce: u32,
    ) {
        if state.app_hash.is_empty() {
            state.app_hash = Sha256::digest("genesis").to_vec();
        }
        state.chain_id = "test-chain".to_string();

        let sender =
            Address::from_public_key(&signing_key.verifying_key()).expect("derive address in test");
        let account = Account::new(
            sender,
            Coin::new(balance).expect("balance"),
            Nonce::new(nonce),
        );
        let serialized = bincode::serialize(&account).expect("serialize account");
        let sender_str = sender.to_string();
        let cado = CadoBody::mutable_new(
            serialized,
            CADOMetadata::new(CadoType::Account, &sender_str),
        );
        let path =
            CadoPath::new(CadoType::Account, CadoPathKey::Address(sender)).expect("account path");
        state.envelope.update_cado_cache(path.clone(), cado.clone());
        state
            .envelope
            .committed_cado_cache
            .insert(path.as_str().as_bytes(), cado);
    }

    fn signed_transfer_tx(
        signing_key: &SigningKey,
        recipient: Address,
        nonce: u32,
        chain_id: &str,
    ) -> Tx {
        let sender =
            Address::from_public_key(&signing_key.verifying_key()).expect("derive address in test");
        let inner = TransferTx::new(sender, recipient, 1.into()).expect("valid transfer");
        let mut tx = Tx {
            sig: TxSig::empty(),
            nonce: nonce.into(),
            payload: Payload::new(inner),
            public_key: TxPublicKey::from(signing_key.verifying_key()),
            fee: 0.into(),
        };
        let fee_config = FeeConfig::default();
        let required_fee = calculate_dynamic_fee(&tx, &fee_config).expect("fee estimate");
        tx.fee = required_fee.amount().into();
        tx.sign(signing_key, chain_id).expect("sign");
        tx
    }

    fn hex_encode_tx(tx: &Tx) -> Vec<u8> {
        hex::encode(serde_json::to_string(tx).expect("serialize tx")).into_bytes()
    }

    #[tokio::test]
    async fn check_tx_rejects_non_sequential_nonce() {
        let temp_dir = TempDir::new().unwrap();
        let rocksdb_storage = Arc::new(RocksDBStorage::new(temp_dir.path()).unwrap());
        let storage = Arc::new(HybridStorage::new(rocksdb_storage));
        let committed_state = Arc::new(Mutex::new(AppState::default()));

        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        {
            let mut state = committed_state.lock().expect("committed lock");
            seed_committed_account(&mut state, &signing_key, 10_000, 0);
        }

        let recipient = Address::parse_hex_str("0xabcdef1234567890abcdef1234567890abcdef12")
            .expect("recipient");
        let bad_nonce_tx = signed_transfer_tx(&signing_key, recipient, 2, "test-chain");
        let good_nonce_tx = signed_transfer_tx(&signing_key, recipient, 1, "test-chain");

        let mempool = MempoolConnection::new(
            "test-chain".to_string(),
            1024 * 1024,
            FeeConfig::default(),
            committed_state,
            storage,
        );

        let bad_response = mempool
            .check_tx(RequestCheckTx {
                tx: hex_encode_tx(&bad_nonce_tx),
                ..Default::default()
            })
            .await;
        assert_eq!(bad_response.code, 3);

        let good_response = mempool
            .check_tx(RequestCheckTx {
                tx: hex_encode_tx(&good_nonce_tx),
                ..Default::default()
            })
            .await;
        assert_eq!(good_response.code, 0);
    }

    #[tokio::test]
    async fn check_tx_rejects_nonce_overflow() {
        let temp_dir = TempDir::new().unwrap();
        let rocksdb_storage = Arc::new(RocksDBStorage::new(temp_dir.path()).unwrap());
        let storage = Arc::new(HybridStorage::new(rocksdb_storage));
        let committed_state = Arc::new(Mutex::new(AppState::default()));

        let signing_key = SigningKey::from_bytes(&[9u8; 32]);
        {
            let mut state = committed_state.lock().expect("committed lock");
            seed_committed_account(&mut state, &signing_key, 10_000, u32::MAX);
        }

        let recipient = Address::parse_hex_str("0xabcdef1234567890abcdef1234567890abcdef12")
            .expect("recipient");
        let overflow_tx = signed_transfer_tx(&signing_key, recipient, 0, "test-chain");

        let mempool = MempoolConnection::new(
            "test-chain".to_string(),
            1024 * 1024,
            FeeConfig::default(),
            committed_state,
            storage,
        );

        let response = mempool
            .check_tx(RequestCheckTx {
                tx: hex_encode_tx(&overflow_tx),
                ..Default::default()
            })
            .await;
        assert_eq!(response.code, 6);
    }

    #[tokio::test]
    async fn check_tx_resolves_account_nonce_from_db_when_not_in_committed_cache() {
        let temp_dir = TempDir::new().unwrap();
        let rocksdb_storage = Arc::new(RocksDBStorage::new(temp_dir.path()).unwrap());
        let storage = Arc::new(HybridStorage::new(rocksdb_storage.clone()));
        let committed_state = Arc::new(Mutex::new(AppState::default()));

        let signing_key = SigningKey::from_bytes(&[8u8; 32]);
        let sender =
            Address::from_public_key(&signing_key.verifying_key()).expect("derive address in test");
        let account = Account::new(sender, Coin::new(10_000).expect("balance"), Nonce::new(0));
        let serialized = bincode::serialize(&account).expect("serialize account");
        let sender_str = sender.to_string();
        let cado = CadoBody::mutable_new(
            serialized,
            CADOMetadata::new(CadoType::Account, &sender_str),
        );
        let path =
            CadoPath::new(CadoType::Account, CadoPathKey::Address(sender)).expect("account path");
        rocksdb_storage
            .put_cado_type(path, cado)
            .expect("persist account to db");

        let recipient = Address::parse_hex_str("0xabcdef1234567890abcdef1234567890abcdef12")
            .expect("recipient");
        let tx = signed_transfer_tx(&signing_key, recipient, 1, "test-chain");

        let mempool = MempoolConnection::new(
            "test-chain".to_string(),
            1024 * 1024,
            FeeConfig::default(),
            committed_state,
            storage,
        );

        let response = mempool
            .check_tx(RequestCheckTx {
                tx: hex_encode_tx(&tx),
                ..Default::default()
            })
            .await;
        assert_eq!(response.code, 0);
    }

    #[tokio::test]
    async fn check_tx_rejects_malformed_encoding_without_panic() {
        let temp_dir = TempDir::new().unwrap();
        let rocksdb_storage = Arc::new(RocksDBStorage::new(temp_dir.path()).unwrap());
        let storage = Arc::new(HybridStorage::new(rocksdb_storage));
        let committed_state = Arc::new(Mutex::new(AppState::default()));

        let mempool = MempoolConnection::new(
            "test-chain".to_string(),
            1024 * 1024,
            FeeConfig::default(),
            committed_state,
            storage,
        );

        let invalid_utf8 = mempool
            .check_tx(RequestCheckTx {
                tx: vec![0xff, 0xfe, 0xfd],
                ..Default::default()
            })
            .await;
        assert_eq!(invalid_utf8.code, 76);

        let invalid_hex = mempool
            .check_tx(RequestCheckTx {
                tx: b"not-valid-hex".to_vec(),
                ..Default::default()
            })
            .await;
        assert_eq!(invalid_hex.code, 77);

        let invalid_json = mempool
            .check_tx(RequestCheckTx {
                tx: hex::encode(b"{not json}").into_bytes(),
                ..Default::default()
            })
            .await;
        assert_eq!(invalid_json.code, 78);
    }
}
