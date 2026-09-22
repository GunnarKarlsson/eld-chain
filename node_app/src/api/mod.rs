//! HTTP API (Axum routers and handlers).

pub mod admin;
pub mod api_rate_limiting;
pub mod cado;
pub mod epoch;
pub mod error;
pub mod health;
pub mod indexer;
pub mod namespace;
pub mod pagination;
pub mod pinboard;
pub mod slots;
pub mod state;

pub(crate) use api_rate_limiting::check_rate_limit;
pub use api_rate_limiting::{ApiEndpointType, RateLimitState};
pub(crate) use cado::lookup_cado_by_path_excluding_current_cache;
#[allow(unused_imports)]
pub use error::{ApiError, ApiErrorResponse};
pub use state::ApiState;

use crate::capacity::capacity_manager::CapacityManager;
use crate::config::ConsensusConfig;
use crate::indexer::TransactionIndexer;
use crate::node_identity::LocalNodeIdentity;
use crate::storage::rocksdb::RocksDBStorage;
use axum::{
    routing::{get, post},
    Router,
};
use eld_client::facade::ChainClient;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};

/// Inputs for [`init_router_with_storage`].
pub struct ApiRouterInitContext {
    pub storage: Arc<RocksDBStorage>,
    pub consensus_config: Arc<Mutex<ConsensusConfig>>,
    pub rate_limit_state: Arc<RwLock<RateLimitState>>,
    pub indexer: Option<Arc<TransactionIndexer>>,
    pub cli: Arc<ChainClient>,
    pub capacity_manager: Arc<CapacityManager>,
    pub local_identity: Arc<std::sync::RwLock<LocalNodeIdentity>>,
    pub admin_token: Option<String>,
    pub committed_state: Arc<Mutex<crate::app_state::AppState>>,
    pub current_state: Arc<Mutex<Option<crate::app_state::AppState>>>,
}

/// Initialize router with storage access for content browser API.
///
/// Pinboard **message bytes** in GET/submit flows are read from [`CapacityManager::get_content_from_slots`]
/// only (not from RocksDB `pinboard_blob`).
pub fn init_router_with_storage(ctx: ApiRouterInitContext) -> Router {
    let ApiRouterInitContext {
        storage,
        consensus_config,
        rate_limit_state,
        indexer,
        cli,
        capacity_manager,
        local_identity,
        admin_token,
        committed_state,
        current_state,
    } = ctx;

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app_state = ApiState {
        storage: storage.clone(),
        consensus_config,
        rate_limit_state: rate_limit_state.clone(),
        cli,
        capacity_manager,
        local_identity,
        admin_token,
        indexer,
        committed_state,
        current_state,
    };

    // Base router: pinboard + common endpoints + admin (same listener as upload/content port)
    let base_router = Router::new()
        .route("/health", get(health::health))
        .route("/node_identity", get(health::handle_get_node_identity))
        .route(
            "/v1/pinboard/messages:submit",
            post(pinboard::handle_pinboard_submit),
        )
        .route(
            "/v1/pinboard/posts",
            get(pinboard::handle_get_pinboard_posts),
        )
        .route(
            "/v1/pinboard/post",
            get(pinboard::handle_get_pinboard_post_by_path),
        )
        .route("/v1/namespaces", get(namespace::handle_list_namespaces))
        .route(
            "/v1/namespace/{namespace_slug}",
            get(namespace::handle_get_namespace),
        )
        .route("/admin/v1/status", get(admin::handle_admin_status));

    let transaction_router = Router::new()
        .route("/transactions", get(indexer::handle_get_transactions))
        .route("/transaction", get(indexer::handle_get_transaction))
        .route("/events", get(indexer::handle_get_events))
        .route(
            "/v1/capacity/verified-proof-rewards/sum",
            get(indexer::handle_get_verified_proof_rewards_sum),
        )
        .route(
            "/v1/capacity/verified-proof-rewards/{address}",
            get(indexer::handle_get_verified_proof_rewards),
        )
        .route("/epochs", get(epoch::handle_get_epochs))
        .route("/epoch/current", get(epoch::handle_get_epoch_current))
        .route("/epoch/height", get(epoch::handle_get_epoch_by_height))
        .route("/epoch/{epoch}", get(epoch::handle_get_epoch_by_number))
        .route("/slotallocation", get(slots::handle_get_slot_allocation));

    base_router
        .merge(transaction_router)
        .merge(cado::init_cado_routes())
        .with_state(app_state)
        .layer(cors)
}
