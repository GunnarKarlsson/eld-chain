//! Axum application state and [`FromRef`] sub-extracts for HTTP handlers.

use crate::app_state::AppState;
use crate::capacity::capacity_manager::CapacityManager;
use crate::config::ConsensusConfig;
use crate::indexer::TransactionIndexer;
use crate::node_identity::LocalNodeIdentity;
use crate::storage::rocksdb::RocksDBStorage;
use axum::extract::FromRef;
use eld_client::facade::cli::Cli;
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::RwLock as TokioRwLock;

use super::RateLimitState;

/// Shared state for the content/upload HTTP API (all routes on that listener).
#[derive(Clone)]
pub struct ApiState {
    pub storage: Arc<RocksDBStorage>,
    pub consensus_config: Arc<Mutex<ConsensusConfig>>,
    pub rate_limit_state: Arc<TokioRwLock<RateLimitState>>,
    pub cli: Arc<Cli>,
    pub capacity_manager: Arc<CapacityManager>,
    pub local_identity: Arc<RwLock<LocalNodeIdentity>>,
    pub admin_token: Option<String>,
    pub indexer: Option<Arc<TransactionIndexer>>,
    /// Committed application state (namespace registry, CADO cache).
    pub committed_state: Arc<Mutex<AppState>>,
    /// In-flight block state (includes same-block namespace staging).
    pub current_state: Arc<Mutex<Option<AppState>>>,
}

impl FromRef<ApiState> for Arc<RocksDBStorage> {
    fn from_ref(state: &ApiState) -> Self {
        state.storage.clone()
    }
}

impl FromRef<ApiState> for Arc<Mutex<ConsensusConfig>> {
    fn from_ref(state: &ApiState) -> Self {
        state.consensus_config.clone()
    }
}

impl FromRef<ApiState> for Arc<TokioRwLock<RateLimitState>> {
    fn from_ref(state: &ApiState) -> Self {
        state.rate_limit_state.clone()
    }
}

impl FromRef<ApiState> for Arc<Cli> {
    fn from_ref(state: &ApiState) -> Self {
        state.cli.clone()
    }
}

impl FromRef<ApiState> for Arc<CapacityManager> {
    fn from_ref(state: &ApiState) -> Self {
        state.capacity_manager.clone()
    }
}

impl FromRef<ApiState> for Arc<RwLock<LocalNodeIdentity>> {
    fn from_ref(state: &ApiState) -> Self {
        state.local_identity.clone()
    }
}

impl FromRef<ApiState> for Option<String> {
    fn from_ref(state: &ApiState) -> Self {
        state.admin_token.clone()
    }
}

impl FromRef<ApiState> for Option<Arc<TransactionIndexer>> {
    fn from_ref(state: &ApiState) -> Self {
        state.indexer.clone()
    }
}

impl FromRef<ApiState> for Arc<Mutex<AppState>> {
    fn from_ref(state: &ApiState) -> Self {
        state.committed_state.clone()
    }
}

impl FromRef<ApiState> for Arc<Mutex<Option<AppState>>> {
    fn from_ref(state: &ApiState) -> Self {
        state.current_state.clone()
    }
}
