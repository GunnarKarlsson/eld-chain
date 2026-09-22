use super::rewards::ValidatorRewardManager;
use crate::abci_interface::chain_tip::ChainTip;
use crate::abci_interface::snapshot::SnapshotManager;
use crate::app_state::AppState;
use crate::capacity::capacity_manager::CapacityManager;
use crate::config::ConsensusConfig;
use crate::content::sync::P2pCoordinatorTrait;
use crate::node_identity::LocalNodeIdentity;
use crate::storage::traits::ConsensusConnectionStorage;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use tokio::sync::oneshot;

pub struct ConsensusConnection<S>
where
    S: ConsensusConnectionStorage,
{
    pub consensus_config: Arc<Mutex<ConsensusConfig>>,
    pub committed_state: Arc<Mutex<AppState>>,
    pub chain_tip: Arc<ChainTip>,
    pub current_state: Arc<Mutex<Option<AppState>>>,
    pub storage: Arc<S>,
    pub reward_manager: ValidatorRewardManager<S>,
    pub snapshot_manager: Arc<SnapshotManager<S>>,
    pub p2p_sync_coordinator: Arc<dyn P2pCoordinatorTrait>,
    pub(crate) last_commit_time: Arc<Mutex<Instant>>,
    pub(crate) cado_type_counts: Arc<Mutex<HashMap<String, usize>>>,
    pub(crate) ready_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    /// Promoted pinboard blobs are also mirrored into capacity slots after RocksDB write.
    pub(crate) capacity_manager: Arc<CapacityManager>,
    /// Paired Tendermint validator identity (from `/status`); not part of app state.
    pub(crate) local_identity: Arc<RwLock<LocalNodeIdentity>>,
}

/// Construction inputs for [`ConsensusConnection::new`].
pub struct ConsensusConnectionNewContext<S>
where
    S: ConsensusConnectionStorage,
{
    pub consensus_config: Arc<Mutex<ConsensusConfig>>,
    pub committed_state: Arc<Mutex<AppState>>,
    pub chain_tip: Arc<ChainTip>,
    pub current_state: Arc<Mutex<Option<AppState>>>,
    pub storage: Arc<S>,
    pub snapshot_manager: Arc<SnapshotManager<S>>,
    pub p2p_sync_coordinator: Arc<dyn P2pCoordinatorTrait>,
    pub ready_tx: Option<oneshot::Sender<()>>,
    pub capacity_manager: Arc<CapacityManager>,
    pub local_identity: Arc<RwLock<LocalNodeIdentity>>,
}

impl<S> ConsensusConnection<S>
where
    S: ConsensusConnectionStorage,
{
    pub fn new(ctx: ConsensusConnectionNewContext<S>) -> Self {
        Self {
            consensus_config: ctx.consensus_config,
            committed_state: ctx.committed_state,
            chain_tip: ctx.chain_tip,
            current_state: ctx.current_state,
            storage: ctx.storage.clone(),
            reward_manager: ValidatorRewardManager::new(ctx.storage),
            snapshot_manager: ctx.snapshot_manager,
            p2p_sync_coordinator: ctx.p2p_sync_coordinator,
            last_commit_time: Arc::new(Mutex::new(Instant::now())),
            cado_type_counts: Arc::new(Mutex::new(HashMap::new())),
            ready_tx: Arc::new(Mutex::new(ctx.ready_tx)),
            capacity_manager: ctx.capacity_manager,
            local_identity: ctx.local_identity,
        }
    }
}
