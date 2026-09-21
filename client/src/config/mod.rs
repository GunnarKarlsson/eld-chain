//! CWD JSON config: [`client_config`] types and [`config_loader`].

pub mod client_config;
pub mod config_loader;

pub use client_config::{
    get_client_setup, get_client_setup_from_arg, get_config, get_config_from_arg,
    load_client_setup, CliConfig, ClientSetup, ConsensusConfig, FeeConfig,
    BROADCAST_TX_COMMIT_WITH_BODY_URL_PATH, CONSENSUS_CONFIG_PATH, DEFAULT_CONFIG_PATH, HEX_PREFIX,
    WALLETS_PATH,
};
