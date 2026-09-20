#![doc = include_str!("../README.md")]
#![warn(unreachable_pub)]

pub mod account;
pub mod address;
pub mod cado;
pub mod capacity;
pub mod capacity_challenge;
pub mod capacity_merkle_root;
pub mod capacity_proof;
pub mod capacity_seed;
pub mod challenge_id;
pub mod coin;
pub mod constants;
pub mod content_id;
pub mod error;
pub mod fee;
mod hex_encoding;
pub mod logging;
pub mod manifest_id;
pub mod missing_content;
pub mod namespace;
pub mod nonce;
pub mod pinboard;
pub mod public_key;
pub mod staking_account;
pub mod storage;
pub mod sync_msg;
pub mod tx;
pub mod utils;
pub mod validation;
pub mod validator;
pub mod wallet;

use abci::types::EventAttribute;
pub use address::Address;
pub use capacity_merkle_root::CapacityMerkleRoot;
pub use capacity_seed::CapacitySeed;
pub use challenge_id::ChallengeId;
pub use content_id::ContentId;
pub use manifest_id::ManifestId;
pub use public_key::PublicKey;
use tendermint::Time; // Re-export Address

/// Seconds since UNIX epoch
pub type Timespec = u64;

pub fn to_timespec(time: Time) -> Result<Timespec, error::EldError> {
    time.duration_since(Time::unix_epoch())
        .map(|duration| duration.as_secs())
        .map_err(|e| error::EldError::ValidationError {
            field: "time".to_string(),
            value: time.to_rfc3339(),
            details: format!("Failed to calculate duration since Unix epoch: {e}"),
        })
}

pub fn create_event_attribute(key: String, value: String) -> EventAttribute {
    let e = EventAttribute {
        key: key.as_bytes().to_vec(),
        value: value.as_bytes().to_vec(),
        index: true,
    };
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_timespec_unix_epoch_is_zero() {
        assert_eq!(to_timespec(Time::unix_epoch()).unwrap(), 0);
    }
}
