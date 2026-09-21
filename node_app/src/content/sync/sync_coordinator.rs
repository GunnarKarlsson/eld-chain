use crate::app_state::AppState;
use crate::capacity::capacity_manager::{CapacityManager, CapacityProofGenerationParams};
use crate::capacity::missing_content_tracker::MissingContentTracker;
use crate::node_identity::LocalNodeIdentity;
use crate::storage::traits::PinboardQueryStorage;
use crate::wallet::VerifiedProofChainSubmitter;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use eld_client::facade::CliConfig;
use eld_common::constants::p2p::{
    GOSSIPSUB_MAX_TRANSMIT_SIZE_BYTES, P2P_TOPIC_CONTENT_SYNC, P2P_TOPIC_CONTENT_SYNC_RESPONSE,
};
use eld_common::constants::pinboard::MAX_CHUNK_SIZE;
use eld_common::error::EldError;
use eld_common::pinboard::PinboardMessageMetadata;
pub use eld_common::sync_msg::SyncMsg;
use eld_common::CapacityMerkleRoot;
use eld_common::ChallengeId;
use hex;
use libp2p::futures::StreamExt;
use libp2p::{
    gossipsub::{self, IdentTopic, MessageAuthenticity},
    identity, mdns,
    swarm::{NetworkBehaviour, SwarmEvent},
    PeerId,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

#[derive(NetworkBehaviour)]
#[behaviour(event_process = false)]
pub struct EldBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: mdns::tokio::Behaviour,
}

#[derive(Debug, Clone)]
pub struct P2pConfig {
    pub tcp_port: u16,
    pub udp_port: u16,
}

/// Object-safe storage surface required by the P2P sync coordinator.
pub trait SyncCoordinatorStorage: Send + Sync {
    fn get_pinboard_temp_blob(
        &self,
        content_key: &str,
    ) -> Result<Option<Vec<u8>>, eld_common::error::EldError>;
}

impl<T> SyncCoordinatorStorage for T
where
    T: PinboardQueryStorage + Send + Sync,
{
    fn get_pinboard_temp_blob(
        &self,
        content_key: &str,
    ) -> Result<Option<Vec<u8>>, eld_common::error::EldError> {
        <T as PinboardQueryStorage>::get_pinboard_temp_blob(self, content_key)
    }
}

impl From<&CliConfig> for P2pConfig {
    fn from(config: &CliConfig) -> Self {
        Self {
            tcp_port: config
                .p2p_tcp_port
                .as_deref()
                .unwrap_or("4001")
                .parse::<u16>()
                .unwrap_or(4001),
            udp_port: config
                .p2p_udp_port
                .as_deref()
                .unwrap_or("4002")
                .parse::<u16>()
                .unwrap_or(4002),
        }
    }
}

/// Trait for P2P coordinator operations
pub trait P2pCoordinatorTrait: Send + Sync {
    fn broadcast_announce(&self, key: String);
    fn broadcast_content_request(&self, key: String);
    fn broadcast_content_response(&self, key: String, content: String);
    fn find_pinboard_and_broadcast_announce(
        &self,
        pinboard_meta_cache: &BTreeMap<String, PinboardMessageMetadata>,
    );
    fn subscribe_to_topic(&self, topic: &str) -> Result<(), String>;
    fn publish_to_topic(&self, topic: &str, msg: SyncMsg) -> Result<(), String>;

    /// Start the message handler task that processes incoming P2P sync messages.
    ///
    /// Implementations should typically delegate to their internal message loop
    /// and return the spawned task handle.
    fn run_message_handler(
        self: Arc<Self>,
        capacity_manager: Arc<CapacityManager>,
        missing_content_tracker: Arc<Mutex<MissingContentTracker>>,
        msg_rx: mpsc::UnboundedReceiver<SyncMsg>,
    ) -> tokio::task::JoinHandle<()>;
}

#[derive(Debug)]
enum P2pCommand {
    BroadcastAnnounce { key: String },
    BroadcastContentRequest { key: String },
    BroadcastContentResponse { key: String, content: String },
    SubscribeToTopic { topic: String },
    PublishToTopic { topic: String, msg: SyncMsg },
}

pub struct P2pSyncCoordinator {
    _swarm_task: tokio::task::JoinHandle<()>,
    cmd_tx: mpsc::UnboundedSender<P2pCommand>,
    committed_state: Arc<Mutex<Option<Arc<std::sync::Mutex<AppState>>>>>,
    local_identity: Arc<RwLock<LocalNodeIdentity>>,
    storage: Arc<dyn SyncCoordinatorStorage>,
    /// Capacity challenge `VerifiedProof` broadcasts (optimistic nonce + persisted dedup).
    verified_proof_submitter: Arc<VerifiedProofChainSubmitter>,
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
        info!(
            "[P2PKey] Local PeerId: {} (using provided keypair)",
            local_peer_id
        );
        info!(
            "[P2PKey] Public key type: {:?}",
            local_key.public().key_type()
        );

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

        // TODO: Add Mplex support alongside Yamux for Android client compatibility
        // Current issue: libp2p-mplex 0.43.1 is not compatible with libp2p 0.53.2
        // Options: 1) Update Android client to use Yamux (recommended)
        //          2) Downgrade libp2p to version compatible with libp2p-mplex 0.43.1
        //          3) Wait for libp2p-mplex update compatible with libp2p 0.53.2
        // For now, using Yamux which is more modern and better supported
        let mut swarm = libp2p::SwarmBuilder::with_existing_identity(local_key.clone())
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

        // Use configured ports for TCP and QUIC
        swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{}", port_config.tcp_port).parse()?)?;
        swarm.listen_on(format!("/ip4/0.0.0.0/udp/{}/quic-v1", port_config.udp_port).parse()?)?;

        let topic = IdentTopic::new(P2P_TOPIC_CONTENT_SYNC);
        swarm.behaviour_mut().gossipsub.subscribe(&topic)?;
        info!("[P2PKey] Subscribed to topic: {}", P2P_TOPIC_CONTENT_SYNC);

        let response_topic = IdentTopic::new(P2P_TOPIC_CONTENT_SYNC_RESPONSE);
        swarm.behaviour_mut().gossipsub.subscribe(&response_topic)?;
        info!(
            "[P2PKey] Subscribed to topic: {}",
            P2P_TOPIC_CONTENT_SYNC_RESPONSE
        );

        let (msg_tx, msg_rx) = mpsc::unbounded_channel();
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel();

        // Track subscribed topics ourselves since gossipsub.topics() may not include our own subscriptions
        let subscribed_topics: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

        let swarm_task = tokio::spawn(async move {
            let subscribed_topics = subscribed_topics.clone();
            loop {
                tokio::select! {
                    event = swarm.select_next_some() => {
                        match event {
                            SwarmEvent::NewListenAddr { address, .. } => {
                                info!("[P2PKey] P2P listening on {}", address);
                            }
                            SwarmEvent::Behaviour(EldBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                                for (peer_id, addr) in list {
                                    info!("[P2PKey] mDNS discovered peer {} at {}", peer_id, addr);
                                    info!("[P2PKey] Our peer ID: {}", local_peer_id);
                                    if peer_id == local_peer_id {
                                        info!("[P2PKey] Ignoring self-discovery");
                                    } else {
                                        swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                                        let _ = swarm.dial(addr);
                                    }
                                }
                            }
                            SwarmEvent::Behaviour(EldBehaviourEvent::Gossipsub(gossipsub::Event::Message { message, .. })) => {
                                // Process messages from any subscribed topic (not just `P2P_TOPIC_CONTENT_SYNC`)
                                // This includes challenge topics like `{ELD_STORAGE_CHALLENGE_TOPIC_PREFIX}{capacity_provider}`
                                info!("P2P Gossipsub: message received");
                                match bincode::deserialize::<SyncMsg>(&message.data) {
                                    Ok(sync_msg) => {
                                        info!("P2P Gossipsub: message event deserialized");
                                        let _ = msg_tx.send(sync_msg);
                                    }
                                    Err(e) => {
                                        warn!(
                                            error = %e,
                                            message_size = message.data.len(),
                                            "P2P Failed to deserialize SyncMsg from P2P message"
                                        );
                                    }
                                }
                            }
                            SwarmEvent::Behaviour(EldBehaviourEvent::Gossipsub(gossipsub::Event::Subscribed { peer_id, topic })) => {
                                info!(
                                    "[P2PKey] Peer {} subscribed to topic {}",
                                    peer_id, topic
                                );
                            }
                            SwarmEvent::Behaviour(EldBehaviourEvent::Gossipsub(gossipsub::Event::Unsubscribed { peer_id, topic })) => {
                                info!(
                                    "[P2PKey] Peer {} unsubscribed from topic {}",
                                    peer_id, topic
                                );
                            }
                            SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                                info!(
                                    "[P2PKey] Connection established with peer {} via {}",
                                    peer_id,
                                    endpoint.get_remote_address()
                                );
                                // Add peer as explicit peer to GossipSub so it stays connected
                                swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                                info!("[P2PKey] Added peer {} as explicit GossipSub peer", peer_id);
                            }
                            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                                info!(
                                    "[P2PKey] Connection closed with peer {}: {:?}",
                                    peer_id, cause
                                );
                            }
                            SwarmEvent::OutgoingConnectionError { peer_id, error, connection_id: _, .. } => {
                                warn!(
                                    "[P2PKey] Failed to connect to peer {:?}: {}",
                                    peer_id, error
                                );
                            }
                            SwarmEvent::IncomingConnectionError { error, .. } => {
                                warn!("[P2PKey] Incoming connection error: {}", error);
                            }
                            _ => {
                                info!("XZXZ5: Other event received");
                            }
                        }
                    }

                    Some(cmd) = cmd_rx.recv() => {
                        match cmd {
                            P2pCommand::BroadcastAnnounce { key } => {
                                let announce = SyncMsg::Announce { content_id: key.clone() };
                                if let Ok(data) = bincode::serialize(&announce) {
                                    if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), data) {
                                        warn!("Failed to broadcast Announce for key '{}': {}", key, e);
                                    } else {
                                        info!("Successfully broadcasted Announce for key: {}", key);
                                    }
                                }
                            }
                            P2pCommand::BroadcastContentRequest { key } => {
                                let msg = SyncMsg::ContentRequest { content_id: key.clone() };
                                if let Ok(data) = bincode::serialize(&msg) {
                                    if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), data) {
                                        warn!("Failed to broadcast ContentRequest for key '{}': {}", key, e);
                                    } else {
                                        info!("Broadcasted ContentRequest for key: {}", key);
                                    }
                                }
                            }
                            P2pCommand::BroadcastContentResponse { key, content } => {
                                let msg = SyncMsg::ContentResponse { content_id: key.clone(), content: content.clone() };
                                if let Ok(data) = bincode::serialize(&msg) {
                                    if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), data) {
                                        warn!("Failed to broadcast ContentResponse for key '{}': {}", key, e);
                                    } else {
                                        info!("Broadcasted ContentResponse for key: {}", key);
                                    }
                                }
                            }
                            P2pCommand::SubscribeToTopic { topic: topic_str } => {
                                info!("P2P XZXZ16: SubscribeToTopic command received topic={}", topic_str);
                                let challenge_topic = IdentTopic::new(&topic_str);
                                let topic_hash = challenge_topic.hash();
                                info!("P2P XZXZ17: Created IdentTopic for subscription topic={} hash={}", topic_str, topic_hash);
                                match swarm.behaviour_mut().gossipsub.subscribe(&challenge_topic) {
                                    Ok(_) => {
                                        info!("P2P XZXZ18: Successfully subscribed to topic topic={} hash={}", topic_str, topic_hash);
                                        // Track subscription ourselves
                                        subscribed_topics.lock().unwrap().insert(topic_str.clone());
                                    }
                                    Err(e) => {
                                        warn!("P2P XZXZ19: Failed to subscribe to topic topic={} hash={} error={}", topic_str, topic_hash, e);
                                    }
                                }
                            }
                            P2pCommand::PublishToTopic { topic: topic_str, msg } => {
                                info!("XZXZ6: Received PublishToTopic command topic={}", topic_str);
                                if let Ok(data) = bincode::serialize(&msg) {
                                    info!("XZXZ7: Message serialized successfully, size={} bytes", data.len());
                                    let topic_ident = IdentTopic::new(&topic_str);
                                    let topic_hash = topic_ident.hash();
                                    info!("XZXZ8: Created IdentTopic for publish topic={} hash={}", topic_str, topic_hash);
                                    // Check if we're subscribed to this topic using our own tracking
                                    let is_subscribed = subscribed_topics.lock().unwrap().contains(&topic_str);
                                    let subscribed_count = subscribed_topics.lock().unwrap().len();
                                    info!("XZXZ20: Currently subscribed topics count={} is_subscribed_to_topic={} publish_topic={} publish_hash={}", subscribed_count, is_subscribed, topic_str, topic_hash);

                                    // Clone data for potential manual injection
                                    let data_clone = data.clone();

                                    // Try to publish - even if we get InsufficientPeers, the message might still be delivered locally
                                    match swarm.behaviour_mut().gossipsub.publish(topic_ident, data) {
                                        Ok(message_id) => {
                                            info!("XZXZ9: Published message to topic: {} message_id={:?}", topic_str, message_id);
                                        }
                                        Err(e) => {
                                            // Even if publish fails with InsufficientPeers, manually inject the message locally
                                            // if we're subscribed (for single-node testing)
                                            if is_subscribed {
                                                warn!("XZXZ10: Failed to publish to topic '{}': {} (but we're subscribed, manually injecting locally)", topic_str, e);
                                                // Manually send the message to our handler since gossipsub isn't delivering it locally
                                                if let Ok(sync_msg) = bincode::deserialize::<SyncMsg>(&data_clone) {
                                                    info!("XZXZ11: Manually injecting message locally for topic={}", topic_str);
                                                    let _ = msg_tx.send(sync_msg);
                                                }
                                            } else {
                                                warn!("XZXZ10: Failed to publish to topic '{}': {} (NOT subscribed!)", topic_str, e);
                                            }
                                        }
                                    }
                                } else {
                                    warn!("XZXZ11: Failed to serialize message for topic '{}'", topic_str);
                                }
                            }
                        }
                    }
                }
            }
        });

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

    /// Start the heartbeat task that sends heartbeats every 5 seconds to all registered capacity providers
    pub fn start_heartbeat_task(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        use eld_common::constants::p2p::ELD_STORAGE_CHALLENGE_TOPIC_PREFIX;
        use std::time::{SystemTime, UNIX_EPOCH};

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
            // Skip the first tick to avoid immediate execution
            interval.tick().await;

            info!("Heartbeat task started (sending heartbeats every 5 seconds to registered capacity providers)");

            loop {
                interval.tick().await;

                // Get committed state
                let committed_state = match self.committed_state.lock() {
                    Ok(state) => state.clone(),
                    Err(e) => {
                        warn!(
                            error = %e,
                            "Failed to acquire committed_state lock for heartbeat"
                        );
                        continue;
                    }
                };

                let capacity_validators = match committed_state {
                    Some(state_lock) => {
                        let state = match state_lock.lock() {
                            Ok(state) => state,
                            Err(e) => {
                                warn!(
                                    error = %e,
                                    "Failed to acquire app_state lock for heartbeat"
                                );
                                continue;
                            }
                        };
                        state.envelope.capacity_validators.clone()
                    }
                    None => {
                        // No committed state yet, skip this iteration
                        continue;
                    }
                };

                if capacity_validators.is_empty() {
                    info!("No registered capacity providers, skipping heartbeat");
                    continue;
                }

                // Get current timestamp
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                // Send heartbeat to each registered capacity provider
                for provider in &capacity_validators {
                    let capacity_provider = &provider.address;
                    let challenge_topic =
                        format!("{ELD_STORAGE_CHALLENGE_TOPIC_PREFIX}{capacity_provider}");

                    let heartbeat_msg = SyncMsg::Heartbeat { timestamp };

                    match self.publish_to_topic(&challenge_topic, heartbeat_msg) {
                        Ok(_) => {
                            info!(
                                capacity_provider = %capacity_provider,
                                topic = %challenge_topic,
                                timestamp = timestamp,
                                "Sent heartbeat to capacity provider"
                            );
                        }
                        Err(e) => {
                            warn!(
                                capacity_provider = %capacity_provider,
                                topic = %challenge_topic,
                                error = %e,
                                "Failed to send heartbeat to capacity provider"
                            );
                        }
                    }
                }

                info!(
                    provider_count = capacity_validators.len(),
                    "Sent heartbeats to all registered capacity providers"
                );
            }
        })
    }

    /// Start the content sync heartbeat task that sends ContentSyncHeartbeat every 5 seconds on `P2P_TOPIC_CONTENT_SYNC`
    pub fn start_content_sync_heartbeat_task(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        use std::time::{SystemTime, UNIX_EPOCH};

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
            // Skip the first tick to avoid immediate execution
            interval.tick().await;

            info!(
                "Content sync heartbeat task started (sending ContentSyncHeartbeat every 5 seconds to {} topic)",
                P2P_TOPIC_CONTENT_SYNC
            );

            loop {
                interval.tick().await;

                // Get current timestamp
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                // Send ContentSyncHeartbeat to content sync topic
                let content_sync_topic = P2P_TOPIC_CONTENT_SYNC;
                let heartbeat_msg = SyncMsg::ContentSyncHeartbeat { timestamp };

                match self.publish_to_topic(content_sync_topic, heartbeat_msg) {
                    Ok(_) => {
                        info!(
                            topic = %content_sync_topic,
                            timestamp = timestamp,
                            "Sent ContentSyncHeartbeat to {} topic",
                            content_sync_topic
                        );
                    }
                    Err(e) => {
                        warn!(
                            topic = %content_sync_topic,
                            error = %e,
                            "Failed to send ContentSyncHeartbeat to {} topic",
                            content_sync_topic
                        );
                    }
                }
            }
        })
    }

    /// Start the message handler task that processes incoming P2P sync messages
    pub fn run_message_handler(
        self: Arc<Self>,
        capacity_manager: Arc<CapacityManager>,
        missing_content_tracker: Arc<Mutex<MissingContentTracker>>,
        mut msg_rx: mpsc::UnboundedReceiver<SyncMsg>,
    ) -> tokio::task::JoinHandle<()> {
        let committed_state_ref = self.committed_state.clone();
        tokio::spawn(async move {
            while let Some(msg) = msg_rx.recv().await {
                let committed_state = committed_state_ref.lock().ok().and_then(|s| s.clone());
                self.handle_sync_message(
                    &capacity_manager,
                    &missing_content_tracker,
                    committed_state.as_ref(),
                    msg,
                )
                .await;
            }
        })
    }

    /// Handle a single sync message
    async fn handle_sync_message(
        &self,
        capacity_manager: &CapacityManager,
        missing_content_tracker: &Mutex<MissingContentTracker>,
        committed_state: Option<&Arc<std::sync::Mutex<AppState>>>,
        msg: SyncMsg,
    ) {
        match msg {
            SyncMsg::Heartbeat { timestamp } => {
                info!(
                    timestamp = timestamp,
                    "💓 P2P Heartbeat received from storage validator (debugging message)"
                );
            }
            SyncMsg::ContentSyncHeartbeat { timestamp } => {
                info!(
                    timestamp = timestamp,
                    "💓 P2P ContentSyncHeartbeat received on {} topic (debugging message)",
                    P2P_TOPIC_CONTENT_SYNC
                );
            }
            SyncMsg::ContentSyncHeartbeatResponse {
                provider_id: provider,
                timestamp,
            } => {
                info!(
                    provider = %provider,
                    timestamp = timestamp,
                    "💓 P2P ContentSyncHeartbeatResponse received from capacity provider on {} topic",
                    P2P_TOPIC_CONTENT_SYNC
                );
                // Log that we received a heartbeat response from a capacity provider
                // This confirms the provider is active and can receive/send messages
            }
            SyncMsg::Announce { content_id } => {
                // Compatibility mode: keep SyncMsg field name `content_id`, but treat it as a generic blob key.
                let content_key = content_id;
                info!(%content_key, "P2P Announce received (pinboard content_key)");
                let has_temp_blob = self
                    .storage
                    .get_pinboard_temp_blob(&content_key)
                    .map(|blob| blob.is_some())
                    .unwrap_or(false);
                if has_temp_blob {
                    info!(%content_key, "Pinboard temp blob exists locally, ignoring Announce");
                    return;
                }
                match capacity_manager.get_content_from_slots(&content_key).await {
                    Ok(_) => {
                        info!(%content_key, "Slot content exists locally, ignoring Announce");
                    }
                    Err(_) => {
                        info!(%content_key, "Content not found locally — sending ContentRequest");
                        if let Ok(mut tracker) = missing_content_tracker.lock() {
                            if let Err(e) =
                                tracker.upsert_missing_content_by_content_id(&content_key)
                            {
                                warn!(
                                    %content_key,
                                    error = %e,
                                    "Failed to track missing content key"
                                );
                            }
                        }
                        self.broadcast_content_request(content_key);
                    }
                }
            }
            SyncMsg::ContentRequest { content_id } => {
                // Compatibility mode: field name is `content_id`, value is interpreted as pinboard content_key.
                let content_key = content_id;
                info!(%content_key, "P2P ContentRequest received (pinboard content_key)");
                let pinboard_temp_blob = self
                    .storage
                    .get_pinboard_temp_blob(&content_key)
                    .ok()
                    .flatten();
                if let Some(content_bytes) = pinboard_temp_blob {
                    info!(
                        %content_key,
                        content_size = content_bytes.len(),
                        "Sending ContentResponse for locally available pinboard temp blob"
                    );
                    let content_base64 = BASE64_STANDARD.encode(&content_bytes);
                    self.broadcast_content_response(content_key, content_base64);
                    return;
                }

                match capacity_manager.get_content_from_slots(&content_key).await {
                    Ok(content_bytes) => {
                        info!(
                            %content_key,
                            content_size = content_bytes.len(),
                            "Sending ContentResponse for locally available slot content"
                        );
                        // Encode content as base64 for transmission
                        let content_base64 = BASE64_STANDARD.encode(&content_bytes);
                        self.broadcast_content_response(content_key, content_base64);
                    }
                    Err(_) => {
                        info!(%content_key, "ContentRequest received but content not found locally");
                    }
                }
            }
            SyncMsg::ContentResponse {
                content_id,
                content,
            } => {
                // Compatibility mode: field name is `content_id`, value is interpreted as pinboard content_key.
                let content_key = content_id;
                info!(
                    %content_key,
                    content_size = content.len(),
                    "P2P ContentResponse received"
                );

                if capacity_manager
                    .get_content_from_slots(&content_key)
                    .await
                    .is_ok()
                {
                    info!(%content_key, "Slot content already exists locally, ignoring ContentResponse");
                    return;
                }

                let content_bytes = match BASE64_STANDARD.decode(&content) {
                    Ok(content_bytes) => content_bytes,
                    Err(e) => {
                        error!(
                            %content_key,
                            error = %e,
                            "Failed to decode base64 content from ContentResponse"
                        );
                        return;
                    }
                };

                let computed_key = hex::encode(Sha256::digest(&content_bytes));
                if computed_key != content_key {
                    warn!(
                        %content_key,
                        %computed_key,
                        decoded_size = content_bytes.len(),
                        "Rejecting ContentResponse: sha256(content) did not match content_key"
                    );
                    return;
                }
                info!(
                    %content_key,
                    decoded_size = content_bytes.len(),
                    "Verified ContentResponse hash matches content_key"
                );

                match capacity_manager.get_content_from_slots(&content_key).await {
                    Ok(_) => {
                        info!(
                            content_key = %content_key,
                            "P2P pinboard capacity mirror: content already in slots, skipping store_content_chunks"
                        );
                    }
                    Err(_) => {
                        let chunks: Vec<Vec<u8>> = content_bytes
                            .chunks(MAX_CHUNK_SIZE)
                            .map(|c| c.to_vec())
                            .collect();
                        match capacity_manager
                            .store_content_chunks(content_key.clone(), chunks)
                            .await
                        {
                            Ok(_) => {
                                info!(
                                    content_key = %content_key,
                                    byte_len = content_bytes.len(),
                                    "PINBOARD_P2P_SYNC_CAPACITY_WRITE_OK: pinboard content stored to capacity slots after ContentResponse"
                                );
                            }
                            Err(e) => {
                                error!(
                                    content_key = %content_key,
                                    error = %e,
                                    "P2P pinboard capacity store failed after ContentResponse"
                                );
                            }
                        }
                    }
                }

                // Remove from missing content list (best effort).
                if let Ok(mut tracker) = missing_content_tracker.lock() {
                    if let Err(e) = tracker.mark_content_synced_by_content_id(&content_key) {
                        warn!(
                            content_id = %content_key,
                            error = %e,
                            "Failed to remove content from missing list"
                        );
                    }
                }
            }
            SyncMsg::CapacityChallenge {
                challenge_id: challenge_id_str,
                challenger,
                provider_id: provider,
                chunk_indices,
                block_height,
                merkle_root,
                seed: _,
                expiration_block,
                timestamp,
            } => {
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
                    "✅ XZXZ2 CAPACITY PROVIDER: Received capacity challenge via P2P"
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
            SyncMsg::CapacityChallengeResponse {
                challenge_id,
                provider_id: provider,
                challenger,
                block_height,
                proofs,
                generated_at,
                provider_pubkey,
                provider_signature,
            } => {
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
                    "✅ Active capacity validator: Received capacity challenge proof response via P2P"
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
                            "✅ Proof validation succeeded"
                        );
                        info!(
                            challenge_id = %challenge_id,
                            capacity_provider = %provider,
                            "XWXW3 Successful validator of storage proof"
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
                            "❌ Proof validation failed"
                        );
                        error!(
                            challenge_id = %challenge_id,
                            capacity_provider = %provider,
                            "XWXW2 Failed validation of storage proof"
                        );
                        // TODO: Record failure in challenge tracker and check threshold for slashing
                    }
                }
            }
            SyncMsg::ContentInventoryRequest { timestamp } => {
                info!(
                    timestamp = timestamp,
                    "P2P ContentInventoryRequest received"
                );
                // Legacy manifest inventory has been removed; respond with empty list.
                let items_json = match serde_json::to_string(&Vec::<serde_json::Value>::new()) {
                    Ok(json) => json,
                    Err(e) => {
                        error!(
                            error = %e,
                            "Failed to serialize content inventory items to JSON"
                        );
                        "[]".to_string() // Send empty array on serialization error
                    }
                };

                info!(
                    item_count = 0,
                    last_updated_block = 0,
                    "Sending ContentInventoryResponse"
                );

                // Publish response on P2P_TOPIC_CONTENT_SYNC
                let response = SyncMsg::ContentInventoryResponse { items_json };
                if let Err(e) = self.publish_to_topic(P2P_TOPIC_CONTENT_SYNC, response) {
                    error!(
                        error = %e,
                        "Failed to publish ContentInventoryResponse"
                    );
                }
            }
            SyncMsg::ContentInventoryResponse { items_json } => {
                info!(
                    items_json_len = items_json.len(),
                    "P2P ContentInventoryResponse received"
                );
                // Parse and log the inventory items
                match serde_json::from_str::<Vec<serde_json::Value>>(&items_json) {
                    Ok(items) => {
                        info!(
                            item_count = items.len(),
                            "ContentInventoryResponse contains {} items",
                            items.len()
                        );
                        if items.len() > 5 {
                            info!("... and {} more items", items.len() - 5);
                        }
                    }
                    Err(e) => {
                        error!(
                            error = %e,
                            "Failed to parse ContentInventoryResponse JSON"
                        );
                    }
                }
            }
        }
    }
}

impl P2pCoordinatorTrait for P2pSyncCoordinator {
    fn broadcast_announce(&self, key: String) {
        let _ = self.cmd_tx.send(P2pCommand::BroadcastAnnounce { key });
    }

    fn broadcast_content_request(&self, key: String) {
        let _ = self
            .cmd_tx
            .send(P2pCommand::BroadcastContentRequest { key });
    }

    fn broadcast_content_response(&self, key: String, content: String) {
        let _ = self
            .cmd_tx
            .send(P2pCommand::BroadcastContentResponse { key, content });
    }

    fn find_pinboard_and_broadcast_announce(
        &self,
        pinboard_meta_cache: &BTreeMap<String, PinboardMessageMetadata>,
    ) {
        let cache_size = pinboard_meta_cache.len();
        let mut announced = 0u32;
        for (message_id, meta) in pinboard_meta_cache {
            info!(
                message_id = %message_id,
                content_key = %meta.content_key,
                "Announcing pinboard blob via P2P"
            );
            // Compatibility mode: continue using SyncMsg::Announce { content_id } where the field
            // carries the pinboard content_key.
            self.broadcast_announce(meta.content_key.clone());
            announced += 1;
        }
        info!(
            cache_size = cache_size,
            announced = announced,
            "Pinboard announce scan finished"
        );
    }

    fn subscribe_to_topic(&self, topic: &str) -> Result<(), String> {
        self.cmd_tx
            .send(P2pCommand::SubscribeToTopic {
                topic: topic.to_string(),
            })
            .map_err(|e| format!("Failed to send subscribe command: {e}"))?;
        Ok(())
    }

    fn publish_to_topic(&self, topic: &str, msg: SyncMsg) -> Result<(), String> {
        info!("XZXZ12: publish_to_topic called topic={}", topic);
        match self.cmd_tx.send(P2pCommand::PublishToTopic {
            topic: topic.to_string(),
            msg,
        }) {
            Ok(_) => {
                info!("XZXZ13: PublishToTopic command sent to channel successfully");
                Ok(())
            }
            Err(e) => {
                warn!(
                    "XZXZ14: Failed to send PublishToTopic command to channel: {}",
                    e
                );
                Err(format!("Failed to send command: {e}"))
            }
        }
    }

    fn run_message_handler(
        self: Arc<Self>,
        capacity_manager: Arc<CapacityManager>,
        missing_content_tracker: Arc<Mutex<MissingContentTracker>>,
        msg_rx: mpsc::UnboundedReceiver<SyncMsg>,
    ) -> tokio::task::JoinHandle<()> {
        Self::run_message_handler(self, capacity_manager, missing_content_tracker, msg_rx)
    }
}

/// Mock P2P coordinator for single-node mode
/// Implements the same interface as P2pSyncCoordinator but does nothing
#[derive(Debug)]
pub struct MockP2pSyncCoordinator;

impl MockP2pSyncCoordinator {
    /// Create a new mock P2P coordinator
    pub fn new() -> (Self, mpsc::UnboundedReceiver<SyncMsg>) {
        let (_msg_tx, msg_rx) = mpsc::unbounded_channel();
        (Self, msg_rx)
    }
}

impl P2pCoordinatorTrait for MockP2pSyncCoordinator {
    fn broadcast_announce(&self, _key: String) {
        // No-op in single-node mode
    }

    fn broadcast_content_request(&self, _key: String) {
        // No-op in single-node mode
    }

    fn broadcast_content_response(&self, _key: String, _content: String) {
        // No-op in single-node mode
    }

    fn find_pinboard_and_broadcast_announce(
        &self,
        _pinboard_meta_cache: &BTreeMap<String, PinboardMessageMetadata>,
    ) {
        // No-op in single-node mode
    }

    fn subscribe_to_topic(&self, _topic: &str) -> Result<(), String> {
        // No-op in single-node mode
        Ok(())
    }

    fn publish_to_topic(&self, _topic: &str, _msg: SyncMsg) -> Result<(), String> {
        // No-op in single-node mode
        info!("XZXZ15: Mock sync coordinator");
        Ok(())
    }

    fn run_message_handler(
        self: Arc<Self>,
        capacity_manager: Arc<CapacityManager>,
        missing_content_tracker: Arc<Mutex<MissingContentTracker>>,
        msg_rx: mpsc::UnboundedReceiver<SyncMsg>,
    ) -> tokio::task::JoinHandle<()> {
        Self::run_message_handler(self, capacity_manager, missing_content_tracker, msg_rx)
    }
}

impl MockP2pSyncCoordinator {
    /// Start the message handler task that processes incoming P2P sync messages
    pub fn run_message_handler(
        self: Arc<Self>,
        _capacity_manager: Arc<CapacityManager>,
        _missing_content_tracker: Arc<Mutex<MissingContentTracker>>,
        mut msg_rx: mpsc::UnboundedReceiver<SyncMsg>,
    ) -> tokio::task::JoinHandle<()> {
        // In single-node mode, just consume messages without processing
        tokio::spawn(async move {
            while let Some(_msg) = msg_rx.recv().await {
                // Messages are consumed but not processed in single-node mode
            }
        })
    }
}
