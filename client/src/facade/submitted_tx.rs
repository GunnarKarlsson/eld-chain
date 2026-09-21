//! Result of signing and broadcasting a transaction via `broadcast_tx_commit`.

use eld_common::coin::Coin;
use eld_common::nonce::Nonce;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct SubmittedTx {
    pub response: Value,
    pub signed_tx_json: String,
    pub fee: Coin,
    pub nonce: Nonce,
}
