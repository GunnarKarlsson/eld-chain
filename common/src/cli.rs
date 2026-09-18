//! Back-compat barrel: prefer [`crate::client::ChainClient`] and [`crate::client_config`].
//!
//! Historical name `Cli` is a type alias for [`crate::client::ChainClient`].

pub use crate::client::ChainClient;
pub use crate::client_config::{
    get_config, get_config_from_arg, CliConfig, ConsensusConfig, FeeConfig,
    BROADCAST_TX_COMMIT_WITH_BODY_URL_PATH, CONSENSUS_CONFIG_PATH, DEFAULT_CONFIG_PATH, HEX_PREFIX,
    WALLETS_PATH,
};
pub use crate::error::{EldError, ErrorBuilder};

/// Historical name for [`ChainClient`]; kept for existing `use eld_common::cli::Cli` imports.
pub type Cli = ChainClient;
