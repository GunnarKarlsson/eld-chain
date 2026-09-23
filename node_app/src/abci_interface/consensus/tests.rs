use super::connection::ConsensusConnectionNewContext;
use super::rewards::ValidatorRewardManager;
use crate::abci_interface::chain_tip::ChainTip;
use crate::abci_interface::mempool::MempoolConnection;
use crate::abci_interface::snapshot::SnapshotManager;
use crate::abci_interface::ConsensusConnection;
use crate::app_state::AppState;
use crate::capacity::capacity_manager::CapacityManager;
use crate::config::ConsensusConfig;
use crate::config::FeeConfig;
use crate::content::sync::{MockP2pSyncCoordinator, P2pConfig, P2pSyncCoordinator};
use crate::node_identity::LocalNodeIdentity;
use crate::storage::hybrid_storage::HybridStorage;
use crate::storage::rocksdb::RocksDBStorage;
use crate::wallet::VerifiedProofChainSubmitter;
use abci::async_api::{Consensus, Mempool};
use abci::types::{
    Header, PublicKey, RequestBeginBlock, RequestCheckTx, RequestCommit, RequestDeliverTx,
    RequestEndBlock, ValidatorUpdate,
};
use ed25519_dalek::SigningKey;
use eld_client::config::ClientConfig;
use eld_common::account::Account;
use eld_common::address::Address;
use eld_common::cado::{CadoPath, CadoPathKey, CadoType};
use eld_common::capacity::CapacityConfig;
use eld_common::coin::Coin;
use eld_common::constants::protocol::BLOCKS_PER_EPOCH;
use eld_common::fee::calculate_dynamic_fee;
use eld_common::nonce::Nonce;
use eld_common::tx::{Payload, TransferTx, Tx, TxPublicKey, TxSig};
use eld_common::validation::safe_deserialize_account_data;
use std::sync::{Arc, Mutex, RwLock};
use tempfile::TempDir;

#[cfg(test)]
fn mock_consensus_connection() -> ConsensusConnection<HybridStorage> {
    let config = ConsensusConfig {
        chain_id: "test-chain".to_string(),
        app_host: "127.0.0.1".to_string(),
        app_port: "8080".to_string(),
        accounts: Default::default(),
        max_tx_bytes: 1024 * 1024,
        fee_config: FeeConfig::default(),
        storage_limits: crate::config::StorageLimits::default(),
    };
    let consensus_config = Arc::new(Mutex::new(config));
    let committed_state = Arc::new(Mutex::new(AppState::default()));
    let current_state = Arc::new(Mutex::new(Some(AppState::default())));
    let rocksdb_dir = tempfile::TempDir::new().unwrap();
    let capacity_dir = rocksdb_dir.path().join("capacity");
    std::fs::create_dir_all(&capacity_dir).unwrap();
    let rocksdb_storage = Arc::new(RocksDBStorage::new(rocksdb_dir.path()).unwrap());
    let storage = Arc::new(HybridStorage::new(rocksdb_storage.clone()));
    let snapshot_manager = Arc::new(SnapshotManager::new(storage.clone()));
    let _reward_manager = ValidatorRewardManager::new(storage.clone());

    // Create a mock P2P sync coordinator for tests
    // Use a separate thread with its own runtime to avoid nested runtime issues
    // Use random ports to avoid conflicts between parallel tests
    use std::sync::atomic::{AtomicU16, Ordering};
    static PORT_COUNTER: AtomicU16 = AtomicU16::new(5000);
    let tcp_port = PORT_COUNTER.fetch_add(2, Ordering::Relaxed);
    let udp_port = tcp_port + 1;

    let p2p_config = P2pConfig { tcp_port, udp_port };
    // Generate a random keypair for tests
    let test_keypair = libp2p::identity::Keypair::generate_ed25519();
    let storage_clone = storage.clone();
    let cli_config = ClientConfig {
        node_host: "127.0.0.1".to_string(),
        node_port: "26657".to_string(),
        chain_id: "test-chain".to_string(),
        faucet_host: "127.0.0.1".to_string(),
        faucet_port: "8080".to_string(),
        faucet_end_point: "/faucet/request".to_string(),
        faucet_url: None,
        app_port: "9001".to_string(),
        node_url: None,
        app_url: None,
    };
    let cli = Arc::new(eld_client::facade::ChainClient::new(
        cli_config,
        eld_common::fee::FeeConfig::default(),
    ));
    let verified_proof_submitter = Arc::new(VerifiedProofChainSubmitter::new(
        "wallet1".into(),
        cli.clone(),
        consensus_config.clone(),
        storage.clone(),
    ));
    let local_identity = Arc::new(RwLock::new(LocalNodeIdentity::default()));
    let local_identity_for_p2p = local_identity.clone();
    let (p2p_sync_coordinator, _rx) = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(P2pSyncCoordinator::new(
            p2p_config,
            test_keypair,
            storage_clone,
            verified_proof_submitter,
            local_identity_for_p2p,
        ))
        .map_err(|e| format!("Failed to create P2P coordinator: {e}"))
    })
    .join()
    .map_err(|e| format!("Thread join error: {e:?}"))
    .unwrap()
    .unwrap();
    let p2p_sync_coordinator = Arc::new(p2p_sync_coordinator);
    let chain_tip = Arc::new(ChainTip::default());

    use eld_common::address::Address;

    let test_provider =
        Address::parse_hex_str("0xcccccccccccccccccccccccccccccccccccccccc").expect("provider");
    let capacity_config = CapacityConfig {
        capacity_dir,
        max_capacity_gb: 10,
        provider_id: test_provider,
        auto_register: true,
        registration_retry_interval_secs: 60,
        tendermint_rpc_url: "http://127.0.0.1:26657".to_string(),
    };
    let capacity_manager = Arc::new(CapacityManager::new(
        capacity_config,
        "wallet1".to_string(),
        cli,
        consensus_config.clone(),
    ));

    ConsensusConnection::new(ConsensusConnectionNewContext {
        consensus_config,
        committed_state,
        chain_tip,
        current_state,
        storage,
        snapshot_manager,
        p2p_sync_coordinator,
        ready_tx: None,
        capacity_manager,
        local_identity,
    })
}

#[tokio::test]
async fn test_deliver_tx_invalid_utf8() {
    let consensus = mock_consensus_connection();
    let invalid_tx = vec![0xFF, 0xFF]; // Invalid UTF-8
    let response = consensus
        .deliver_tx(RequestDeliverTx { tx: invalid_tx })
        .await;
    assert_eq!(response.code, 62); // response_deliver_tx_error_invalid_utf8_encoding
    assert!(response.log.contains("Invalid UTF-8 encoding"));
}

#[tokio::test]
async fn test_deliver_tx_invalid_hex() {
    let consensus = mock_consensus_connection();
    let invalid_hex = String::from("invalid_hex").into_bytes();
    let response = consensus
        .deliver_tx(RequestDeliverTx { tx: invalid_hex })
        .await;
    assert_eq!(response.code, 71); // response_deliver_tx_error_hex_validation_failed (now happens before hex decoding)
    assert!(response.log.contains("Hex validation failed"));
}

#[tokio::test]
async fn test_deliver_tx_hex_validation_failed() {
    let consensus = mock_consensus_connection();
    // Test hex string with odd length (invalid hex)
    let odd_length_hex = "aaa"; // 3 characters - odd length
    let response = consensus
        .deliver_tx(RequestDeliverTx {
            tx: odd_length_hex.as_bytes().to_vec(),
        })
        .await;
    assert_eq!(response.code, 71); // response_deliver_tx_error_hex_validation_failed
    assert!(response.log.contains("Hex validation failed"));
}

#[tokio::test]
async fn test_deliver_tx_transaction_structure_invalid() {
    let consensus = mock_consensus_connection();
    // Test invalid transaction structure (missing required fields)
    let invalid_tx_json = r#"{"sig": "test", "nonce": 1}"#; // Missing payload, public_key, fee
    let hex_encoded = hex::encode(invalid_tx_json.as_bytes());
    let response = consensus
        .deliver_tx(RequestDeliverTx {
            tx: hex_encoded.into_bytes(),
        })
        .await;
    assert_eq!(response.code, 73); // response_deliver_tx_error_transaction_structure_invalid
    assert!(response.log.contains("Transaction structure invalid"));
}

#[tokio::test]
async fn test_deliver_tx_deeply_nested_json() {
    let consensus_connection = mock_consensus_connection();

    // Create a deeply nested JSON structure that exceeds the limit
    let deeply_nested =
        r#"{"a":{"b":{"c":{"d":{"e":{"f":{"g":{"h":{"i":{"j":{"k":{"l":"v"}}}}}}}}}}}"#;
    let hex_encoded = hex::encode(deeply_nested.as_bytes());

    let request = RequestDeliverTx {
        tx: hex_encoded.as_bytes().to_vec(),
    };

    let response = consensus_connection.deliver_tx(request).await;
    assert_ne!(response.code, 0, "Deeply nested JSON should be rejected");
}

#[cfg(test)]
fn seed_sender_account(
    state: &mut AppState,
    signing_key: &ed25519_dalek::SigningKey,
    balance: u128,
) {
    use eld_common::cado::{CADOMetadata, CadoBody, CadoPath, CadoPathKey, CadoType};
    use sha2::{Digest, Sha256};

    if state.app_hash().is_empty() {
        state.set_app_hash(Sha256::digest("genesis").into());
    }
    if state.chain_id.is_empty() {
        state.chain_id = "test-chain".to_string();
    }

    let sender =
        Address::from_public_key(&signing_key.verifying_key()).expect("derive address in test");
    let account = Account::new(sender, Coin::new(balance).expect("balance"), Nonce::new(0));
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

#[cfg(test)]
fn signed_add_namespace_tx(
    signing_key: &ed25519_dalek::SigningKey,
    slug: &str,
    nonce: u32,
    chain_id: &str,
) -> Tx {
    use crate::config::FeeConfig;
    use eld_common::fee::calculate_dynamic_fee;
    use eld_common::tx::{AddNamespaceTx, Payload, TxPublicKey, TxSig};

    let sender =
        Address::from_public_key(&signing_key.verifying_key()).expect("derive address in test");
    let inner =
        AddNamespaceTx::new(sender, slug.to_string(), 1.into()).expect("valid AddNamespaceTx");
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
    assert!(tx.verify(chain_id).expect("verify"));
    assert_eq!(
        tx.payload.r#type,
        eld_common::constants::tx_type::TX_TYPE_ADD_NAMESPACE
    );
    tx
}

#[cfg(test)]
fn hex_encode_tx(tx: &Tx) -> Vec<u8> {
    hex::encode(serde_json::to_string(tx).expect("serialize tx")).into_bytes()
}

#[tokio::test]
async fn test_add_namespace_commit_flushes_registry_to_cache_and_trie() {
    let consensus = mock_consensus_connection();
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]);
    let chain_id = "test-chain";

    {
        let mut current = consensus.current_state.lock().expect("current lock");
        seed_sender_account(
            current.as_mut().expect("current state"),
            &signing_key,
            10_000_000,
        );
    }
    {
        let mut committed = consensus.committed_state.lock().expect("committed lock");
        committed.chain_id = chain_id.to_string();
        let mut current = consensus.current_state.lock().expect("current lock");
        current.as_mut().expect("current state").chain_id = chain_id.to_string();
    }

    let trie_root_before = {
        let current = consensus.current_state.lock().expect("current lock");
        current
            .as_ref()
            .expect("current state")
            .envelope
            .state_trie
            .root_hash()
    };

    let tx = signed_add_namespace_tx(&signing_key, "peter", 1, chain_id);
    let deliver = consensus
        .deliver_tx(RequestDeliverTx {
            tx: hex_encode_tx(&tx),
        })
        .await;
    assert_eq!(deliver.code, 0, "deliver: {}", deliver.log);

    {
        let current = consensus.current_state.lock().expect("current lock");
        let state = current.as_ref().expect("current state");
        assert!(state
            .envelope
            .namespace_registry_cache
            .contains_key("peter"));
        let path = eld_common::namespace::slug_to_namespace_cadopath("peter").expect("path");
        assert!(
            state
                .envelope
                .committed_cado_cache
                .get(path.as_str().as_bytes())
                .is_none(),
            "not committed until commit()"
        );
    }

    consensus.commit(RequestCommit::default()).await;

    let committed = consensus.committed_state.lock().expect("committed lock");
    assert!(committed.envelope.namespace_registry_cache.is_empty());
    let path = eld_common::namespace::slug_to_namespace_cadopath("peter").expect("path");
    assert!(committed
        .envelope
        .committed_cado_cache
        .get(path.as_str().as_bytes())
        .is_some());
    assert!(committed
        .envelope
        .is_namespace_registered("peter")
        .expect("check"));
    assert_ne!(
        committed.envelope.state_trie.root_hash(),
        trie_root_before,
        "trie root should change after namespace registry insert"
    );
}

#[tokio::test]
async fn test_add_namespace_duplicate_slug_same_block_rejected() {
    let consensus = mock_consensus_connection();
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&[4u8; 32]);
    let chain_id = "test-chain";

    {
        let mut current = consensus.current_state.lock().expect("current lock");
        seed_sender_account(
            current.as_mut().expect("current state"),
            &signing_key,
            10_000_000,
        );
        current.as_mut().expect("current state").chain_id = chain_id.to_string();
    }
    {
        let mut committed = consensus.committed_state.lock().expect("committed lock");
        committed.chain_id = chain_id.to_string();
    }

    let tx1 = signed_add_namespace_tx(&signing_key, "peter", 1, chain_id);
    assert_eq!(
        consensus
            .deliver_tx(RequestDeliverTx {
                tx: hex_encode_tx(&tx1),
            })
            .await
            .code,
        0
    );

    let tx2 = signed_add_namespace_tx(&signing_key, "peter", 2, chain_id);
    let deliver2 = consensus
        .deliver_tx(RequestDeliverTx {
            tx: hex_encode_tx(&tx2),
        })
        .await;
    assert_eq!(deliver2.code, 51, "Layer A: {}", deliver2.log);
    assert!(deliver2.log.contains("already registered"));
}

#[tokio::test]
async fn test_add_namespace_duplicate_slug_next_block_rejected() {
    let consensus = mock_consensus_connection();
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&[5u8; 32]);
    let chain_id = "test-chain";

    {
        let mut current = consensus.current_state.lock().expect("current lock");
        seed_sender_account(
            current.as_mut().expect("current state"),
            &signing_key,
            10_000_000,
        );
        current.as_mut().expect("current state").chain_id = chain_id.to_string();
    }
    {
        let mut committed = consensus.committed_state.lock().expect("committed lock");
        committed.chain_id = chain_id.to_string();
    }

    let tx1 = signed_add_namespace_tx(&signing_key, "peter", 1, chain_id);
    assert_eq!(
        consensus
            .deliver_tx(RequestDeliverTx {
                tx: hex_encode_tx(&tx1),
            })
            .await
            .code,
        0
    );
    consensus.commit(RequestCommit::default()).await;
    {
        let mut committed = consensus.committed_state.lock().expect("committed lock");
        committed.chain_id = chain_id.to_string();
    }

    let tx2 = signed_add_namespace_tx(&signing_key, "peter", 2, chain_id);
    let deliver2 = consensus
        .deliver_tx(RequestDeliverTx {
            tx: hex_encode_tx(&tx2),
        })
        .await;
    assert_eq!(deliver2.code, 51, "Layer B: {}", deliver2.log);
    assert!(deliver2.log.contains("already registered"));
}

#[tokio::test]
#[allow(clippy::needless_update)]
async fn init_chain_stages_validator_accounts_until_first_commit() {
    use crate::storage::traits::CADOStorage;
    use abci::async_api::Consensus;
    use abci::types::{RequestBeginBlock, RequestCommit, RequestEndBlock, RequestInitChain, Sum};
    use eld_common::cado::{CadoPath, CadoPathKey, CadoType};

    let consensus = mock_consensus_connection();
    let storage = consensus.storage.clone();
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
    let validator_addr =
        Address::from_public_key(&signing_key.verifying_key()).expect("validator address");
    let account_path =
        CadoPath::new(CadoType::Account, CadoPathKey::Address(validator_addr)).expect("path");

    assert!(
        storage
            .get_cado_by_path(account_path.clone())
            .expect("db read")
            .is_none(),
        "validator account must not exist in db before init_chain"
    );

    consensus
        .init_chain(RequestInitChain {
            chain_id: "test-chain".into(),
            validators: vec![ValidatorUpdate {
                pub_key: Some(PublicKey {
                    sum: Some(Sum::Ed25519(
                        signing_key.verifying_key().to_bytes().to_vec(),
                    )),
                }),
                power: 100,
            }],
            ..Default::default()
        })
        .await;

    {
        let committed = consensus.committed_state.lock().expect("committed lock");
        assert!(
            committed
                .envelope
                .cado_cache
                .contains_key(account_path.as_str()),
            "init_chain must stage validator account in cado_cache"
        );
        assert!(
            committed
                .envelope
                .committed_cado_cache
                .get(account_path.as_str().as_bytes())
                .is_none(),
            "committed_cado_cache must stay empty until commit"
        );
        assert_eq!(committed.envelope.validators.len(), 1);
    }

    assert!(
        storage
            .get_cado_by_path(account_path.clone())
            .expect("db read after init_chain")
            .is_none(),
        "init_chain must not persist validator accounts to db"
    );

    consensus
        .begin_block(RequestBeginBlock {
            header: Some(Header {
                height: 1,
                chain_id: "test-chain".into(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await;
    consensus
        .end_block(RequestEndBlock {
            height: 1,
            ..Default::default()
        })
        .await;
    consensus.commit(RequestCommit::default()).await;

    assert!(
        storage
            .get_cado_by_path(account_path.clone())
            .expect("db read after commit")
            .is_some(),
        "first commit must persist staged validator account"
    );
    {
        let committed = consensus.committed_state.lock().expect("committed lock");
        assert!(committed
            .envelope
            .committed_cado_cache
            .get(account_path.as_str().as_bytes())
            .is_some());
        assert!(!committed
            .envelope
            .cado_cache
            .contains_key(account_path.as_str()));
    }
}

#[test]
fn build_challenged_capacity_provider_data_skips_unregistered_providers() {
    use ed25519_dalek::SigningKey;
    use eld_common::address::Address;
    use eld_common::coin::Coin;
    use eld_common::public_key::PublicKey;
    use eld_common::validator::CapacityValidatorInfo;

    let registered = Address::parse_hex_str("0x2222222222222222222222222222222222222222")
        .expect("registered provider");
    let unregistered = Address::parse_hex_str("0x3333333333333333333333333333333333333333")
        .expect("unregistered provider");

    let mut state = AppState::default();
    state.envelope.capacity_validators = vec![CapacityValidatorInfo {
        address: registered,
        stake: Coin::zero(),
        public_key: PublicKey::from(SigningKey::from_bytes(&[7u8; 32]).verifying_key()),
        storage_capacity: 1_000,
        merkle_root: Some([9u8; 32]),
        seed: Some([8u8; 32]),
        chunk_count: Some(50),
        registered_at: Some(1),
        last_merkle_root_update: Some(1),
        registered_block: 1,
        registration_duration: 1000,
    }];

    let data = ConsensusConnection::<HybridStorage>::build_challenged_capacity_provider_data(
        &state,
        &[registered, unregistered],
    );

    assert_eq!(data.len(), 1);
    assert_eq!(data[0].0, registered);
    assert_eq!(data[0].1, Some([9u8; 32]));
    assert_eq!(data[0].2, Some([8u8; 32]));
    assert_eq!(data[0].3, Some(50));
}

#[cfg(test)]
const TRANSFER_CHAIN_ID: &str = "test-chain";
#[cfg(test)]
const TRANSFER_SENDER_SEED: [u8; 32] = [11u8; 32];
#[cfg(test)]
const TRANSFER_INITIAL_BALANCE: u128 = 10_000_000;

/// Test-only: keeps RocksDB alive and wires consensus without libp2p or Tendermint.
#[cfg(test)]
struct InProcessHarness {
    _rocksdb_dir: TempDir,
    consensus: ConsensusConnection<HybridStorage>,
    storage: Arc<HybridStorage>,
}

#[cfg(test)]
fn dummy_client_config() -> ClientConfig {
    ClientConfig {
        node_host: "127.0.0.1".to_string(),
        node_port: "26657".to_string(),
        chain_id: TRANSFER_CHAIN_ID.to_string(),
        faucet_host: "127.0.0.1".to_string(),
        faucet_port: "8080".to_string(),
        faucet_end_point: "/faucet/request".to_string(),
        faucet_url: None,
        app_port: "9001".to_string(),
        node_url: None,
        app_url: None,
    }
}

#[cfg(test)]
fn configured_local_identity() -> LocalNodeIdentity {
    LocalNodeIdentity {
        consensus_validator_address: Some(
            Address::parse_hex_str("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").expect("identity"),
        ),
        capacity_validator_address: None,
    }
}

#[cfg(test)]
fn in_process_harness() -> InProcessHarness {
    let config = ConsensusConfig {
        chain_id: TRANSFER_CHAIN_ID.to_string(),
        app_host: "127.0.0.1".to_string(),
        app_port: "8080".to_string(),
        accounts: Default::default(),
        max_tx_bytes: 1024 * 1024,
        fee_config: FeeConfig::default(),
        storage_limits: crate::config::StorageLimits::default(),
    };
    let consensus_config = Arc::new(Mutex::new(config));
    let committed_state = Arc::new(Mutex::new(AppState::default()));
    let current_state = Arc::new(Mutex::new(Some(AppState::default())));
    let rocksdb_dir = TempDir::new().expect("tempdir");
    let capacity_dir = rocksdb_dir.path().join("capacity");
    std::fs::create_dir_all(&capacity_dir).expect("capacity dir");
    let rocksdb_storage = Arc::new(RocksDBStorage::new(rocksdb_dir.path()).expect("rocksdb"));
    let storage = Arc::new(HybridStorage::new(rocksdb_storage));
    let snapshot_manager = Arc::new(SnapshotManager::new(storage.clone()));
    let (mock_p2p, _rx) = MockP2pSyncCoordinator::new();
    let cli = Arc::new(eld_client::facade::ChainClient::new(
        dummy_client_config(),
        eld_common::fee::FeeConfig::default(),
    ));
    let test_provider =
        Address::parse_hex_str("0xcccccccccccccccccccccccccccccccccccccccc").expect("provider");
    let capacity_config = CapacityConfig {
        capacity_dir,
        max_capacity_gb: 10,
        provider_id: test_provider,
        auto_register: true,
        registration_retry_interval_secs: 60,
        tendermint_rpc_url: "http://127.0.0.1:26657".to_string(),
    };
    let capacity_manager = Arc::new(CapacityManager::new(
        capacity_config,
        "wallet1".to_string(),
        cli,
        consensus_config.clone(),
    ));
    let local_identity = Arc::new(RwLock::new(configured_local_identity()));

    let consensus = ConsensusConnection::new(ConsensusConnectionNewContext {
        consensus_config,
        committed_state,
        chain_tip: Arc::new(ChainTip::default()),
        current_state,
        storage: storage.clone(),
        snapshot_manager,
        p2p_sync_coordinator: Arc::new(mock_p2p),
        ready_tx: None,
        capacity_manager,
        local_identity,
    });

    InProcessHarness {
        _rocksdb_dir: rocksdb_dir,
        consensus,
        storage,
    }
}

#[cfg(test)]
fn seed_harness_sender(harness: &InProcessHarness, signing_key: &SigningKey) {
    {
        let mut committed = harness.consensus.committed_state.lock().expect("committed");
        seed_sender_account(&mut committed, signing_key, TRANSFER_INITIAL_BALANCE);
    }
    {
        let mut current = harness.consensus.current_state.lock().expect("current");
        seed_sender_account(
            current.as_mut().expect("current state"),
            signing_key,
            TRANSFER_INITIAL_BALANCE,
        );
    }
}

#[cfg(test)]
fn sender_account_path(signing_key: &SigningKey) -> CadoPath {
    let sender =
        Address::from_public_key(&signing_key.verifying_key()).expect("derive address in test");
    CadoPath::new(CadoType::Account, CadoPathKey::Address(sender)).expect("account path")
}

#[cfg(test)]
fn account_from_committed(state: &AppState, path: &CadoPath) -> Account {
    let cado = state
        .envelope
        .committed_cado_cache
        .get(path.as_str().as_bytes())
        .expect("sender in committed cache");
    safe_deserialize_account_data::<Account>(cado.data(), "test_account").expect("account")
}

#[cfg(test)]
fn signed_transfer_tx(signing_key: &SigningKey, recipient: Address, nonce: u32) -> Tx {
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
    let required_fee = calculate_dynamic_fee(&tx, &FeeConfig::default()).expect("fee estimate");
    tx.fee = required_fee.amount().into();
    tx.sign(signing_key, TRANSFER_CHAIN_ID).expect("sign");
    tx
}

#[cfg(test)]
fn transfer_recipient() -> Address {
    Address::parse_hex_str("0xabcdef1234567890abcdef1234567890abcdef12").expect("recipient")
}

#[cfg(test)]
fn snapshot_height() -> i64 {
    BLOCKS_PER_EPOCH
}

#[cfg(test)]
#[allow(clippy::needless_update)]
async fn begin_deliver_end_commit(
    consensus: &ConsensusConnection<HybridStorage>,
    tx_bytes: Vec<u8>,
) {
    let height = snapshot_height();
    consensus
        .begin_block(RequestBeginBlock {
            header: Some(Header {
                height,
                chain_id: TRANSFER_CHAIN_ID.into(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await;
    let deliver = consensus
        .deliver_tx(RequestDeliverTx { tx: tx_bytes })
        .await;
    assert_eq!(deliver.code, 0, "deliver_tx: {}", deliver.log);
    assert_eq!(deliver.gas_used, 21_000);
    consensus.end_block(RequestEndBlock { height }).await;
    consensus.commit(RequestCommit::default()).await;
}

#[tokio::test]
async fn check_tx_then_deliver_tx_transfer_commits_account() {
    let harness = in_process_harness();
    let signing_key = SigningKey::from_bytes(&TRANSFER_SENDER_SEED);
    seed_harness_sender(&harness, &signing_key);
    let path = sender_account_path(&signing_key);
    let tx = signed_transfer_tx(&signing_key, transfer_recipient(), 1);
    let tx_bytes = hex_encode_tx(&tx);

    let mempool = MempoolConnection::new(
        TRANSFER_CHAIN_ID.to_string(),
        1024 * 1024,
        FeeConfig::default(),
        harness.consensus.committed_state.clone(),
        harness.storage.clone(),
    );
    let check = mempool
        .check_tx(RequestCheckTx {
            tx: tx_bytes.clone(),
            ..Default::default()
        })
        .await;
    assert_eq!(check.code, 0, "check_tx: {}", check.log);

    {
        let committed = harness.consensus.committed_state.lock().expect("committed");
        let account = account_from_committed(&committed, &path);
        assert_eq!(account.nonce().value(), 0);
        assert_eq!(account.balance().amount(), TRANSFER_INITIAL_BALANCE);
    }

    begin_deliver_end_commit(&harness.consensus, tx_bytes).await;

    let committed = harness.consensus.committed_state.lock().expect("committed");
    let account = account_from_committed(&committed, &path);
    assert_eq!(account.nonce().value(), 1);
    assert!(
        account.balance().amount() < TRANSFER_INITIAL_BALANCE,
        "fee and transfer must reduce sender balance"
    );
}

#[tokio::test]
async fn transfer_app_hash_is_stable_and_reloads_from_rocksdb() {
    let signing_key = SigningKey::from_bytes(&TRANSFER_SENDER_SEED);
    let tx = signed_transfer_tx(&signing_key, transfer_recipient(), 1);
    let tx_bytes = hex_encode_tx(&tx);

    let harness_a = in_process_harness();
    seed_harness_sender(&harness_a, &signing_key);
    begin_deliver_end_commit(&harness_a.consensus, tx_bytes.clone()).await;
    let hash_a = *harness_a
        .consensus
        .committed_state
        .lock()
        .expect("committed")
        .app_hash();
    let trie_a = harness_a
        .consensus
        .committed_state
        .lock()
        .expect("committed")
        .envelope
        .state_trie
        .root_hash();

    let harness_b = in_process_harness();
    seed_harness_sender(&harness_b, &signing_key);
    begin_deliver_end_commit(&harness_b.consensus, tx_bytes).await;
    let hash_b = *harness_b
        .consensus
        .committed_state
        .lock()
        .expect("committed")
        .app_hash();
    let trie_b = harness_b
        .consensus
        .committed_state
        .lock()
        .expect("committed")
        .envelope
        .state_trie
        .root_hash();

    assert_eq!(
        hash_a, hash_b,
        "same transfer sequence must yield same app_hash"
    );
    assert_eq!(
        trie_a, trie_b,
        "same transfer sequence must yield same trie root"
    );
    assert!(!hash_a.is_empty());

    let mut restored = AppState::default();
    restored
        .initialize_with_data(harness_a.storage.as_ref())
        .expect("reload from rocksdb");
    assert_eq!(restored.app_hash(), &hash_a);
    assert_eq!(restored.envelope.state_trie.root_hash(), trie_a);
}
