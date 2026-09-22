mod capacity;
mod content_server;
mod p2p;
mod shutdown;

use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use eld_client::config::{DEFAULT_CONFIG_PATH, WALLETS_PATH};
use eld_client::facade::ChainClient;
use eld_client::logging::SanitizedLog;
use eld_common::error::EldError;
use tokio::sync::{broadcast, oneshot};
use tracing::info;

use crate::abci_interface::snapshot::SnapshotManager;
use crate::abci_interface::{init_abci_server, AbciServerInitContext};
use crate::app_state::AppState;
use crate::config::AppConfig;
use crate::config::ConsensusConfig;
use crate::errors::handle_fatal_eld_error;
use crate::storage::hybrid_storage::HybridStorage;
use crate::storage::pinboard_blob_gc::start_pinboard_blob_gc_worker;
use crate::storage::rocksdb::RocksDBStorage;
use crate::Args;

pub(crate) async fn run(args: Args) -> Result<(), EldError> {
    let config_path = ConsensusConfig::resolve_path(args.config_path.clone());
    let db_path = crate::config::resolve_db_path(args.db_path.clone());

    info!(
        "Using configuration file: {}",
        SanitizedLog::as_path(&config_path)
    );
    info!(
        "Using database directory: {}",
        SanitizedLog::as_path(&db_path)
    );

    let consensus_config = Arc::new(Mutex::new(ConsensusConfig::load_for_node(
        &config_path,
        args.chainid.clone(),
    )?));

    info!(
        "Chain ID: {}",
        consensus_config
            .lock()
            .unwrap_or_else(|e| { handle_fatal_eld_error(e.into()) })
            .chain_id
    );

    let rocks_db_storage = match RocksDBStorage::new(&PathBuf::from(&db_path)) {
        Ok(db) => db,
        Err(e) => handle_fatal_eld_error(e),
    };
    let rocks_db_storage = Arc::new(rocks_db_storage);
    let node_storage = Arc::new(HybridStorage::new(rocks_db_storage.clone()));

    let app_config = AppConfig::from_file(DEFAULT_CONFIG_PATH)?;
    let mut client_config = app_config.client;
    let node_config = app_config.node;
    let fee_config = {
        let consensus = consensus_config
            .lock()
            .unwrap_or_else(|e| handle_fatal_eld_error(e.into()));
        client_config.chain_id = consensus.chain_id.clone();
        consensus.fee_config.clone()
    };
    let cli = Arc::new(
        ChainClient::with_wallets(client_config.clone(), fee_config, WALLETS_PATH)
            .unwrap_or_else(|e| handle_fatal_eld_error(e)),
    );

    let capacity = capacity::init_capacity(
        &node_config,
        &client_config,
        cli.clone(),
        consensus_config.clone(),
    )
    .await;

    let p2p = p2p::init_p2p(
        &node_config,
        capacity.manager.clone(),
        cli.clone(),
        consensus_config.clone(),
        node_storage.clone(),
        capacity.local_identity.clone(),
        capacity.validator_wallet_name.clone(),
    )
    .await?;

    let transaction_indexer: Option<Arc<crate::indexer::TransactionIndexer>> = {
        if node_config.indexer {
            info!("Transaction indexer enabled, initializing...");
            Some(Arc::new(crate::indexer::TransactionIndexer::new(
                rocks_db_storage.clone(),
            )))
        } else {
            info!("Transaction indexer NOT enabled, as per config");
            None
        }
    };

    if let Some(ref indexer) = transaction_indexer {
        let tendermint_rpc_url = format!(
            "http://{}:{}",
            client_config.node_host, client_config.node_port
        );
        indexer.clone().start(tendermint_rpc_url);
    }

    let committed_state = Arc::new(Mutex::new(AppState::default()));
    let current_state: Arc<Mutex<Option<AppState>>> = Arc::new(Mutex::new(None));

    let content_server =
        content_server::bind_content_server(content_server::ContentServerBindContext {
            rocks_db_storage: rocks_db_storage.clone(),
            consensus_config: consensus_config.clone(),
            transaction_indexer,
            cli: cli.clone(),
            capacity_manager: capacity.manager.clone(),
            local_identity: capacity.local_identity.clone(),
            committed_state: committed_state.clone(),
            current_state: current_state.clone(),
            app_port: client_config.app_port.clone(),
        })
        .await?;

    let snapshot_manager = Arc::new(SnapshotManager::new(node_storage.clone()));

    let abci_ip_address = consensus_config
        .lock()
        .unwrap_or_else(|e| {
            handle_fatal_eld_error(e.into());
        })
        .clone()
        .ip_address();

    let (ready_tx, ready_rx) = oneshot::channel();

    let (consensus_server, chain_tip) = init_abci_server(AbciServerInitContext {
        consensus_config,
        storage: node_storage,
        snapshot_manager,
        p2p_sync_coordinator: p2p.coordinator.clone(),
        init_data: args.init_data,
        ready_tx: Some(ready_tx),
        capacity_manager: capacity.manager.clone(),
        local_identity: capacity.local_identity.clone(),
        committed_state: committed_state.clone(),
        current_state,
    })
    .await
    .unwrap_or_else(|e| {
        handle_fatal_eld_error(e);
    });

    start_pinboard_blob_gc_worker(rocks_db_storage, chain_tip, capacity.manager.clone());

    p2p::start_proof_validation_and_heartbeats(p2p.real_coordinator, committed_state);

    let consensus_server = async move {
        let consensus_server_future =
            consensus_server.run(abci_ip_address.parse::<SocketAddr>().unwrap_or_else(|e| {
                handle_fatal_eld_error(EldError::ValidationError {
                    field: "ABCI IP address".to_string(),
                    value: abci_ip_address.clone(),
                    details: format!("Failed to parse ABCI IP address: {e}"),
                });
            }));

        consensus_server_future
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    };

    let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);

    let content_server_handle = shutdown::handle_server_startup(
        "Content Server".to_string(),
        content_server,
        shutdown_tx.clone(),
    )
    .await;

    let consensus_server_handle = shutdown::handle_server_startup(
        "Consensus Server".to_string(),
        consensus_server,
        shutdown_tx.clone(),
    )
    .await;

    crate::capacity::on_ready::after_first_commit(ready_rx, capacity.manager, p2p.coordinator)
        .await;

    shutdown::wait_and_shutdown(
        content_server_handle,
        consensus_server_handle,
        shutdown_tx,
        shutdown_rx,
    )
    .await;

    info!("Eld node shutdown complete");
    Ok(())
}
