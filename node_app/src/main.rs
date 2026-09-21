mod abci_interface;
mod api;
mod app_state;
mod broadcast_log;
mod capacity;
pub mod config;
mod content;
pub mod errors;
mod indexer;
mod node_identity;
mod p2p_keypair;
mod process_logging;
mod storage;
mod sys_disk;
mod wallet;

use crate::abci_interface::snapshot::SnapshotManager;
use crate::api::api_rate_limiting::create_api_rate_limit_config_from_env;
use crate::capacity::missing_content_tracker::MissingContentTracker;
use crate::errors::{handle_fatal_eld_error, log_server_io_error_details};
use crate::storage::pinboard_blob_gc::start_pinboard_blob_gc_worker;
use crate::storage::rocksdb::RocksDBStorage;
use crate::wallet::VerifiedProofChainSubmitter;
use content::sync::{
    MockP2pSyncCoordinator, P2pConfig, P2pCoordinatorTrait, P2pSyncCoordinator, SyncMsg,
};
use eld_client::logging::SanitizedLog;

use crate::abci_interface::{init_abci_server, AbciServerInitContext};
use crate::api::{init_router_with_storage, ApiRouterInitContext};
use crate::app_state::AppState;
use crate::node_identity::LocalNodeIdentity;
use crate::process_logging::init_default_logging;
use clap::Parser;
use config::ConsensusConfig;
use eld_client::config::{CliConfig, DEFAULT_CONFIG_PATH, WALLETS_PATH};
use eld_client::facade::ChainClient;
use eld_common::error::EldError;
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
};
use storage::hybrid_storage::HybridStorage;
use tokio::{
    net::TcpListener,
    sync::{broadcast, mpsc, oneshot},
};
use tracing::{error, info, warn};

// Default paths that can be overridden by environment variables or command-line arguments
const DEFAULT_P2P_KEYPAIR_CONFIG_PATH: &str = "./config/p2p_keypair.json";

#[derive(Parser, Debug)]
#[command(author = "BREE LABS", version, about = "Bree Tendermint App")]
struct Args {
    /// Chain Id
    #[arg()]
    chainid: Option<String>,
    /// Initialize data from database
    #[arg(long)]
    init_data: bool,
    /// Path to consensus configuration file
    #[arg(long)]
    config_path: Option<String>,
    /// Path to database directory
    #[arg(long)]
    db_path: Option<String>,
}

/// Handle server startup errors with detailed logging and graceful failure
async fn handle_server_startup<T>(
    server_name: String,
    server_future: T,
    shutdown_tx: broadcast::Sender<()>,
) -> tokio::task::JoinHandle<()>
where
    T: std::future::Future<
            Output = std::result::Result<(), Box<dyn std::error::Error + Send + Sync>>,
        > + Send
        + 'static,
{
    tokio::spawn(async move {
        let result = server_future.await;
        match result {
            Ok(_) => {
                info!("{} shut down gracefully", server_name);
            }
            Err(e) => {
                error!("{} failed: {}", server_name, e);

                // Log specific IO-related error details for debugging (centralized in error.rs)
                log_server_io_error_details(&server_name, e.as_ref());

                // Trigger shutdown of other servers
                let _ = shutdown_tx.send(());
            }
        }
    })
}

#[tokio::main]
async fn main() -> Result<(), EldError> {
    let (_shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);
    print_name();
    let args = Args::parse();

    init_default_logging().unwrap_or_else(|e| {
        handle_fatal_eld_error(e);
    });

    // Get configurable paths
    let config_path = ConsensusConfig::resolve_path(args.config_path.clone());
    let db_path = config::StorageConfig::resolve_db_path(args.db_path.clone());

    info!(
        "Using configuration file: {}",
        SanitizedLog::as_path(&config_path)
    );
    info!(
        "Using database directory: {}",
        SanitizedLog::as_path(&db_path)
    );

    // Load consensus configuration (includes permission, integrity and semantic validation)
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

    let mut config = CliConfig::from_file(DEFAULT_CONFIG_PATH)?;
    let fee_config = {
        let consensus = consensus_config
            .lock()
            .unwrap_or_else(|e| handle_fatal_eld_error(e.into()));
        config.chain_id = consensus.chain_id.clone();
        consensus.fee_config.clone()
    };
    let cli = Arc::new(
        ChainClient::with_wallets(config.clone(), fee_config, WALLETS_PATH)
            .unwrap_or_else(|e| handle_fatal_eld_error(e)),
    );
    let cli_for_capacity = cli.clone();

    // Initialize capacity manager (required) so we can pass it to orchestrator and sync components
    let (capacity_size_mb, capacity_storage_path) =
        match (config.capacity_size_mb, config.capacity_storage_path.as_ref()) {
            (Some(mb), Some(path)) => (mb, path.clone()),
            _ => handle_fatal_eld_error(EldError::InitializationError {
                component: "capacity".into(),
                details: "capacity_size_mb and capacity_storage_path must both be set in config (required for capacity manager)".into(),
            }),
        };

    use crate::capacity::capacity_manager::CapacityManager;
    use crate::node_identity::{
        capacity_validator_address_from_wallet, resolve_capacity_validator_wallet_name_from_env,
        ELD_CAPACITY_VALIDATOR_WALLET_NAME_ENV,
    };
    use eld_common::capacity::CapacityConfig;

    let capacity_validator_wallet_name = resolve_capacity_validator_wallet_name_from_env()
        .unwrap_or_else(|e| handle_fatal_eld_error(e));

    let capacity_validator_wallet = match cli_for_capacity
        .get_wallet_by_name(capacity_validator_wallet_name.clone())
        .await
    {
        Ok(Some(w)) => w,
        Ok(None) => handle_fatal_eld_error(EldError::InitializationError {
            component: "capacity_validator".into(),
            details: format!(
                "Wallet '{capacity_validator_wallet_name}' not found in wallets.json ({ELD_CAPACITY_VALIDATOR_WALLET_NAME_ENV})"
            ),
        }),
        Err(e) => handle_fatal_eld_error(e),
    };
    let capacity_validator_wallet_address =
        capacity_validator_address_from_wallet(&capacity_validator_wallet);
    info!(
        capacity_size_mb = capacity_size_mb,
        capacity_storage_path = %capacity_storage_path,
        capacity_validator_wallet_name = %capacity_validator_wallet_name,
        capacity_validator_wallet_address = %capacity_validator_wallet_address,
        env = ELD_CAPACITY_VALIDATOR_WALLET_NAME_ENV,
        "Initializing capacity manager with single capacity-validator wallet"
    );

    let capacity_config = CapacityConfig {
        capacity_dir: PathBuf::from(&capacity_storage_path),
        max_capacity_gb: capacity_size_mb / 1024, // Convert MB to GB for internal use
        provider_id: capacity_validator_wallet_address,
        auto_register: true,
        registration_retry_interval_secs: 60,
        tendermint_rpc_url: format!("http://{}:{}", config.node_host, config.node_port),
    };

    let manager = CapacityManager::new(
        capacity_config,
        capacity_validator_wallet_name.clone(),
        cli_for_capacity.clone(),
        consensus_config.clone(),
    );
    manager.initialize().await.unwrap_or_else(|e| {
        error!(error = %e, "Failed to initialize capacity manager");
        handle_fatal_eld_error(e);
    });

    manager
        .allocate_capacity(capacity_size_mb)
        .await
        .unwrap_or_else(|e| {
            error!(error = %e, "Failed to ensure capacity allocation");
            handle_fatal_eld_error(e);
        });

    let capacity_manager = Arc::new(manager);

    let local_identity = Arc::new(RwLock::new(LocalNodeIdentity {
        consensus_validator_address: None,
        capacity_validator_address: Some(capacity_validator_wallet_address),
    }));

    // Initialize P2P sync coordinator (or mock for single-node mode)
    // Note: This must happen after capacity_manager initialization
    let single_node_mode = config.single_node.unwrap_or(false);
    let mut coordinator_arc_opt: Option<Arc<P2pSyncCoordinator>> = None;

    let (p2p_sync_coordinator, msg_rx): (
        Arc<dyn P2pCoordinatorTrait>,
        mpsc::UnboundedReceiver<SyncMsg>,
    ) = if single_node_mode {
        info!("Running in single-node mode — using mock P2P coordinator");
        let (mock_coordinator, msg_rx) = MockP2pSyncCoordinator::new();
        let mock_coordinator_arc = Arc::new(mock_coordinator);
        (mock_coordinator_arc as Arc<dyn P2pCoordinatorTrait>, msg_rx)
    } else {
        let p2p_config = P2pConfig::from(&config);

        // Load dedicated P2P keypair from config file instead of reusing a wallet keypair.
        let p2p_keypair =
            crate::p2p_keypair::P2PKeypair::load_libp2p_keypair(DEFAULT_P2P_KEYPAIR_CONFIG_PATH)?;

        // Add detailed logging to verify the conversion
        let derived_peer_id = libp2p::PeerId::from(p2p_keypair.public());
        info!(
            "[P2PKey] Using dedicated P2P keypair for P2P identity (peer ID: {})",
            derived_peer_id
        );

        let verified_proof_submitter = Arc::new(VerifiedProofChainSubmitter::new(
            capacity_validator_wallet_name,
            cli.clone(),
            consensus_config.clone(),
            node_storage.clone(),
        ));
        let (coordinator, msg_rx) = P2pSyncCoordinator::new(
            p2p_config,
            p2p_keypair,
            node_storage.clone(),
            verified_proof_submitter,
            local_identity.clone(),
        )
        .await
        .unwrap_or_else(|e| handle_fatal_eld_error(e));
        info!("P2P sync coordinator started — mDNS + QUIC active");
        let coordinator_arc = Arc::new(coordinator);
        coordinator_arc_opt = Some(coordinator_arc.clone());

        (coordinator_arc as Arc<dyn P2pCoordinatorTrait>, msg_rx)
    };

    // Start message handler for the selected P2P coordinator
    let capacity_dir = capacity_manager.config().capacity_dir.as_path();
    let mut tracker = MissingContentTracker::new(capacity_dir);
    tracker.load().unwrap_or_else(|e| {
        error!(error = %e, "Failed to load missing content tracker");
    });
    let tracker = Arc::new(Mutex::new(tracker));
    let cm_arc = capacity_manager.clone();
    let coordinator_for_handler = Arc::clone(&p2p_sync_coordinator);
    coordinator_for_handler.run_message_handler(cm_arc, tracker, msg_rx);

    // Start periodic sync service
    let periodic_sync_config = content::sync::PeriodicSyncConfig::default();
    let periodic_sync_service = content::sync::PeriodicSyncService::new(
        capacity_manager.clone(),
        p2p_sync_coordinator.clone(),
        periodic_sync_config,
    )
    .unwrap_or_else(|e| {
        error!(error = %e, "Failed to create periodic sync service");
        handle_fatal_eld_error(e);
    });
    tokio::spawn(async move {
        if let Err(e) = periodic_sync_service.start().await {
            error!(error = %e, "Periodic sync service failed");
        }
    });
    info!("Periodic sync service started");

    // Initialize indexer if enabled (needed for API routes)
    let transaction_indexer: Option<Arc<crate::indexer::TransactionIndexer>> = {
        let indexer_enabled = config.indexer;

        if indexer_enabled {
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
        let tendermint_rpc_url = format!("http://{}:{}", config.node_host, config.node_port);
        indexer.clone().start(tendermint_rpc_url);
    }

    let committed_state = Arc::new(Mutex::new(AppState::default()));
    let current_state: Arc<Mutex<Option<AppState>>> = Arc::new(Mutex::new(None));

    let content_server = {
        // Create rate limiting configuration and state
        let rate_limit_config = create_api_rate_limit_config_from_env();
        let rate_limit_state = Arc::new(tokio::sync::RwLock::new(crate::api::RateLimitState::new(
            rate_limit_config.clone(),
        )));

        let admin_token = std::env::var(crate::api::admin::ELD_ADMIN_TOKEN_ENV)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let router = init_router_with_storage(ApiRouterInitContext {
            storage: rocks_db_storage.clone(),
            consensus_config: consensus_config.clone(),
            rate_limit_state: rate_limit_state.clone(),
            indexer: transaction_indexer.clone(),
            cli: cli.clone(),
            capacity_manager: capacity_manager.clone(),
            local_identity: local_identity.clone(),
            admin_token,
            committed_state: committed_state.clone(),
            current_state: current_state.clone(),
        });

        // Log rate limiting configuration
        info!(
            "Content server rate limiting enabled: {} requests/minute",
            rate_limit_config.general_requests_per_minute
        );

        let app_addr = format!("0.0.0.0:{}", config.app_port.to_owned());

        let listener = TcpListener::bind(app_addr.clone()).await.map_err(|e| {
            error!("Failed to bind content server to {}: {}", app_addr, e);
            match e.kind() {
                std::io::ErrorKind::AddrInUse => {
                    EldError::InitializationError {
                        component: "content server".to_string(),
                        details: "Port 8081 is already in use. Check if another instance is running.".to_string(),
                    }
                }
                std::io::ErrorKind::PermissionDenied => {
                    EldError::InitializationError {
                        component: "content server".to_string(),
                        details: "Permission denied to bind to port 8081. Check if you have sufficient privileges.".to_string(),
                    }
                }
                _ => EldError::InitializationError {
                    component: "content server".to_string(),
                    details: format!("Failed to start content server: {e}"),
                }
            }
        })?;
        info!(
            "Content server listening on {}",
            listener
                .local_addr()
                .map_err(|e| EldError::InitializationError {
                    component: "content server".to_string(),
                    details: format!("Failed to get local address: {e}"),
                })?
        );
        async move {
            axum::serve(listener, router)
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        }
    };

    let snapshot_manager = Arc::new(SnapshotManager::new(node_storage.clone()));

    let abci_ip_address = consensus_config
        .lock()
        .unwrap_or_else(|e| {
            handle_fatal_eld_error(e.into());
        })
        .clone()
        .ip_address();

    // Create ready channel to signal when node is ready (block_height > 0)
    let (ready_tx, ready_rx) = oneshot::channel();

    let (consensus_server, chain_tip) = init_abci_server(AbciServerInitContext {
        consensus_config,
        storage: node_storage,
        snapshot_manager,
        p2p_sync_coordinator: p2p_sync_coordinator.clone(),
        init_data: args.init_data,
        ready_tx: Some(ready_tx),
        capacity_manager: capacity_manager.clone(),
        local_identity: local_identity.clone(),
        committed_state: committed_state.clone(),
        current_state: current_state.clone(),
    })
    .await
    .unwrap_or_else(|e| {
        handle_fatal_eld_error(e);
    });

    // Run pinboard GC in a separate background task (capacity-slot mode).
    start_pinboard_blob_gc_worker(
        rocks_db_storage.clone(),
        chain_tip.clone(),
        capacity_manager.clone(),
    );

    // Update sync coordinator with committed_state for proof validation
    if let Some(coordinator_arc) = coordinator_arc_opt {
        coordinator_arc.set_committed_state(committed_state.clone());
        info!("Committed state set for proof validation");

        // Start heartbeat task for debugging - sends heartbeats every 5 seconds to registered capacity providers
        coordinator_arc.clone().start_heartbeat_task();
        info!("Heartbeat task started for P2P debugging");

        // Start content sync heartbeat task for debugging - sends ContentSyncHeartbeat every 5 seconds on P2P_TOPIC_CONTENT_SYNC
        coordinator_arc.start_content_sync_heartbeat_task();
        info!("Content sync heartbeat task started for P2P debugging");
    }

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

    // Create shutdown channels for graceful shutdown
    let (shutdown_tx, _shutdown_rx) = broadcast::channel::<()>(1);
    let shutdown_tx_clone = shutdown_tx.clone();
    let shutdown_tx_consensus = shutdown_tx.clone();

    // Spawn servers with individual error handling
    let mut content_server_handle = handle_server_startup(
        "Content Server".to_string(),
        content_server,
        shutdown_tx_clone,
    )
    .await;

    let mut consensus_server_handle = handle_server_startup(
        "Consensus Server".to_string(),
        consensus_server,
        shutdown_tx_consensus,
    )
    .await;

    // Wait for node to be ready (block_height > 0) before submitting capacity registration
    info!("Waiting for node to be ready (block height > 0)...");
    match ready_rx.await {
        Ok(_) => {
            info!("Node is READY: First consensus commit completed, block height > 0");

            // Check if this node is already registered as a capacity provider
            {
                use eld_client::api::abci::AbciHttpApi;

                let provider_id = capacity_manager.config().provider_id;
                let provider_id_str = provider_id.to_string();
                let tendermint_rpc_url = capacity_manager.config().tendermint_rpc_url.clone();

                match AbciHttpApi::new(tendermint_rpc_url) {
                    Ok(abci_api) => {
                        match abci_api
                            .is_capacity_provider_registered(&provider_id_str)
                            .await
                        {
                            Ok(true) => {
                                info!(
                                    provider_id = %provider_id,
                                    "Current node is registered as a capacity provider"
                                );
                                let challenge_topic = format!(
                                    "{}{}",
                                    eld_common::constants::p2p::ELD_STORAGE_CHALLENGE_TOPIC_PREFIX,
                                    provider_id
                                );
                                info!("XZXZ21: Subscribing to challenge topic on startup provider_id={} topic={}", provider_id, challenge_topic);
                                if let Err(e) =
                                    p2p_sync_coordinator.subscribe_to_topic(&challenge_topic)
                                {
                                    error!(
                                        provider_id = %provider_id,
                                        topic = %challenge_topic,
                                        error = %e,
                                        "Failed to subscribe to challenge topic"
                                    );
                                } else {
                                    info!(
                                        provider_id = %provider_id,
                                        topic = %challenge_topic,
                                        "Successfully subscribed to challenge topic"
                                    );
                                }
                            }
                            Ok(false) => {
                                info!(
                                    provider_id = %provider_id,
                                    "Current node is not yet registered as a capacity provider"
                                );
                            }
                            Err(e) => {
                                warn!(
                                    provider_id = %provider_id,
                                    error = %e,
                                    "Failed to check if node is registered as capacity provider"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        warn!(
                            provider_id = %provider_id,
                            error = %e,
                            "Failed to create ABCI HTTP client to check capacity provider registration"
                        );
                    }
                }
            }

            // Submit capacity registration transaction if capacity is allocated
            {
                let manager = &*capacity_manager;
                // Check if capacity is allocated and not already registered
                let slot_allocator = manager.slot_allocator();
                let slot_allocator_guard = slot_allocator.lock().await;
                let capacity_allocated = slot_allocator_guard.slot_map_exists();
                drop(slot_allocator_guard);

                if capacity_allocated {
                    // Query registration info from capacity manager
                    match manager.get_registration_info().await {
                        Ok((capacity_bytes, seed, merkle_root)) => {
                            info!("Submitting capacity registration transaction...");
                            if let Err(e) = manager
                                .register_capacity_onchain(capacity_bytes, seed, merkle_root)
                                .await
                            {
                                let wallet_missing = matches!(
                                    &e,
                                    EldError::StorageError { operation, details }
                                        if operation == "register_capacity_onchain"
                                            && details.contains("Failed to get wallet")
                                );
                                if wallet_missing {
                                    handle_fatal_eld_error(e);
                                }
                                error!(
                                    error = %e,
                                    "Failed to submit capacity registration transaction"
                                );
                                // Non-wallet failures: node can continue; registration may be retried later
                            } else {
                                info!("Capacity registration transaction submitted successfully");

                                // Subscribe to challenge topic after successful registration
                                let provider_id = manager.config().provider_id;
                                let challenge_topic = format!(
                                    "{}{}",
                                    eld_common::constants::p2p::ELD_STORAGE_CHALLENGE_TOPIC_PREFIX,
                                    provider_id
                                );
                                info!("XZXZ21: Subscribing to challenge topic provider_id={} topic={}", provider_id, challenge_topic);
                                if let Err(e) =
                                    p2p_sync_coordinator.subscribe_to_topic(&challenge_topic)
                                {
                                    error!(
                                        provider_id = %provider_id,
                                        topic = %challenge_topic,
                                        error = %e,
                                        "Failed to subscribe to challenge topic after registration"
                                    );
                                } else {
                                    info!(
                                        provider_id = %provider_id,
                                        topic = %challenge_topic,
                                        "Successfully subscribed to challenge topic after registration"
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            warn!(
                                error = %e,
                                "Failed to get registration info - capacity may not be allocated"
                            );
                        }
                    }
                } else {
                    info!("Capacity not allocated, skipping registration");
                }
            }
        }
        Err(_) => {
            warn!("Ready channel was closed before signaling - node may not be ready");
        }
    }

    // Wait for any server to fail or shutdown signal
    let mut content_completed = false;
    let mut consensus_completed = false;

    loop {
        tokio::select! {
            result = &mut content_server_handle, if !content_completed => {
                content_completed = true;
                match result {
                    Ok(_) => {
                        info!("Content server task completed");
                    }
                    Err(e) => {
                        error!("Content server task panicked: {}", e);
                    }
                }
            }
            result = &mut consensus_server_handle, if !consensus_completed => {
                consensus_completed = true;
                match result {
                    Ok(_) => {
                        info!("Consensus server task completed");
                    }
                    Err(e) => {
                        error!("Consensus server task panicked: {}", e);
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!("Shutdown signal received, initiating graceful shutdown");
                break;
            }
        }

        // If any server completed or failed, trigger shutdown
        if content_completed || consensus_completed {
            let _ = shutdown_tx.send(());
            break;
        }
    }

    // Graceful shutdown
    let shutdown_timeout = tokio::time::Duration::from_secs(10);
    let shutdown_future = tokio::time::timeout(shutdown_timeout, async {
        // Cancel all server tasks
        content_server_handle.abort();
        consensus_server_handle.abort();

        // Wait for tasks to complete
        let _ = tokio::join!(content_server_handle, consensus_server_handle);
    });

    match shutdown_future.await {
        Ok(_) => {
            info!("All servers shut down gracefully");
        }
        Err(_) => {
            warn!("Some servers did not shut down within timeout, forcing exit");
        }
    }

    info!("Eld node shutdown complete");
    Ok(())
}

fn print_name() {
    info!("\x1b[34m             "); // Blue color
    info!("=                          =");
    info!("=== === = = ==   ==  === === ===");
    info!("= = === === = =  = = = = = = =");
    info!("=== = = = = = =  = = === === ===");
    info!("    \x1b[0m"); // Reset color
    info!("\nEld Node Starting...\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_db_path() {
        let invalid_path = PathBuf::from("/invalid/rocksdb");

        // Test that RocksDBStorage::new fails with invalid path
        let rocksdb_result = RocksDBStorage::new(&invalid_path);
        assert!(rocksdb_result.is_err());
    }

    #[tokio::test]
    async fn test_server_error_handling() {
        use tokio::sync::broadcast;

        // Test the handle_server_startup function with a failing server
        let (shutdown_tx, _shutdown_rx) = broadcast::channel::<()>(1);

        // Create a server that will fail immediately
        let failing_server = async {
            Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                "Port already in use",
            )) as Box<dyn std::error::Error + Send + Sync>)
        };

        let handle =
            handle_server_startup("Test Server".to_string(), failing_server, shutdown_tx).await;

        // Wait for the task to complete
        let result = handle.await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_port_binding_error_messages() {
        // Test that port binding error messages are descriptive
        let addr_in_use_error =
            std::io::Error::new(std::io::ErrorKind::AddrInUse, "Address already in use");

        let permission_error =
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Permission denied");

        // These would be used in the actual error handling logic
        // Just testing that we can create the error types
        assert_eq!(addr_in_use_error.kind(), std::io::ErrorKind::AddrInUse);
        assert_eq!(
            permission_error.kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }
}
