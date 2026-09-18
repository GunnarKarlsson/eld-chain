//! Account, staking, and wallet display flows.

use crate::abci_api::AbciInfoWrapper;
use crate::client::ChainClient;
use crate::error::{EldError, ErrorBuilder};
use crate::logging::{SanitizedLog, SanitizedLoggable};
use tracing::{error, info, warn};

pub(crate) async fn display_account(client: &ChainClient, address: String) -> Result<(), EldError> {
    match client.get_account_by_address(address.clone()).await? {
        Some(account) => {
            info!("Account found:");
            info!("{}", account.sanitized_log());
            Ok(())
        }
        None => {
            warn!(
                "No account found for address: {}",
                SanitizedLog::as_address(&address)
            );
            Ok(())
        }
    }
}

pub(crate) async fn get_abci_info(client: &ChainClient) -> AbciInfoWrapper {
    let abci_info = crate::client::query::get_abci_info(&client.config).await;
    info!("{}", abci_info);
    abci_info
}

pub(crate) async fn get_account(client: &ChainClient, address: String) {
    match client.display_account(address.clone()).await {
        Ok(_) => (),
        Err(e) => error!(%e),
    }
}

pub(crate) async fn get_staking_account(client: &ChainClient, address: String) {
    match crate::client::query::get_staking_account(&client.config, &address).await {
        Ok(staking_account) => {
            if let Some(account) = staking_account {
                info!(address = %address, "Retrieved staking account");
                info!("staking_account: {}", account.sanitized_log());
            } else {
                warn!(address = %address, "Staking account not found");
                error!(error = %(ErrorBuilder::not_found_error("Staking Account", &address)));
            }
        }
        Err(e) => error!(%e),
    }
}

pub(crate) async fn get_provider_id_for_capacity(
    client: &ChainClient,
    wallet_name: &str,
) -> Result<String, EldError> {
    let wallet = client
        .get_wallet_by_name(wallet_name.to_string())
        .await
        .ok_or_else(|| EldError::StorageError {
            operation: "get_wallet_for_capacity".to_string(),
            details: format!(
                "Failed to get wallet '{wallet_name}' for capacity registration. The node requires this wallet to initialize capacity management."
            ),
        })?;

    Ok(wallet.address.hex_with_prefix())
}

pub(crate) async fn display_wallet_by_name(client: &ChainClient, name: String) {
    if let Some(wallet) = client.get_wallet_by_name(name.clone()).await {
        info!(wallet_name = %name, "Displaying wallet");
        info!("{}", wallet.terminal_display());
    } else {
        warn!(wallet_name = %name, "Couldn't find wallet for display");
        warn!("Couldn't find wallet");
    }
}
