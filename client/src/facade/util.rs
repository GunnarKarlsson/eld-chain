//! Shared helpers for flow implementations.

use super::ChainClient;
use eld_common::error::{EldError, ErrorBuilder};
use eld_common::nonce::Nonce;
use eld_common::wallet::Wallet;

pub(crate) async fn require_wallet(
    client: &ChainClient,
    wallet_name: &str,
) -> Result<Wallet, EldError> {
    client
        .get_wallet_by_name(wallet_name.to_string())
        .await?
        .ok_or_else(|| ErrorBuilder::wallet_error("retrieval", wallet_name, "Wallet not found"))
}

pub(crate) fn require_nonce(nonce: Result<Option<Nonce>, EldError>) -> Result<Nonce, EldError> {
    nonce?.ok_or_else(|| {
        ErrorBuilder::network_error("nonce retrieval", "Failed to get account nonce")
    })
}
