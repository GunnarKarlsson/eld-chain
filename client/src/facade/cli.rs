//! Back-compat barrel: prefer [`crate::facade::ChainClient`] and [`crate::config::client_config`].
//!
//! Historical name `Cli` is a type alias for [`crate::facade::ChainClient`].

pub use crate::config::client_config::{
    get_config, get_config_from_arg, CliConfig, ConsensusConfig, FeeConfig,
    BROADCAST_TX_COMMIT_WITH_BODY_URL_PATH, CONSENSUS_CONFIG_PATH, DEFAULT_CONFIG_PATH, HEX_PREFIX,
    WALLETS_PATH,
};
pub use crate::facade::ChainClient;
pub use eld_common::error::{EldError, ErrorBuilder};

/// Historical name for [`ChainClient`]; kept for existing `use eld_client::facade::cli::Cli` imports.
pub type Cli = ChainClient;
