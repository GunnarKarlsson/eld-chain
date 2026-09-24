use std::sync::{Arc, Mutex};

use eld_client::facade::ChainClient;
use eld_common::error::EldError;
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::app_state::AppState;
use crate::capacity::capacity_manager::CapacityManager;
use crate::capacity::missing_content_tracker::MissingContentTracker;
use crate::config::{ConsensusConfig, NodeRuntimeConfig};
use crate::content::sync::{
    MockP2pSyncCoordinator, P2pConfig, P2pCoordinatorTrait, P2pSyncCoordinator, SyncMsg,
};
use crate::errors::handle_fatal_eld_error;
use crate::node_identity::LocalNodeIdentity;
use crate::storage::hybrid_storage::HybridStorage;
use crate::wallet::VerifiedProofChainSubmitter;

const DEFAULT_P2P_KEYPAIR_CONFIG_PATH: &str = "./config/p2p_keypair.json";

pub struct P2pRuntime {
    pub coordinator: Arc<dyn P2pCoordinatorTrait>,
    pub real_coordinator: Option<Arc<P2pSyncCoordinator>>,
}

pub async fn init_p2p(
    node_config: &NodeRuntimeConfig,
    capacity_manager: Arc<CapacityManager>,
    cli: Arc<ChainClient>,
    consensus_config: Arc<Mutex<ConsensusConfig>>,
    node_storage: Arc<HybridStorage>,
    local_identity: Arc<std::sync::RwLock<LocalNodeIdentity>>,
    capacity_validator_wallet_name: String,
) -> Result<P2pRuntime, EldError> {
    let single_node_mode = node_config.single_node.unwrap_or(false);
    let mut real_coordinator: Option<Arc<P2pSyncCoordinator>> = None;

    let (coordinator, msg_rx): (
        Arc<dyn P2pCoordinatorTrait>,
        mpsc::UnboundedReceiver<SyncMsg>,
    ) = if single_node_mode {
        info!("Running in single-node mode — using mock P2P coordinator");
        let (mock_coordinator, msg_rx) = MockP2pSyncCoordinator::new();
        let mock_coordinator_arc = Arc::new(mock_coordinator);
        (mock_coordinator_arc as Arc<dyn P2pCoordinatorTrait>, msg_rx)
    } else {
        let p2p_config = P2pConfig::from(node_config);

        let p2p_keypair =
            crate::p2p_keypair::P2PKeypair::load_libp2p_keypair(DEFAULT_P2P_KEYPAIR_CONFIG_PATH)?;

        let derived_peer_id = libp2p::PeerId::from(p2p_keypair.public());
        info!(
            "Using dedicated P2P keypair for P2P identity (peer ID: {})",
            derived_peer_id
        );

        let verified_proof_submitter = Arc::new(VerifiedProofChainSubmitter::new(
            capacity_validator_wallet_name,
            cli,
            consensus_config,
            capacity_manager.protocol_handle(),
            node_storage.clone(),
        ));
        let (coordinator, msg_rx) = P2pSyncCoordinator::new(
            p2p_config,
            p2p_keypair,
            node_storage,
            verified_proof_submitter,
            local_identity,
        )
        .await
        .unwrap_or_else(|e| handle_fatal_eld_error(e));
        info!("P2P sync coordinator started — mDNS + QUIC active");
        let coordinator_arc = Arc::new(coordinator);
        real_coordinator = Some(coordinator_arc.clone());

        (coordinator_arc as Arc<dyn P2pCoordinatorTrait>, msg_rx)
    };

    let capacity_dir = capacity_manager.config().capacity_dir.as_path();
    let mut tracker = MissingContentTracker::new(capacity_dir);
    tracker.load().unwrap_or_else(|e| {
        error!(error = %e, "Failed to load missing content tracker");
    });
    let tracker = Arc::new(Mutex::new(tracker));
    let coordinator_for_handler = Arc::clone(&coordinator);
    coordinator_for_handler.run_message_handler(capacity_manager.clone(), tracker, msg_rx);

    let periodic_sync_config = crate::content::sync::PeriodicSyncConfig::default();
    let periodic_sync_service = crate::content::sync::PeriodicSyncService::new(
        capacity_manager,
        coordinator.clone(),
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

    Ok(P2pRuntime {
        coordinator,
        real_coordinator,
    })
}

/// Heartbeats are always started for a real coordinator (gating is a later hygiene item).
pub fn start_proof_validation_and_heartbeats(
    real_coordinator: Option<Arc<P2pSyncCoordinator>>,
    committed_state: Arc<Mutex<AppState>>,
) {
    if let Some(coordinator_arc) = real_coordinator {
        coordinator_arc.set_committed_state(committed_state);
        info!("Committed state set for proof validation");

        coordinator_arc.clone().start_heartbeat_task();
        coordinator_arc.start_content_sync_heartbeat_task();
        info!("P2P heartbeat tasks started");
    }
}
