//! HTTP client, CWD config, and wallet-file I/O for Eld nodes, CLI, and faucet.
//!
//! Protocol types live in `eld_common`. This crate talks to a running node.
//!
//! - [`api::abci`] — Tendermint RPC / ABCI
//! - [`api::rest`] — node app REST and faucet HTTP
//! - [`facade`] — [`ChainClient`], [`crate::facade::cli`], and mixed command wrappers
//!
//! Wallet identity types are in `eld_common::wallet`; this crate loads `wallets.json`.

#![warn(unreachable_pub)]

pub mod api;
pub mod config;
pub mod endpoint;
pub mod facade;
pub mod logging;
pub mod wallet_store_config;

pub use facade::ChainClient;
