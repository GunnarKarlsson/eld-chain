//! Orchestration and CLI command implementations for [`super::ChainClient`].
//!
//! Split into submodules by area: accounts, transactions, epoch views, pinboard,
//! CADO exploration, and shared [`util`] helpers.

mod accounts;
mod cado;
mod epoch;
mod namespace;
mod pinboard;
mod transactions;
mod util;

pub(crate) use accounts::{
    display_account, display_wallet_by_name, get_abci_info, get_account,
    get_provider_id_for_capacity, get_staking_account,
};
pub(crate) use cado::{get_cado, list_cados};
pub(crate) use epoch::{view_active_validators, view_epoch, view_epoch_info};
pub(crate) use namespace::{add_namespace, get_namespace};
pub(crate) use pinboard::{
    get_content, pinboard_get_post, pinboard_list_by_tag, pinboard_list_by_wallet,
    post_pinboard_message,
};
pub(crate) use transactions::{list_all_transactions, list_transactions, stake, transfer, unstake};
