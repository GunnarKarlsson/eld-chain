//! ABCI-facing connection implementations (deliver, query wiring).

pub mod chain_tip;
pub mod consensus;
pub mod info;
pub mod mempool;
pub mod snapshot;

pub use self::consensus::{ConsensusConnection, ConsensusConnectionNewContext};

use std::sync::{Arc, Mutex, RwLock};

use crate::node_identity::LocalNodeIdentity;

use abci::async_api::Server;
use eld_common::cado::{CADOMetadata, CadoBody, CadoPath, CadoPathKey, CadoType};
use eld_common::error::{EldError, ErrorBuilder};
use tokio::sync::oneshot;
use tracing::{error, info};

use crate::app_state::AppState;
use crate::capacity::capacity_manager::CapacityManager;
use crate::config::ConsensusConfig;
use crate::errors::handle_fatal_eld_error;
use crate::storage::traits::ConsensusConnectionStorage;
use chain_tip::ChainTip;
use info::InfoConnection;
use mempool::MempoolConnection;
use snapshot::{SnapshotConnection, SnapshotManager};

/// Inputs for [`init_abci_server`].
pub struct AbciServerInitContext<S>
where
    S: ConsensusConnectionStorage,
{
    pub consensus_config: Arc<Mutex<ConsensusConfig>>,
    pub storage: Arc<S>,
    pub snapshot_manager: Arc<SnapshotManager<S>>,
    pub p2p_sync_coordinator: Arc<dyn crate::content::sync::P2pCoordinatorTrait>,
    pub init_data: bool,
    pub ready_tx: Option<oneshot::Sender<()>>,
    pub capacity_manager: Arc<CapacityManager>,
    pub local_identity: Arc<RwLock<LocalNodeIdentity>>,
    pub committed_state: Arc<Mutex<AppState>>,
    pub current_state: Arc<Mutex<Option<AppState>>>,
}

pub async fn init_abci_server<S>(
    ctx: AbciServerInitContext<S>,
) -> Result<
    (
        Server<
            ConsensusConnection<S>,
            MempoolConnection<S>,
            InfoConnection<S>,
            SnapshotConnection<S>,
        >,
        Arc<ChainTip>,
    ),
    EldError,
>
where
    S: ConsensusConnectionStorage,
{
    let AbciServerInitContext {
        consensus_config,
        storage,
        snapshot_manager,
        p2p_sync_coordinator,
        init_data,
        ready_tx,
        capacity_manager,
        local_identity,
        committed_state: committed_state_mutex,
        current_state,
    } = ctx;

    let has_persisted_state = match AppState::has_persisted_app_state_tip(storage.as_ref()) {
        Ok(value) => value,
        Err(e) => handle_fatal_eld_error(e),
    };
    let fresh_start = !init_data && !has_persisted_state;

    if has_persisted_state && !init_data {
        info!("Persisted AppStateTip found in RocksDB; restoring committed state on startup");
    }

    let mut committed_state = match committed_state_mutex.lock() {
        Ok(state) => state,
        Err(e) => handle_fatal_eld_error(e.into()),
    };
    committed_state.envelope.init_empty_trie();

    if fresh_start {
        let chain_id = match consensus_config.lock() {
            Ok(config) => config.chain_id.clone(),
            Err(e) => handle_fatal_eld_error(e.into()),
        };

        committed_state.chain_id = chain_id;

        let accounts = match consensus_config.lock() {
            Ok(state) => state.accounts.clone(),
            Err(e) => handle_fatal_eld_error(e.into()),
        };

        let mut ordered_accounts: Vec<(&String, &eld_common::account::Account)> =
            accounts.iter().collect();
        ordered_accounts.sort_by(|(left_name, left_account), (right_name, right_account)| {
            left_account
                .address()
                .hex_with_prefix()
                .cmp(&right_account.address().hex_with_prefix())
                .then_with(|| left_name.cmp(right_name))
        });

        for (_name, account) in ordered_accounts {
            let serialized = bincode::serialize(&account)
                .map_err(|e| ErrorBuilder::storage_error("serialize account", &e.to_string()))?;

            let address_str = account.address().hex_with_prefix();

            let metadata = CADOMetadata::new(CadoType::Account, address_str.clone());

            let path_name =
                match CadoPath::new(CadoType::Account, CadoPathKey::Address(*account.address())) {
                    Ok(cado) => cado,
                    Err(e) => handle_fatal_eld_error(e),
                };
            let account_cado = CadoBody::mutable_new(serialized, metadata);
            committed_state
                .envelope
                .update_cado_cache(path_name, account_cado);
        }

        // Validators list and validator accounts are now set in init_chain
        // which is called by Tendermint with validators from the genesis file.
        // Genesis CADOs stay in cado_cache until the first commit(); app_hash is set in end_block.
    } else {
        match committed_state.initialize_with_data(storage.as_ref()) {
            Ok(_) => info!("Initialized consensus state from db"),
            Err(e) => {
                error!("Error initializing consensus from db: {}", e);
                handle_fatal_eld_error(e);
            }
        }
    }

    let (chain_id, max_tx_bytes, fee_config) = {
        let config = match consensus_config.lock() {
            Ok(config) => config,
            Err(e) => handle_fatal_eld_error(e.into()),
        };

        (
            config.chain_id.clone(),
            config.max_tx_bytes,
            config.fee_config.clone(),
        )
    };

    let chain_tip = Arc::new(ChainTip::from_app_state(&committed_state));
    drop(committed_state);

    let consensus = ConsensusConnection::new(ConsensusConnectionNewContext {
        consensus_config: consensus_config.clone(),
        committed_state: committed_state_mutex.clone(),
        chain_tip: chain_tip.clone(),
        current_state,
        storage: storage.clone(),
        snapshot_manager: snapshot_manager.clone(),
        p2p_sync_coordinator,
        ready_tx,
        capacity_manager,
        local_identity,
    });

    let mempool = MempoolConnection::new(
        chain_id,
        max_tx_bytes,
        fee_config,
        committed_state_mutex.clone(),
        storage.clone(),
    );
    let info = InfoConnection::new(
        committed_state_mutex.clone(),
        chain_tip.clone(),
        storage,
        consensus_config,
    );
    let snapshot = SnapshotConnection::new(snapshot_manager);

    Ok((Server::new(consensus, mempool, info, snapshot), chain_tip))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abci_interface::snapshot::SnapshotManager;
    use crate::app_state::AppState;
    use crate::capacity::capacity_manager::CapacityManager;
    use crate::config::FeeConfig;
    use crate::node_identity::LocalNodeIdentity;
    use crate::storage::hybrid_storage::HybridStorage;
    use crate::storage::rocksdb::RocksDBStorage;
    use crate::wallet::VerifiedProofChainSubmitter;
    use eld_client::config::CliConfig;
    use eld_common::capacity::CapacityConfig;
    use std::collections::HashMap;
    use std::sync::RwLock;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_init_abci_server_invalid_storage() {
        let temp_dir = TempDir::new().unwrap();
        let invalid_path = std::path::PathBuf::from("/invalid/rocksdb");

        // Test that RocksDBStorage::new fails with invalid path
        let rocksdb_result = RocksDBStorage::new(&invalid_path);
        assert!(rocksdb_result.is_err());

        // Test that init_abci_server fails when config file doesn't exist
        let valid_path = temp_dir.path().join("valid_rocksdb");
        let rocksdb_storage = Arc::new(RocksDBStorage::new(&valid_path).unwrap());
        let storage = Arc::new(HybridStorage::new(rocksdb_storage.clone()));

        // Create a minimal valid config manually
        let config = ConsensusConfig {
            chain_id: "test-chain".to_string(),
            app_host: "127.0.0.1".to_string(),
            app_port: "8080".to_string(),
            accounts: HashMap::new(),
            max_tx_bytes: 1024 * 1024, // 1MB
            fee_config: FeeConfig::default(),
            storage_limits: crate::config::StorageLimits::default(),
        };

        let cap_dir = temp_dir.path().join("capacity_abci_test");
        std::fs::create_dir_all(&cap_dir).unwrap();
        let consensus_config_arc = Arc::new(Mutex::new(config));
        let cli_config = CliConfig {
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
            p2p_tcp_port: None,
            p2p_udp_port: None,
            single_node: None,
            capacity_size_mb: None,
            capacity_storage_path: None,
            indexer: false,
        };
        let cli =
            Arc::new(eld_client::facade::ChainClient::new(cli_config).expect("test ChainClient"));
        let verified_proof_submitter = Arc::new(VerifiedProofChainSubmitter::new(
            "wallet1".into(),
            cli.clone(),
            consensus_config_arc.clone(),
            storage.clone(),
        ));
        let local_identity = Arc::new(RwLock::new(LocalNodeIdentity::default()));

        // Create a mock P2P sync coordinator for tests (unique ports to avoid conflicts)
        use std::sync::atomic::{AtomicU16, Ordering};
        static TEST_PORT_COUNTER: AtomicU16 = AtomicU16::new(6000);
        let tcp_port = TEST_PORT_COUNTER.fetch_add(2, Ordering::Relaxed);
        let udp_port = tcp_port + 1;
        let p2p_config = crate::content::sync::P2pConfig { tcp_port, udp_port };
        let test_keypair = libp2p::identity::Keypair::generate_ed25519();
        let (p2p_sync_coordinator, _rx) = crate::content::sync::P2pSyncCoordinator::new(
            p2p_config,
            test_keypair,
            storage.clone(),
            verified_proof_submitter,
            local_identity.clone(),
        )
        .await
        .unwrap();
        let p2p_sync_coordinator = Arc::new(p2p_sync_coordinator);

        use eld_common::address::Address;

        let test_provider =
            Address::parse_hex_str("0xcccccccccccccccccccccccccccccccccccccccc").expect("provider");
        let capacity_config = CapacityConfig {
            capacity_dir: cap_dir,
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
            consensus_config_arc.clone(),
        ));

        // This should succeed with a valid config
        let result = init_abci_server(AbciServerInitContext {
            consensus_config: consensus_config_arc,
            storage: storage.clone(),
            snapshot_manager: Arc::new(SnapshotManager::new(storage)),
            p2p_sync_coordinator,
            init_data: false,
            ready_tx: None,
            capacity_manager,
            local_identity,
            committed_state: Arc::new(Mutex::new(AppState::default())),
            current_state: Arc::new(Mutex::new(None)),
        })
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn fresh_start_stages_config_accounts_without_db_persist() {
        use crate::storage::traits::CADOStorage;
        use eld_common::account::Account;
        use eld_common::address::Address;
        use eld_common::cado::{CadoPath, CadoPathKey, CadoType};
        use eld_common::coin::Coin;
        use eld_common::nonce::Nonce;

        let temp_dir = TempDir::new().unwrap();
        let valid_path = temp_dir.path().join("fresh_start_rocksdb");
        let rocksdb_storage = Arc::new(RocksDBStorage::new(&valid_path).unwrap());
        let storage = Arc::new(HybridStorage::new(rocksdb_storage.clone()));

        let genesis_address = Address::parse_hex_str("0x1234567890123456789012345678901234567890")
            .expect("genesis address");
        let genesis_account = Account::new(
            genesis_address,
            Coin::new(1_000).expect("balance"),
            Nonce::new(0),
        );
        let mut accounts = HashMap::new();
        accounts.insert("genesis".to_string(), genesis_account);

        let config = ConsensusConfig {
            chain_id: "test-chain".to_string(),
            app_host: "127.0.0.1".to_string(),
            app_port: "8080".to_string(),
            accounts,
            max_tx_bytes: 1024 * 1024,
            fee_config: FeeConfig::default(),
            storage_limits: crate::config::StorageLimits::default(),
        };

        let cap_dir = temp_dir.path().join("capacity_fresh_start");
        std::fs::create_dir_all(&cap_dir).unwrap();
        let consensus_config_arc = Arc::new(Mutex::new(config));
        let cli_config = CliConfig {
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
            p2p_tcp_port: None,
            p2p_udp_port: None,
            single_node: None,
            capacity_size_mb: None,
            capacity_storage_path: None,
            indexer: false,
        };
        let cli =
            Arc::new(eld_client::facade::ChainClient::new(cli_config).expect("test ChainClient"));
        let verified_proof_submitter = Arc::new(VerifiedProofChainSubmitter::new(
            "wallet1".into(),
            cli.clone(),
            consensus_config_arc.clone(),
            storage.clone(),
        ));
        let local_identity = Arc::new(RwLock::new(LocalNodeIdentity::default()));

        use std::sync::atomic::{AtomicU16, Ordering};
        static FRESH_START_PORT_COUNTER: AtomicU16 = AtomicU16::new(7000);
        let tcp_port = FRESH_START_PORT_COUNTER.fetch_add(2, Ordering::Relaxed);
        let udp_port = tcp_port + 1;
        let p2p_config = crate::content::sync::P2pConfig { tcp_port, udp_port };
        let test_keypair = libp2p::identity::Keypair::generate_ed25519();
        let (p2p_sync_coordinator, _rx) = crate::content::sync::P2pSyncCoordinator::new(
            p2p_config,
            test_keypair,
            storage.clone(),
            verified_proof_submitter,
            local_identity.clone(),
        )
        .await
        .unwrap();
        let p2p_sync_coordinator = Arc::new(p2p_sync_coordinator);

        let test_provider =
            Address::parse_hex_str("0xcccccccccccccccccccccccccccccccccccccccc").expect("provider");
        let capacity_config = CapacityConfig {
            capacity_dir: cap_dir,
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
            consensus_config_arc.clone(),
        ));

        let committed_state = Arc::new(Mutex::new(AppState::default()));
        init_abci_server(AbciServerInitContext {
            consensus_config: consensus_config_arc,
            storage: storage.clone(),
            snapshot_manager: Arc::new(SnapshotManager::new(storage.clone())),
            p2p_sync_coordinator,
            init_data: false,
            ready_tx: None,
            capacity_manager,
            local_identity,
            committed_state: committed_state.clone(),
            current_state: Arc::new(Mutex::new(None)),
        })
        .await
        .expect("init_abci_server");

        let account_path = CadoPath::new(CadoType::Account, CadoPathKey::Address(genesis_address))
            .expect("account path");
        assert!(
            storage
                .get_cado_by_path(account_path.clone())
                .expect("db read")
                .is_none(),
            "fresh_start must not persist config accounts to db"
        );

        let state = committed_state.lock().expect("committed lock");
        assert!(
            state
                .envelope
                .cado_cache
                .contains_key(account_path.as_str()),
            "fresh_start must stage config accounts in cado_cache"
        );
        assert!(
            state
                .envelope
                .committed_cado_cache
                .get(account_path.as_str().as_bytes())
                .is_none(),
            "committed_cado_cache must stay empty until first commit"
        );
        assert!(
            state.app_hash.is_empty(),
            "app_hash deferred until end_block"
        );
    }
}
