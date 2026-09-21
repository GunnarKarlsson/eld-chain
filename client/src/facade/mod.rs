//! `ChainClient` and domain command wrappers (may call ABCI and REST).
//!
//! Config helpers used by node/CLI binaries are re-exported here for a short import path.

mod accounts;
mod cado;
mod chain_client;
mod epoch;
mod namespace;
mod pinboard;
mod submitted_tx;
mod transactions;
mod util;
pub(crate) mod wallets;

pub use chain_client::ChainClient;
pub use namespace::NamespaceLookup;
pub use submitted_tx::SubmittedTx;

pub use crate::config::client_config::{
    get_config, get_config_from_arg, CliConfig, ConsensusConfig, FeeConfig,
    BROADCAST_TX_COMMIT_WITH_BODY_URL_PATH, CONSENSUS_CONFIG_PATH, DEFAULT_CONFIG_PATH, HEX_PREFIX,
    WALLETS_PATH,
};

pub(crate) use accounts::{
    get_abci_info, get_account, get_provider_id_for_capacity, get_staking_account,
};
pub(crate) use cado::{get_cado, list_cados};
pub(crate) use epoch::{view_active_validators, view_epoch, view_epoch_info};
pub(crate) use namespace::{add_namespace, get_namespace};
pub(crate) use pinboard::{
    get_content, pinboard_get_post, pinboard_list_by_tag, pinboard_list_by_wallet,
    post_pinboard_message,
};
pub(crate) use transactions::{list_all_transactions, list_transactions, stake, transfer, unstake};
