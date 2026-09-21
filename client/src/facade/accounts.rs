//! Account, staking, and wallet lookup flows.

use super::ChainClient;
use crate::api::abci::AbciInfoWrapper;
use eld_common::account::Account;
use eld_common::error::EldError;
use eld_common::staking_account::StakingAccount;

pub(crate) async fn get_abci_info(client: &ChainClient) -> Result<AbciInfoWrapper, EldError> {
    crate::api::abci::query::get_abci_info(&client.config).await
}

pub(crate) async fn get_account(
    client: &ChainClient,
    address: String,
) -> Result<Option<Account>, EldError> {
    client.get_account_by_address(address).await
}

pub(crate) async fn get_staking_account(
    client: &ChainClient,
    address: String,
) -> Result<Option<StakingAccount>, EldError> {
    crate::api::abci::query::get_staking_account(&client.config, &address).await
}

pub(crate) async fn get_provider_id_for_capacity(
    client: &ChainClient,
    wallet_name: &str,
) -> Result<String, EldError> {
    let wallet = client
        .get_wallet_by_name(wallet_name.to_string())
        .await?
        .ok_or_else(|| EldError::StorageError {
            operation: "get_wallet_for_capacity".to_string(),
            details: format!(
                "Failed to get wallet '{wallet_name}' for capacity registration. The node requires this wallet to initialize capacity management."
            ),
        })?;

    Ok(wallet.address.hex_with_prefix())
}
