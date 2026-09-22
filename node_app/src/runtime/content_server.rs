use std::sync::Arc;

use eld_client::facade::ChainClient;
use eld_common::error::EldError;
use tokio::net::TcpListener;
use tracing::{error, info};

use crate::api::api_rate_limiting::create_api_rate_limit_config_from_env;
use crate::api::{init_router_with_storage, ApiRouterInitContext, RateLimitState};
use crate::app_state::AppState;
use crate::capacity::capacity_manager::CapacityManager;
use crate::config::ConsensusConfig;
use crate::indexer::TransactionIndexer;
use crate::node_identity::LocalNodeIdentity;
use crate::storage::rocksdb::RocksDBStorage;

pub struct ContentServerBindContext {
    pub rocks_db_storage: Arc<RocksDBStorage>,
    pub consensus_config: Arc<std::sync::Mutex<ConsensusConfig>>,
    pub transaction_indexer: Option<Arc<TransactionIndexer>>,
    pub cli: Arc<ChainClient>,
    pub capacity_manager: Arc<CapacityManager>,
    pub local_identity: Arc<std::sync::RwLock<LocalNodeIdentity>>,
    pub committed_state: Arc<std::sync::Mutex<AppState>>,
    pub current_state: Arc<std::sync::Mutex<Option<AppState>>>,
    pub app_port: String,
}

pub async fn bind_content_server(
    ctx: ContentServerBindContext,
) -> Result<
    impl std::future::Future<
            Output = std::result::Result<(), Box<dyn std::error::Error + Send + Sync>>,
        > + Send,
    EldError,
> {
    let ContentServerBindContext {
        rocks_db_storage,
        consensus_config,
        transaction_indexer,
        cli,
        capacity_manager,
        local_identity,
        committed_state,
        current_state,
        app_port,
    } = ctx;

    let rate_limit_config = create_api_rate_limit_config_from_env();
    let rate_limit_state = Arc::new(tokio::sync::RwLock::new(RateLimitState::new(
        rate_limit_config.clone(),
    )));

    let admin_token = std::env::var(crate::api::admin::ELD_ADMIN_TOKEN_ENV)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let router = init_router_with_storage(ApiRouterInitContext {
        storage: rocks_db_storage,
        consensus_config,
        rate_limit_state,
        indexer: transaction_indexer,
        cli,
        capacity_manager,
        local_identity,
        admin_token,
        committed_state,
        current_state,
    });

    info!(
        "Content server rate limiting enabled: {} requests/minute",
        rate_limit_config.general_requests_per_minute
    );

    let app_addr = format!("0.0.0.0:{app_port}");

    let listener = TcpListener::bind(app_addr.clone()).await.map_err(|e| {
        error!("Failed to bind content server to {}: {}", app_addr, e);
        match e.kind() {
            std::io::ErrorKind::AddrInUse => EldError::InitializationError {
                component: "content server".to_string(),
                details: format!(
                    "Port {app_port} is already in use. Check if another instance is running."
                ),
            },
            std::io::ErrorKind::PermissionDenied => EldError::InitializationError {
                component: "content server".to_string(),
                details: format!(
                    "Permission denied to bind to port {app_port}. Check if you have sufficient privileges."
                ),
            },
            _ => EldError::InitializationError {
                component: "content server".to_string(),
                details: format!("Failed to start content server: {e}"),
            },
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
    Ok(async move {
        axum::serve(listener, router)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
}
