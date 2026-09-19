//! Shared types, transaction wire format, validation, and client helpers for Eld nodes and the CLI.
//!
//! The `unreachable_pub` lint keeps implementation details (for example serde helpers in private
//! modules) at `pub(super)` visibility unless other crates need to name them.

#![warn(unreachable_pub)]

pub mod abci_api;
pub mod account;
pub mod address;
pub mod app_api;
pub mod cado;
pub mod capacity;
pub mod capacity_challenge;
pub mod capacity_merkle_root;
pub mod capacity_proof;
pub mod capacity_seed;
pub mod challenge_id;
pub mod cli;
pub mod client;
pub mod client_config;
pub mod coin;
pub mod config_loader;
pub mod constants;
pub mod content_id;
pub mod contract;
pub mod contract_id;
pub mod device;
pub mod endpoint;
pub mod error;
pub mod fee;
mod hex_encoding;
pub mod logging;
pub mod manifest_id;
pub mod missing_content;
pub mod namespace;
pub mod namespace_api;
pub mod nonce;
pub mod pinboard;
pub mod pinboard_api;
pub mod public_key;
pub mod staking_account;
pub mod storage;
pub mod sync_msg;
pub mod tx;
pub mod utils;
pub mod validation;
pub mod validator;
pub mod wallet;
pub mod wallet_store_config;

use abci::types::EventAttribute;
pub use address::Address;
pub use capacity_merkle_root::CapacityMerkleRoot;
pub use capacity_seed::CapacitySeed;
pub use challenge_id::ChallengeId;
pub use content_id::ContentId;
pub use contract_id::ContractId;
pub use manifest_id::ManifestId;
pub use public_key::PublicKey;
use tendermint::Time; // Re-export Address

/// Seconds since UNIX epoch
pub type Timespec = u64;

pub fn to_timespec(time: Time) -> Timespec {
    time.duration_since(Time::unix_epoch())
        .expect("Failed to calculate duration since Unix epoch")
        .as_secs()
}

pub fn create_event_attribute(key: String, value: String) -> EventAttribute {
    let e = EventAttribute {
        key: key.as_bytes().to_vec(),
        value: value.as_bytes().to_vec(),
        index: true,
    };
    e
}
