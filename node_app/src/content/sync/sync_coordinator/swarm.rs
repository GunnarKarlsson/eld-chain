use super::behaviour::{EldBehaviour, EldBehaviourEvent};
use super::commands::P2pCommand;
use super::SyncMsg;
use eld_common::constants::p2p::{P2P_TOPIC_CONTENT_SYNC, P2P_TOPIC_CONTENT_SYNC_RESPONSE};
use libp2p::futures::StreamExt;
use libp2p::gossipsub::IdentTopic;
use libp2p::{gossipsub, mdns, swarm::SwarmEvent, PeerId, Swarm};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

type SwarmTaskChannels = (
    tokio::task::JoinHandle<()>,
    mpsc::UnboundedSender<P2pCommand>,
    mpsc::UnboundedReceiver<SyncMsg>,
);

pub(crate) fn spawn_swarm_task(
    mut swarm: Swarm<EldBehaviour>,
    local_peer_id: PeerId,
    tcp_port: u16,
    udp_port: u16,
) -> Result<SwarmTaskChannels, Box<dyn std::error::Error + Send + Sync>> {
    swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{tcp_port}").parse()?)?;
    swarm.listen_on(format!("/ip4/0.0.0.0/udp/{udp_port}/quic-v1").parse()?)?;

    let topic = IdentTopic::new(P2P_TOPIC_CONTENT_SYNC);
    swarm.behaviour_mut().gossipsub.subscribe(&topic)?;
    info!("Subscribed to topic: {}", P2P_TOPIC_CONTENT_SYNC);

    let response_topic = IdentTopic::new(P2P_TOPIC_CONTENT_SYNC_RESPONSE);
    swarm.behaviour_mut().gossipsub.subscribe(&response_topic)?;
    info!("Subscribed to topic: {}", P2P_TOPIC_CONTENT_SYNC_RESPONSE);

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
                            info!("P2P listening on {}", address);
                        }
                        SwarmEvent::Behaviour(EldBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                            for (peer_id, addr) in list {
                                info!("mDNS discovered peer {} at {}", peer_id, addr);
                                info!("Our peer ID: {}", local_peer_id);
                                if peer_id == local_peer_id {
                                    info!("Ignoring self-discovery");
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
                            info!("Peer {} subscribed to topic {}", peer_id, topic);
                        }
                        SwarmEvent::Behaviour(EldBehaviourEvent::Gossipsub(gossipsub::Event::Unsubscribed { peer_id, topic })) => {
                            info!("Peer {} unsubscribed from topic {}", peer_id, topic);
                        }
                        SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                            info!(
                                "Connection established with peer {} via {}",
                                peer_id,
                                endpoint.get_remote_address()
                            );
                            // Add peer as explicit peer to GossipSub so it stays connected
                            swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                            info!("Added peer {} as explicit GossipSub peer", peer_id);
                        }
                        SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                            info!("Connection closed with peer {}: {:?}", peer_id, cause);
                        }
                        SwarmEvent::OutgoingConnectionError { peer_id, error, connection_id: _, .. } => {
                            warn!("Failed to connect to peer {:?}: {}", peer_id, error);
                        }
                        SwarmEvent::IncomingConnectionError { error, .. } => {
                            warn!("Incoming connection error: {}", error);
                        }
                        _ => {
                            debug!("Other swarm event received");
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
                            debug!(topic = %topic_str, "SubscribeToTopic command received");
                            let challenge_topic = IdentTopic::new(&topic_str);
                            let topic_hash = challenge_topic.hash();
                            debug!(topic = %topic_str, hash = %topic_hash, "Created IdentTopic for subscription");
                            match swarm.behaviour_mut().gossipsub.subscribe(&challenge_topic) {
                                Ok(_) => {
                                    debug!(topic = %topic_str, hash = %topic_hash, "Subscribed to topic");
                                    // Track subscription ourselves
                                    subscribed_topics.lock().unwrap().insert(topic_str.clone());
                                }
                                Err(e) => {
                                    warn!(
                                        topic = %topic_str,
                                        hash = %topic_hash,
                                        error = %e,
                                        "Failed to subscribe to topic"
                                    );
                                }
                            }
                        }
                        P2pCommand::PublishToTopic { topic: topic_str, msg } => {
                            debug!(topic = %topic_str, "Received PublishToTopic command");
                            if let Ok(data) = bincode::serialize(&msg) {
                                debug!(size = data.len(), "Message serialized successfully");
                                let topic_ident = IdentTopic::new(&topic_str);
                                debug!(topic = %topic_str, hash = %topic_ident.hash(), "Created IdentTopic for publish");
                                // Check if we're subscribed to this topic using our own tracking
                                let is_subscribed = subscribed_topics.lock().unwrap().contains(&topic_str);
                                debug!(
                                    subscribed_count = subscribed_topics.lock().unwrap().len(),
                                    is_subscribed,
                                    topic = %topic_str,
                                    hash = %topic_ident.hash(),
                                    "Currently subscribed topics"
                                );

                                // Clone data for potential manual injection
                                let data_clone = data.clone();

                                // Try to publish - even if we get InsufficientPeers, the message might still be delivered locally
                                match swarm.behaviour_mut().gossipsub.publish(topic_ident, data) {
                                    Ok(message_id) => {
                                        debug!(topic = %topic_str, ?message_id, "Published message to topic");
                                    }
                                    Err(e) => {
                                        // Even if publish fails with InsufficientPeers, manually inject the message locally
                                        // if we're subscribed (for single-node testing)
                                        if is_subscribed {
                                            warn!(
                                                topic = %topic_str,
                                                error = %e,
                                                "Failed to publish (subscribed; injecting locally)"
                                            );
                                            // Manually send the message to our handler since gossipsub isn't delivering it locally
                                            if let Ok(sync_msg) = bincode::deserialize::<SyncMsg>(&data_clone) {
                                                debug!(topic = %topic_str, "Manually injecting message locally");
                                                let _ = msg_tx.send(sync_msg);
                                            }
                                        } else {
                                            warn!(
                                                topic = %topic_str,
                                                error = %e,
                                                "Failed to publish to topic (not subscribed)"
                                            );
                                        }
                                    }
                                }
                            } else {
                                warn!(topic = %topic_str, "Failed to serialize message for topic");
                            }
                        }
                    }
                }
            }
        }
    });

    Ok((swarm_task, cmd_tx, msg_rx))
}
