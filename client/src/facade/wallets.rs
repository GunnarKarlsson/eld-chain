//! Local `wallets.json` read/write helpers.

use crate::wallet_store_config::WalletStoreConfig;
use ed25519_dalek::SigningKey;
use eld_common::error::EldError;
use eld_common::wallet::Wallet;
use rand::RngCore;
use serde_json;
use std::fs;
use std::path::Path;

/// Writes wallet JSON; on Unix also chmods the file to owner read/write only (`0600`).
fn write_wallet_file(path: impl AsRef<Path>, contents: &str) -> std::io::Result<()> {
    let path = path.as_ref();
    fs::write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn not_found_to_none(result: Result<Wallet, EldError>) -> Result<Option<Wallet>, EldError> {
    match result {
        Ok(wallet) => Ok(Some(wallet)),
        Err(EldError::NotFoundError { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

pub(crate) async fn create_wallet_with_store_config(
    name: String,
    wallet_store_config: &WalletStoreConfig,
) -> Result<Wallet, EldError> {
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

    Ok(wallet)
}

pub(crate) async fn get_wallets_with_store_config(
    wallet_store_config: &WalletStoreConfig,
) -> Result<Vec<Wallet>, EldError> {
    wallet_store_config.load_wallets()
}

pub(crate) async fn remove_wallet_with_store_config(
    name: String,
    wallet_store_config: &WalletStoreConfig,
) -> Result<bool, EldError> {
    let mut wallets = wallet_store_config.load_json_wallets()?;

    let initial_len = wallets.len();
    wallets.retain(|w| w.name != name);

    if wallets.len() == initial_len {
        return Ok(false);
    }

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
    Ok(true)
}

pub(crate) async fn get_wallet_by_name_with_store_config(
    name: String,
    wallet_store_config: &WalletStoreConfig,
) -> Result<Option<Wallet>, EldError> {
    not_found_to_none(wallet_store_config.wallet_by_name(&name))
}

pub(crate) async fn get_wallet_by_address_with_store_config(
    address: &str,
    wallet_store_config: &WalletStoreConfig,
) -> Result<Option<Wallet>, EldError> {
    not_found_to_none(wallet_store_config.wallet_by_address(address))
}

pub(crate) async fn list_wallets_with_store_config(
    wallet_store_config: &WalletStoreConfig,
) -> Result<Vec<Wallet>, EldError> {
    get_wallets_with_store_config(wallet_store_config).await
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
