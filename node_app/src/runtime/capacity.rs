use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use eld_client::facade::ChainClient;
use eld_common::capacity::CapacityConfig;
use eld_common::error::EldError;
use tracing::{error, info};

use crate::capacity::capacity_manager::CapacityManager;
use crate::config::{ConsensusConfig, NodeRuntimeConfig};
use crate::errors::handle_fatal_eld_error;
use crate::node_identity::{
    capacity_validator_address_from_wallet, resolve_capacity_validator_wallet_name_from_env,
    LocalNodeIdentity, ELD_CAPACITY_VALIDATOR_WALLET_NAME_ENV,
};

pub struct CapacityRuntime {
    pub manager: Arc<CapacityManager>,
    pub validator_wallet_name: String,
    pub local_identity: Arc<RwLock<LocalNodeIdentity>>,
}

pub async fn init_capacity(
    node_config: &NodeRuntimeConfig,
    client_config: &eld_client::config::ClientConfig,
    cli: Arc<ChainClient>,
    consensus_config: Arc<Mutex<ConsensusConfig>>,
) -> CapacityRuntime {
    let (capacity_size_mb, capacity_storage_path) = match (
        node_config.capacity_size_mb,
        node_config.capacity_storage_path.as_ref(),
    ) {
        (Some(mb), Some(path)) => (mb, path.clone()),
        _ => handle_fatal_eld_error(EldError::InitializationError {
            component: "capacity".into(),
            details: "capacity_size_mb and capacity_storage_path must both be set in config.json (required for capacity manager)".into(),
        }),
    };

    let capacity_validator_wallet_name = resolve_capacity_validator_wallet_name_from_env()
        .unwrap_or_else(|e| handle_fatal_eld_error(e));

    let capacity_validator_wallet = match cli
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
        max_capacity_gb: capacity_size_mb / 1024,
        provider_id: capacity_validator_wallet_address,
        auto_register: true,
        registration_retry_interval_secs: 60,
        tendermint_rpc_url: format!(
            "http://{}:{}",
            client_config.node_host, client_config.node_port
        ),
    };

    let manager = CapacityManager::new(
        capacity_config,
        capacity_validator_wallet_name.clone(),
        cli,
        consensus_config,
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

    let local_identity = Arc::new(RwLock::new(LocalNodeIdentity {
        consensus_validator_address: None,
        capacity_validator_address: Some(capacity_validator_wallet_address),
    }));

    CapacityRuntime {
        manager: Arc::new(manager),
        validator_wallet_name: capacity_validator_wallet_name,
        local_identity,
    }
}
