//! Local `wallets.json` read/write helpers.

use crate::address::Address;
use crate::client_config::WALLETS_PATH;
use crate::error::EldError;
use crate::wallet::Wallet;
use crate::wallet_store_config::WalletStoreConfig;
use ed25519_dalek::SigningKey;
use rand::RngCore;
use serde_json;
use std::fs;
use std::path::Path;
use tracing::{error, info, warn};

/// Writes wallet JSON and restricts the file to owner read/write only (Unix).
fn write_wallet_file(path: impl AsRef<Path>, contents: &str) -> std::io::Result<()> {
    let path = path.as_ref();
    fs::write(path, contents)?;
    restrict_wallet_file_permissions(path)
}

#[cfg(unix)]
fn restrict_wallet_file_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_wallet_file_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

pub async fn create_wallet(name: String) -> Result<(), EldError> {
    let wallet_store_config = WalletStoreConfig::at_path(WALLETS_PATH);
    create_wallet_with_store_config(name, &wallet_store_config).await
}

pub async fn create_wallet_with_store_config(
    name: String,
    wallet_store_config: &WalletStoreConfig,
) -> Result<(), EldError> {
    let mut rng = rand::rng();
    let mut secret_bytes = [0u8; 32];
    rng.fill_bytes(&mut secret_bytes);
    let signing_key = SigningKey::from_bytes(&secret_bytes);
    let wallet = Wallet::from_signing_key(name, signing_key);

    let mut wallets = wallet_store_config.load_json_wallets()?;
    wallets.push(wallet.to_json());
    let json_str = serde_json::to_string_pretty(&wallets).map_err(|e| EldError::WalletError {
        operation: "serialize_wallet_file".to_string(),
        wallet_name: wallet_store_config.path().display().to_string(),
        details: format!("Failed to serialize wallet file: {e}"),
    })?;

    write_wallet_file(wallet_store_config.path(), &json_str).map_err(|e| {
        EldError::WalletError {
            operation: "write_wallet_file".to_string(),
            wallet_name: wallet_store_config.path().display().to_string(),
            details: format!("Failed to write wallet file: {e}"),
        }
    })?;

    info!(wallet_name = %wallet.name, "Created wallet");
    info!("{}", wallet.terminal_display());
    Ok(())
}

pub async fn get_wallets() -> Result<Vec<Wallet>, EldError> {
    WalletStoreConfig::load_wallets_from_path(WALLETS_PATH)
}

pub async fn get_wallets_with_store_config(
    wallet_store_config: &WalletStoreConfig,
) -> Result<Vec<Wallet>, EldError> {
    wallet_store_config.load_wallets()
}

pub async fn remove_wallet(name: String) -> Result<(), EldError> {
    let wallet_store_config = WalletStoreConfig::at_path(WALLETS_PATH);
    remove_wallet_with_store_config(name, &wallet_store_config).await
}

pub async fn remove_wallet_with_store_config(
    name: String,
    wallet_store_config: &WalletStoreConfig,
) -> Result<(), EldError> {
    let mut wallets = wallet_store_config.load_json_wallets()?;

    let initial_len = wallets.len();
    wallets.retain(|w| w.name != name);

    if wallets.len() == initial_len {
        warn!(wallet_name = %name, "Wallet not found for removal");
        warn!("Wallet with name '{name}' not found");
        return Ok(());
    }

    let json_str = serde_json::to_string_pretty(&wallets).map_err(|e| EldError::WalletError {
        operation: "serialize_wallet_file".to_string(),
        wallet_name: wallet_store_config.path().display().to_string(),
        details: format!("Failed to serialize wallet file: {e}"),
    })?;

    write_wallet_file(wallet_store_config.path(), &json_str).map_err(|e| EldError::WalletError {
        operation: "write_wallet_file".to_string(),
        wallet_name: wallet_store_config.path().display().to_string(),
        details: format!("Failed to write wallet file: {e}"),
    })
}

pub async fn get_wallet_by_name(name: String) -> Option<Wallet> {
    get_wallet_by_name_at_path(name, WALLETS_PATH).await
}

pub async fn get_wallet_by_name_at_path(
    name: String,
    wallet_path: impl AsRef<Path>,
) -> Option<Wallet> {
    match WalletStoreConfig::load_wallets_from_path(wallet_path) {
        Ok(wallets) => {
            let wallet = wallets.into_iter().find(|w| w.name == name);
            if wallet.is_none() {
                error!(
                    error = %crate::error::ErrorBuilder::not_found_error("Wallet", &name)
                );
            }
            wallet
        }
        Err(e) => {
            error!(%e);
            None
        }
    }
}

pub async fn get_wallet_by_name_with_store_config(
    name: String,
    wallet_store_config: &WalletStoreConfig,
) -> Result<Option<Wallet>, EldError> {
    let wallets = get_wallets_with_store_config(wallet_store_config).await?;
    let wallet = wallets.into_iter().find(|w| w.name == name);
    if wallet.is_none() {
        error!(
            error = %crate::error::ErrorBuilder::not_found_error("Wallet", &name)
        );
    }
    Ok(wallet)
}

pub async fn display_wallet_by_name_with_store_config(
    name: String,
    wallet_store_config: &WalletStoreConfig,
) -> Result<(), EldError> {
    if let Some(wallet) =
        get_wallet_by_name_with_store_config(name.clone(), wallet_store_config).await?
    {
        info!(wallet_name = %name, "Displaying wallet");
        info!("{}", wallet.terminal_display());
    } else {
        warn!(wallet_name = %name, "Couldn't find wallet for display");
        warn!("Couldn't find wallet");
    }
    Ok(())
}

pub async fn get_wallet_by_address(address: &str) -> Option<Wallet> {
    match WalletStoreConfig::load_wallets_from_path(WALLETS_PATH) {
        Ok(wallets) => {
            let target_address = match Address::parse_hex_str(address) {
                Ok(addr) => addr,
                Err(_) => {
                    error!(address = %address, "Invalid address format");
                    return None;
                }
            };
            let wallet = wallets.into_iter().find(|w| w.address == target_address);
            if wallet.is_none() {
                error!(address = %address, "Wallet not found for address");
            }
            wallet
        }
        Err(e) => {
            error!(%e);
            None
        }
    }
}

pub async fn list_wallets() {
    info!("Listing wallets");
    info!("Wallets:\n");
    match get_wallets().await {
        Ok(wallets) => {
            if wallets.is_empty() {
                info!("No wallets found");
                info!("No wallets found. Create one with 'create-wallet <name>'");
            } else {
                info!(wallet_count = wallets.len(), "Retrieved wallets");
                for wallet in wallets {
                    info!("{}", wallet.terminal_display());
                }
            }
        }
        Err(e) => error!(%e),
    }
}

pub async fn list_wallets_with_store_config(
    wallet_store_config: &WalletStoreConfig,
) -> Result<(), EldError> {
    info!("Listing wallets");
    info!("Wallets:\n");
    let wallets = get_wallets_with_store_config(wallet_store_config).await?;
    if wallets.is_empty() {
        info!("No wallets found");
        info!("No wallets found. Create one with 'create-wallet <name>'");
    } else {
        info!(wallet_count = wallets.len(), "Retrieved wallets");
        for wallet in wallets {
            info!("{}", wallet.terminal_display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(unix)]
    #[test]
    fn write_wallet_file_sets_owner_only_permissions() {
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let path = temp_file.path();

        write_wallet_file(path, "[]").unwrap();

        let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
