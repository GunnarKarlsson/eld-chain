//! Chain RPC client and helpers split into focused submodules.
//!
//! Prefer [`ChainClient`] (re-exported as [`crate::cli::Cli`] for compatibility).

pub mod chain_client;
pub mod faucet;
pub mod flows;
pub mod query;
pub mod tx_broadcast;
pub mod wallets;

pub use chain_client::ChainClient;
