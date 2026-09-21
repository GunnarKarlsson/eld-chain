//! Result of signing and broadcasting a transaction via `broadcast_tx_commit`.

use crate::api::abci::broadcast_tx_hash;
use eld_common::coin::Coin;
use eld_common::error::EldError;
use eld_common::nonce::Nonce;
use serde_json::Value;
use tendermint::Hash;

/// Tendermint transaction hash (SHA-256 of the wire bytes committed in a block).
pub type TxHash = Hash;

/// Signed transaction broadcast and committed via `broadcast_tx_commit`.
#[derive(Debug, Clone)]
pub struct SubmittedTx {
    pub tx_hash: TxHash,
    pub response: Value,
    pub signed_tx_json: String,
    pub fee: Coin,
    pub nonce: Nonce,
}

impl SubmittedTx {
    pub(crate) fn from_broadcast(
        tx_wire_hex: &str,
        signed_tx_json: String,
        fee: Coin,
        nonce: Nonce,
        response: Value,
    ) -> Result<Self, EldError> {
        Ok(Self {
            tx_hash: broadcast_tx_hash(&response, tx_wire_hex)?,
            response,
            signed_tx_json,
            fee,
            nonce,
        })
    }
}
