//! Read-only RPC / HTTP query helpers against the configured node URL.

use crate::abci_api::AbciHttpApi;
use crate::abci_api::AbciInfoWrapper;
use crate::account::Account;
use crate::address::Address;
use crate::cado::CadoType;
use crate::cado::{CadoPath, CadoPathKey};
use crate::client_config::CliConfig;
use crate::error::EldError;
use crate::nonce::Nonce;
use crate::staking_account::StakingAccount;
use tendermint_rpc::endpoint::block::Response;
use tracing::debug;

pub async fn get_account_by_address(
    config: &CliConfig,
    address: String,
) -> Result<Option<Account>, EldError> {
    let api = AbciHttpApi::new(config.get_node_url().to_owned());
    api.get_account_by_address(&address).await
}

pub async fn get_block(config: &CliConfig, height: u64) -> Response {
    let api = AbciHttpApi::new(config.get_node_url().to_owned());
    api.get_block(height)
        .await
        .expect("Failed to get block from API")
}

pub async fn get_abci_info(config: &CliConfig) -> AbciInfoWrapper {
    let api = AbciHttpApi::new(config.get_node_url().to_owned());
    api.get_latest_abci_info()
        .await
        .expect("Failed to get latest ABCI info")
}

pub async fn get_staking_account(
    config: &CliConfig,
    address: &str,
) -> Result<Option<StakingAccount>, EldError> {
    let api = AbciHttpApi::new(config.get_node_url().to_owned());
    api.get_staking_account(address).await
}

pub async fn get_next_nonce_for_account(config: &CliConfig, address: String) -> Option<Nonce> {
    match get_account_by_address(config, address.clone()).await {
        Ok(Some(account)) => Some(account.nonce().next()),
        Ok(None) | Err(_) => None,
    }
}

pub async fn is_capacity_provider_registered(
    config: &CliConfig,
    provider_address: &str,
) -> Result<bool, EldError> {
    let api = AbciHttpApi::new(config.get_node_url().to_owned());
    api.is_capacity_provider_registered(provider_address).await
}

pub async fn get_next_nonce_for_account_cado(config: &CliConfig, address: String) -> Option<Nonce> {
    let addr = Address::parse_hex_str(&address).ok()?;
    let path = CadoPath::new(CadoType::Account, CadoPathKey::Address(addr))
        .ok()?
        .as_str()
        .to_string();
    let api = AbciHttpApi::new(config.get_node_url().to_owned());
    debug!(
        path = %crate::logging::SanitizedLog::as_path(&path),
        "Getting account CADO"
    );
    match api.get_cado(path).await {
        Ok(response) => {
            debug!(
                response = %crate::logging::SanitizedLog::new(format!("{response:?}")),
                "Received CADO response"
            );
            if let Some(cado) = response.get("Mutable") {
                if let Some(data) = cado.get("data") {
                    if let Some(data_array) = data.as_array() {
                        let data_bytes: Vec<u8> = data_array
                            .iter()
                            .map(|v| v.as_u64().unwrap_or(0) as u8)
                            .collect();

                        if let Ok(account) = Account::deserialize_bin(&data_bytes) {
                            return Some(account.nonce().next());
                        }
                    }
                }
            }
            None
        }
        Err(e) => {
            tracing::error!(
                address = %crate::logging::SanitizedLog::as_address(&address),
                error = %e,
                "Error getting account CADO"
            );
            None
        }
    }
}

pub async fn get_account_from_cado(config: &CliConfig, path: String) -> Result<Account, EldError> {
    let api = AbciHttpApi::new(config.get_node_url().to_owned());
    let response = api.get_cado(path.clone()).await?;

    if let Some(cado) = response.get("Mutable") {
        if let Some(data) = cado.get("data") {
            if let Some(data_array) = data.as_array() {
                let data_bytes: Vec<u8> = data_array
                    .iter()
                    .map(|v| v.as_u64().unwrap_or(0) as u8)
                    .collect();

                return Account::deserialize_bin(&data_bytes).map_err(|e| {
                    EldError::ValidationError {
                        field: "account_data".to_string(),
                        value: format!("{data_bytes:?}"),
                        details: format!("Failed to deserialize account: {e}"),
                    }
                });
            }
        }
    }

    Err(EldError::NotFoundError {
        resource_type: "Account".to_string(),
        identifier: path,
    })
}
