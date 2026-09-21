//! Read-only RPC / HTTP query helpers against the configured node URL.

use super::{AbciHttpApi, AbciInfoWrapper};
use crate::config::client_config::ClientConfig;
use crate::json_bytes::json_number_array_as_bytes;
use eld_common::account::Account;
use eld_common::address::Address;
use eld_common::cado::CadoType;
use eld_common::cado::{CadoPath, CadoPathKey};
use eld_common::error::EldError;
use eld_common::nonce::Nonce;
use eld_common::staking_account::StakingAccount;
use tendermint_rpc::endpoint::block::Response;

pub(crate) fn abci_http_api(config: &ClientConfig) -> Result<AbciHttpApi, EldError> {
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
    config: &ClientConfig,
    address: String,
) -> Result<Option<Account>, EldError> {
    let api = abci_http_api(config)?;
    api.get_account_by_address(&address).await
}

pub async fn get_block(config: &ClientConfig, height: u64) -> Result<Response, EldError> {
    let api = abci_http_api(config)?;
    api.get_block(height).await
}

pub async fn get_abci_info(config: &ClientConfig) -> Result<AbciInfoWrapper, EldError> {
    let api = abci_http_api(config)?;
    api.get_latest_abci_info().await
}

pub async fn get_staking_account(
    config: &ClientConfig,
    address: &str,
) -> Result<Option<StakingAccount>, EldError> {
    let api = abci_http_api(config)?;
    api.get_staking_account(address).await
}

pub async fn get_next_nonce_for_account(
    config: &ClientConfig,
    address: String,
) -> Result<Option<Nonce>, EldError> {
    match get_account_by_address(config, address).await? {
        Some(account) => Ok(Some(next_nonce_from_account(&account)?)),
        None => Ok(None),
    }
}

pub async fn is_capacity_provider_registered(
    config: &ClientConfig,
    provider_address: &str,
) -> Result<bool, EldError> {
    let api = abci_http_api(config)?;
    api.is_capacity_provider_registered(provider_address).await
}

pub async fn get_next_nonce_for_account_cado(
    config: &ClientConfig,
    address: String,
) -> Result<Option<Nonce>, EldError> {
    let addr = Address::parse_hex_str(&address)?;
    let path = CadoPath::new(CadoType::Account, CadoPathKey::Address(addr))?
        .as_str()
        .to_string();
    let api = abci_http_api(config)?;
    let response = api.get_cado(path).await?;

    let Some(cado) = response.get("Mutable") else {
        return Ok(None);
    };
    let Some(data) = cado.get("data") else {
        return Ok(None);
    };
    let Some(data_array) = data.as_array() else {
        return Ok(None);
    };

    let data_bytes = json_number_array_as_bytes(data_array, "account_cado_data")?;

    let account = Account::deserialize_bin(&data_bytes).map_err(|e| EldError::ValidationError {
        field: "account_data".to_string(),
        value: format!("{data_bytes:?}"),
        details: format!("Failed to deserialize account: {e}"),
    })?;
    Ok(Some(next_nonce_from_account(&account)?))
}

pub async fn get_account_from_cado(
    config: &ClientConfig,
    path: String,
) -> Result<Account, EldError> {
    let api = abci_http_api(config)?;
    let response = api.get_cado(path.clone()).await?;

    if let Some(cado) = response.get("Mutable") {
        if let Some(data) = cado.get("data") {
            if let Some(data_array) = data.as_array() {
                let data_bytes = json_number_array_as_bytes(data_array, "account_cado_data")?;

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
