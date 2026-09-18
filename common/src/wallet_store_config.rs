//! Wallet store path configuration and validation.

use crate::address::Address;
use crate::error::EldError;
use crate::wallet::{JsonWallet, Wallet};
use std::fs;
use std::path::{Path, PathBuf};

/// Configuration for a local JSON wallet store.
pub struct WalletStoreConfig {
    path: PathBuf,
    wallets: Vec<Wallet>,
}

impl WalletStoreConfig {
    /// Creates a wallet store config by loading and validating the wallet file at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, EldError> {
        let path = path.into();
        if !path.exists() {
            return Err(EldError::WalletError {
                operation: "load_wallet_file".to_string(),
                wallet_name: path.display().to_string(),
                details: "Wallet file does not exist".to_string(),
            });
        }
        let wallets = Self::load_wallets_from_path(&path)?;
        Ok(Self { path, wallets })
    }

    /// Path-only store for read/write helpers (does not require the file to exist).
    pub fn at_path(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            wallets: Vec::new(),
        }
    }

    /// Loads and validates all wallets from `path`.
    pub fn load_wallets_from_path(path: impl AsRef<Path>) -> Result<Vec<Wallet>, EldError> {
        let path = path.as_ref().to_path_buf();
        Self {
            path: path.clone(),
            wallets: Vec::new(),
        }
        .load_wallets()
    }

    /// Returns a wallet by name from the loaded store.
    pub fn wallet_by_name(&self, name: &str) -> Result<Wallet, EldError> {
        self.wallets
            .iter()
            .find(|w| w.name == name)
            .cloned()
            .ok_or_else(|| EldError::NotFoundError {
                resource_type: "Wallet".to_string(),
                identifier: name.to_string(),
            })
    }

    /// Returns a wallet by hex address from the loaded store.
    pub fn wallet_by_address(&self, address: &str) -> Result<Wallet, EldError> {
        let target_address =
            Address::parse_hex_str(address).map_err(|e| EldError::WalletError {
                operation: "parse_wallet_address".to_string(),
                wallet_name: address.to_string(),
                details: format!("Invalid address format: {e}"),
            })?;
        self.wallets
            .iter()
            .find(|w| w.address == target_address)
            .cloned()
            .ok_or_else(|| EldError::NotFoundError {
                resource_type: "Wallet".to_string(),
                identifier: address.to_string(),
            })
    }

    /// Returns the configured wallet file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn load_json_wallets(&self) -> Result<Vec<JsonWallet>, EldError> {
        let json = match fs::read_to_string(&self.path) {
            Ok(json) => json,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(e) => {
                return Err(EldError::WalletError {
                    operation: "read_wallet_file".to_string(),
                    wallet_name: self.path.display().to_string(),
                    details: format!("Failed to read wallet file: {e}"),
                });
            }
        };

        serde_json::from_str(&json).map_err(|e| {
            EldError::make_validation_error(
                "wallet_json",
                &self.path.display().to_string(),
                format!("Failed to parse wallet JSON: {e}"),
            )
        })
    }

    pub(crate) fn load_wallets(&self) -> Result<Vec<Wallet>, EldError> {
        self.load_json_wallets()?
            .into_iter()
            .map(Wallet::from_json_wallet)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use std::fs;
    use tempfile::NamedTempFile;

    /// Documented throwaway seed (all 0x01). Not a live-network key.
    fn fixture_wallet() -> Wallet {
        Wallet::from_signing_key(
            "test-wallet-1".to_string(),
            SigningKey::from_bytes(&[1u8; 32]),
        )
    }

    fn valid_wallet_json() -> String {
        serde_json::to_string_pretty(&[fixture_wallet().to_json()]).unwrap()
    }

    #[test]
    fn new_loads_parseable_wallet_file() {
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(temp_file.path(), valid_wallet_json()).unwrap();

        let config = WalletStoreConfig::new(temp_file.path()).unwrap();

        assert_eq!(config.path(), temp_file.path());
    }

    #[test]
    fn load_wallets_from_path_returns_wallets() {
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(temp_file.path(), valid_wallet_json()).unwrap();

        let wallets = WalletStoreConfig::load_wallets_from_path(temp_file.path()).unwrap();

        assert_eq!(wallets.len(), 1);
        assert_eq!(wallets[0].name, "test-wallet-1");
    }

    #[test]
    fn wallet_by_name_returns_loaded_wallet() {
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(temp_file.path(), valid_wallet_json()).unwrap();

        let config = WalletStoreConfig::new(temp_file.path()).unwrap();
        let wallet = config.wallet_by_name("test-wallet-1").unwrap();

        assert_eq!(wallet.name, "test-wallet-1");
    }

    #[test]
    fn wallet_by_address_returns_loaded_wallet() {
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(temp_file.path(), valid_wallet_json()).unwrap();

        let config = WalletStoreConfig::new(temp_file.path()).unwrap();
        let address = fixture_wallet().address.hex();
        let wallet = config.wallet_by_address(&address).unwrap();

        assert_eq!(wallet.name, "test-wallet-1");
    }

    #[test]
    fn load_wallets_from_path_returns_empty_when_file_missing() {
        let temp_file = NamedTempFile::new().unwrap();
        let missing_path = temp_file.path().with_file_name("missing-wallets.json");

        let wallets = WalletStoreConfig::load_wallets_from_path(missing_path).unwrap();

        assert!(wallets.is_empty());
    }

    #[test]
    fn new_errors_for_missing_wallet_file() {
        let temp_file = NamedTempFile::new().unwrap();
        let missing_path = temp_file.path().with_file_name("missing-wallets.json");

        let result = WalletStoreConfig::new(missing_path);

        assert!(matches!(result, Err(EldError::WalletError { .. })));
    }

    #[test]
    fn new_errors_for_invalid_wallet_json() {
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(temp_file.path(), "{}").unwrap();

        let result = WalletStoreConfig::new(temp_file.path());

        assert!(matches!(result, Err(EldError::ValidationError { .. })));
    }
}
