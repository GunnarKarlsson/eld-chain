use super::behaviour::EldBehaviour;
use super::commands::P2pCommand;
use super::config::P2pConfig;
use super::storage::SyncCoordinatorStorage;
use super::swarm::spawn_swarm_task;
use super::SyncMsg;
use crate::app_state::AppState;
use crate::node_identity::LocalNodeIdentity;
use crate::wallet::VerifiedProofChainSubmitter;
use eld_common::constants::p2p::GOSSIPSUB_MAX_TRANSMIT_SIZE_BYTES;
use eld_common::error::EldError;
use libp2p::{
    gossipsub::{self, MessageAuthenticity},
    identity, mdns, PeerId,
};
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::mpsc;
use tracing::info;

pub struct P2pSyncCoordinator {
    _swarm_task: tokio::task::JoinHandle<()>,
    pub(crate) cmd_tx: mpsc::UnboundedSender<P2pCommand>,
    pub(crate) committed_state: Arc<Mutex<Option<Arc<std::sync::Mutex<AppState>>>>>,
    pub(crate) local_identity: Arc<RwLock<LocalNodeIdentity>>,
    pub(crate) storage: Arc<dyn SyncCoordinatorStorage>,
    /// Capacity challenge `VerifiedProof` broadcasts (optimistic nonce + persisted dedup).
    pub(crate) verified_proof_submitter: Arc<VerifiedProofChainSubmitter>,
}

fn p2p_coordinator_init_error(details: impl std::fmt::Display) -> EldError {
    EldError::InitializationError {
        component: "P2P Sync Coordinator".to_string(),
        details: details.to_string(),
    }
}

impl P2pSyncCoordinator {
    pub async fn new(
        port_config: P2pConfig,
        p2p_keypair: identity::Keypair,
        storage: Arc<dyn SyncCoordinatorStorage>,
        verified_proof_submitter: Arc<VerifiedProofChainSubmitter>,
        local_identity: Arc<RwLock<LocalNodeIdentity>>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<SyncMsg>), EldError> {
        Self::new_internal(
            port_config,
            p2p_keypair,
            storage,
            verified_proof_submitter,
            local_identity,
        )
        .await
        .map_err(p2p_coordinator_init_error)
    }

    async fn new_internal(
        port_config: P2pConfig,
        p2p_keypair: identity::Keypair,
        storage: Arc<dyn SyncCoordinatorStorage>,
        verified_proof_submitter: Arc<VerifiedProofChainSubmitter>,
        local_identity: Arc<RwLock<LocalNodeIdentity>>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<SyncMsg>), Box<dyn std::error::Error + Send + Sync>>
    {
        let local_key = p2p_keypair;
        let local_peer_id = PeerId::from(local_key.public());
        info!("Local PeerId: {} (using provided keypair)", local_peer_id);
        info!("Public key type: {:?}", local_key.public().key_type());

        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(std::time::Duration::from_secs(1)) // Your value; or try from_millis(100) for faster testing
            .heartbeat_initial_delay(std::time::Duration::from_millis(100)) // ← ADD: Start heartbeats immediately
            .max_transmit_size(GOSSIPSUB_MAX_TRANSMIT_SIZE_BYTES)
            .flood_publish(true) // Your value: Enables flooding to connected peers (none, but allows local)
            .mesh_n_low(1) // Minimum mesh size: keep at least 1 peer in mesh to maintain connections
            .mesh_n(2) // Target mesh size: maintain 2 peers in mesh for redundancy
            .mesh_n_high(4) // Maximum mesh size: allow up to 4 peers in mesh
            .mesh_outbound_min(1) // Minimum outbound connections: ensure at least 1 outbound connection
            .allow_self_origin(true) // ← ADD: Allows receiving/processing own messages without penalty
            .build()?;

        let swarm = libp2p::SwarmBuilder::with_existing_identity(local_key.clone())
            .with_tokio()
            .with_tcp(
                libp2p::tcp::Config::default(),
                libp2p::noise::Config::new,
                libp2p::yamux::Config::default,
            )?
            .with_quic()
            .with_behaviour(|key| {
                let gossipsub = gossipsub::Behaviour::new(
                    MessageAuthenticity::Signed(key.clone()),
                    gossipsub_config.clone(),
                )
                .expect("Can create gossipsub behavior");

                let mdns =
                    mdns::tokio::Behaviour::new(mdns::Config::default(), key.public().to_peer_id())
                        .expect("Can create mDNS behavior");

                EldBehaviour { gossipsub, mdns }
            })?
            .with_swarm_config(|c| {
                c.with_idle_connection_timeout(std::time::Duration::from_secs(60))
            })
            .build();

        let (swarm_task, cmd_tx, msg_rx) = spawn_swarm_task(
            swarm,
            local_peer_id,
            port_config.tcp_port,
            port_config.udp_port,
        )?;

        Ok((
            P2pSyncCoordinator {
                _swarm_task: swarm_task,
                cmd_tx,
                committed_state: Arc::new(Mutex::new(None)),
                local_identity,
                storage,
                verified_proof_submitter,
            },
            msg_rx,
        ))
    }

    /// Set the committed state for proof validation
    pub fn set_committed_state(&self, committed_state: Arc<std::sync::Mutex<AppState>>) {
        if let Ok(mut state) = self.committed_state.lock() {
            *state = Some(committed_state);
            info!("Committed state set for proof validation");
        }
    }
}
