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

pub(crate) fn abci_http_api(config: &CliConfig) -> Result<AbciHttpApi, EldError> {
    AbciHttpApi::new(config.get_node_url()?)
}

fn next_nonce_from_account(account: &Account) -> Result<Nonce, EldError> {
    account
        .nonce()
        .next()
        .ok_or_else(|| EldError::ValidationError {
            field: "nonce".to_string(),
            value: account.nonce().to_string(),
            details: "Account nonce overflowed (u32::MAX); wrap-around is not allowed".to_string(),
        })
}

pub async fn get_account_by_address(
    config: &CliConfig,
    address: String,
) -> Result<Option<Account>, EldError> {
    let api = abci_http_api(config)?;
    api.get_account_by_address(&address).await
}

pub async fn get_block(config: &CliConfig, height: u64) -> Result<Response, EldError> {
    let api = abci_http_api(config)?;
    api.get_block(height).await
}

pub async fn get_abci_info(config: &CliConfig) -> Result<AbciInfoWrapper, EldError> {
    let api = abci_http_api(config)?;
    api.get_latest_abci_info().await
}

pub async fn get_staking_account(
    config: &CliConfig,
    address: &str,
) -> Result<Option<StakingAccount>, EldError> {
    let api = abci_http_api(config)?;
    api.get_staking_account(address).await
}

pub async fn get_next_nonce_for_account(
    config: &CliConfig,
    address: String,
) -> Result<Option<Nonce>, EldError> {
    match get_account_by_address(config, address).await? {
        Some(account) => Ok(Some(next_nonce_from_account(&account)?)),
        None => Ok(None),
    }
}

pub async fn is_capacity_provider_registered(
    config: &CliConfig,
    provider_address: &str,
) -> Result<bool, EldError> {
    let api = abci_http_api(config)?;
    api.is_capacity_provider_registered(provider_address).await
}

pub async fn get_next_nonce_for_account_cado(
    config: &CliConfig,
    address: String,
) -> Result<Option<Nonce>, EldError> {
    let addr = Address::parse_hex_str(&address)?;
    let path = CadoPath::new(CadoType::Account, CadoPathKey::Address(addr))?
        .as_str()
        .to_string();
    let api = abci_http_api(config)?;
    debug!(
        path = %crate::logging::SanitizedLog::as_path(&path),
        "Getting account CADO"
    );
    let response = api.get_cado(path).await.map_err(|e| {
        tracing::error!(
            address = %crate::logging::SanitizedLog::as_address(&address),
            error = %e,
            "Error getting account CADO"
        );
        e
    })?;

    debug!(
        response = %crate::logging::SanitizedLog::new(format!("{response:?}")),
        "Received CADO response"
    );

    let Some(cado) = response.get("Mutable") else {
        return Ok(None);
    };
    let Some(data) = cado.get("data") else {
        return Ok(None);
    };
    let Some(data_array) = data.as_array() else {
        return Ok(None);
    };

    let data_bytes: Vec<u8> = data_array
        .iter()
        .map(|v| v.as_u64().unwrap_or(0) as u8)
        .collect();

    let account = Account::deserialize_bin(&data_bytes).map_err(|e| EldError::ValidationError {
        field: "account_data".to_string(),
        value: format!("{data_bytes:?}"),
        details: format!("Failed to deserialize account: {e}"),
    })?;
    Ok(Some(next_nonce_from_account(&account)?))
}

pub async fn get_account_from_cado(config: &CliConfig, path: String) -> Result<Account, EldError> {
    let api = abci_http_api(config)?;
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
