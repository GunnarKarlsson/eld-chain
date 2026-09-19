//! HTTP/RPC and wallet orchestration client used by the CLI and node tooling.

use crate::abci_api::AbciInfoWrapper;
use crate::account::Account;
use crate::client_config::{CliConfig, ConsensusConfig, FeeConfig, CONSENSUS_CONFIG_PATH};
use crate::error::EldError;
use crate::nonce::Nonce;
use crate::wallet::Wallet;
use crate::wallet_store_config::WalletStoreConfig;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use tendermint_rpc::endpoint::block::Response;
use tracing::{error, info, warn};

#[derive(Clone)]
pub struct ChainClient {
    pub(crate) config: CliConfig,
    pub(crate) fee_config: FeeConfig,
    wallet_store: Option<Arc<WalletStoreConfig>>,
}

impl ChainClient {
    pub fn new(config: CliConfig) -> Result<Self, EldError> {
        let consensus_config = ConsensusConfig::from_file(CONSENSUS_CONFIG_PATH)?;
        let fee_config = consensus_config.fee_config;

        Ok(Self {
            config,
            fee_config,
            wallet_store: None,
        })
    }

    /// Creates a client with wallets loaded from `wallet_path` via [`WalletStoreConfig`].
    pub fn new_with_wallets(
        config: CliConfig,
        wallet_path: impl AsRef<Path>,
    ) -> Result<Self, EldError> {
        let consensus_config = ConsensusConfig::from_file(CONSENSUS_CONFIG_PATH)?;
        let fee_config = consensus_config.fee_config;

        Ok(Self {
            config,
            fee_config,
            wallet_store: Some(Arc::new(WalletStoreConfig::new(
                wallet_path.as_ref().to_path_buf(),
            )?)),
        })
    }

    fn wallet_store_config_from_path(wallet_path: impl AsRef<Path>) -> WalletStoreConfig {
        WalletStoreConfig::at_path(wallet_path.as_ref())
    }

    pub async fn request_faucet(&self, address: String) -> Result<(), EldError> {
        crate::client::faucet::request_faucet(&self.config, address).await
    }

    pub fn get_chain_id(&self) -> &str {
        &self.config.chain_id
    }

    pub fn get_fee_config(&self) -> &FeeConfig {
        &self.fee_config
    }

    pub async fn get_account_by_address(
        &self,
        address: String,
    ) -> Result<Option<Account>, EldError> {
        crate::client::query::get_account_by_address(&self.config, address).await
    }

    pub async fn display_account(&self, address: String) -> Result<(), EldError> {
        crate::client::flows::display_account(self, address).await
    }

    pub async fn send_tx_rpc(&self, hex_encoded: &str) -> Result<Value, EldError> {
        crate::client::tx_broadcast::send_tx_rpc(&self.config, hex_encoded).await
    }

    pub async fn get_block(&self, height: u64) -> Result<Response, EldError> {
        crate::client::query::get_block(&self.config, height).await
    }

    pub async fn get_abci_info(&self) -> Result<AbciInfoWrapper, EldError> {
        crate::client::flows::get_abci_info(self).await
    }

    pub async fn get_account(&self, address: String) -> Result<(), EldError> {
        crate::client::flows::get_account(self, address).await
    }

    pub async fn get_staking_account(&self, address: String) -> Result<(), EldError> {
        crate::client::flows::get_staking_account(self, address).await
    }

    pub async fn get_next_nonce_for_account(
        &self,
        address: String,
    ) -> Result<Option<Nonce>, EldError> {
        crate::client::query::get_next_nonce_for_account(&self.config, address).await
    }

    /// Check if a capacity provider is registered on-chain (in capacity_validators).
    pub async fn is_capacity_provider_registered(
        &self,
        provider_address: &str,
    ) -> Result<bool, EldError> {
        crate::client::query::is_capacity_provider_registered(&self.config, provider_address).await
    }

    pub async fn get_next_nonce_for_account_cado(
        &self,
        address: String,
    ) -> Result<Option<Nonce>, EldError> {
        crate::client::query::get_next_nonce_for_account_cado(&self.config, address).await
    }

    pub async fn create_wallet(&self, name: String) -> Result<(), EldError> {
        crate::client::wallets::create_wallet(name).await
    }

    pub async fn create_wallet_with_store_config(
        &self,
        name: String,
        wallet_store_config: &WalletStoreConfig,
    ) -> Result<(), EldError> {
        crate::client::wallets::create_wallet_with_store_config(name, wallet_store_config).await
    }

    pub async fn create_wallet_from_path(
        &self,
        name: String,
        wallet_path: impl AsRef<Path>,
    ) -> Result<(), EldError> {
        let wallet_store_config = Self::wallet_store_config_from_path(wallet_path);
        self.create_wallet_with_store_config(name, &wallet_store_config)
            .await
    }

    pub async fn list_wallets(&self) -> Result<(), EldError> {
        crate::client::wallets::list_wallets().await
    }

    pub async fn list_wallets_with_store_config(
        &self,
        wallet_store_config: &WalletStoreConfig,
    ) -> Result<(), EldError> {
        crate::client::wallets::list_wallets_with_store_config(wallet_store_config).await
    }

    pub async fn list_wallets_from_path(
        &self,
        wallet_path: impl AsRef<Path>,
    ) -> Result<(), EldError> {
        let wallets = self.get_wallets_from_path(wallet_path).await?;
        info!("Listing wallets");
        info!("Wallets:\n");
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

    pub async fn get_wallets(&self) -> Result<Vec<Wallet>, EldError> {
        crate::client::wallets::get_wallets().await
    }

    pub async fn get_wallets_from_path(
        &self,
        wallet_path: impl AsRef<Path>,
    ) -> Result<Vec<Wallet>, EldError> {
        WalletStoreConfig::load_wallets_from_path(wallet_path)
    }

    pub async fn remove_wallet(&self, name: String) -> Result<(), EldError> {
        crate::client::wallets::remove_wallet(name).await
    }

    pub async fn remove_wallet_with_store_config(
        &self,
        name: String,
        wallet_store_config: &WalletStoreConfig,
    ) -> Result<(), EldError> {
        crate::client::wallets::remove_wallet_with_store_config(name, wallet_store_config).await
    }

    pub async fn remove_wallet_from_path(
        &self,
        name: String,
        wallet_path: impl AsRef<Path>,
    ) -> Result<(), EldError> {
        let wallet_store_config = Self::wallet_store_config_from_path(wallet_path);
        self.remove_wallet_with_store_config(name, &wallet_store_config)
            .await
    }

    pub async fn get_wallet_by_name(&self, name: String) -> Option<Wallet> {
        if let Some(wallet_store) = &self.wallet_store {
            return match wallet_store.wallet_by_name(&name) {
                Ok(wallet) => Some(wallet),
                Err(e) => {
                    error!(%e);
                    None
                }
            };
        }
        crate::client::wallets::get_wallet_by_name(name).await
    }

    pub async fn display_wallet_by_name_with_store_config(
        &self,
        name: String,
        wallet_store_config: &WalletStoreConfig,
    ) -> Result<(), EldError> {
        crate::client::wallets::display_wallet_by_name_with_store_config(name, wallet_store_config)
            .await
    }

    pub async fn display_wallet_by_name_from_path(
        &self,
        name: String,
        wallet_path: impl AsRef<Path>,
    ) -> Result<(), EldError> {
        let wallets = self.get_wallets_from_path(wallet_path).await?;
        if let Some(wallet) = wallets.into_iter().find(|w| w.name == name) {
            info!(wallet_name = %name, "Displaying wallet");
            info!("{}", wallet.terminal_display());
        } else {
            warn!(wallet_name = %name, "Couldn't find wallet for display");
            warn!("Couldn't find wallet");
        }
        Ok(())
    }

    /// Returns the hex-encoded address (with 0x prefix) for the given wallet name.
    /// Used by node components that need a provider_id derived from a wallet.
    pub async fn get_provider_id_for_capacity(
        &self,
        wallet_name: &str,
    ) -> Result<String, EldError> {
        crate::client::flows::get_provider_id_for_capacity(self, wallet_name).await
    }

    pub async fn get_wallet_by_address(&self, address: &str) -> Option<Wallet> {
        if let Some(wallet_store) = &self.wallet_store {
            return match wallet_store.wallet_by_address(address) {
                Ok(wallet) => Some(wallet),
                Err(e) => {
                    error!(%e);
                    None
                }
            };
        }
        crate::client::wallets::get_wallet_by_address(address).await
    }

    pub async fn display_wallet_by_name(&self, name: String) -> Result<(), EldError> {
        crate::client::flows::display_wallet_by_name(self, name).await
    }

    pub async fn transfer(
        &self,
        wallet_name: String,
        recipient: String,
        amount: u128,
    ) -> Result<(), EldError> {
        crate::client::flows::transfer(self, wallet_name, recipient, amount).await
    }

    pub async fn list_all_transactions(&self) -> Result<(), EldError> {
        crate::client::flows::list_all_transactions(self).await
    }

    pub async fn list_transactions(&self, addr: String) -> Result<(), EldError> {
        crate::client::flows::list_transactions(self, addr).await
    }

    pub async fn stake(&self, wallet_name: String, amount: u128) -> Result<(), EldError> {
        crate::client::flows::stake(self, wallet_name, amount).await
    }

    pub async fn unstake(&self, wallet_name: String, amount: u128) -> Result<(), EldError> {
        crate::client::flows::unstake(self, wallet_name, amount).await
    }

    pub async fn view_active_validators(&self) -> Result<(), EldError> {
        crate::client::flows::view_active_validators(self).await
    }

    pub async fn view_epoch_info(&self) -> Result<(), EldError> {
        crate::client::flows::view_epoch_info(self).await
    }

    pub async fn view_epoch(&self) -> Result<(), EldError> {
        crate::client::flows::view_epoch(self).await
    }

    pub async fn add_namespace(
        &self,
        wallet_name: String,
        namespace_slug: String,
        registration_fee: u128,
    ) -> Result<(), EldError> {
        crate::client::flows::add_namespace(self, wallet_name, namespace_slug, registration_fee)
            .await
    }

    pub async fn get_namespace(&self, namespace_slug: String) -> Result<(), EldError> {
        crate::client::flows::get_namespace(self, namespace_slug).await
    }

    pub async fn post_pinboard_message(
        &self,
        input: crate::pinboard_api::PinboardMessageParams,
    ) -> Result<(), EldError> {
        crate::client::flows::post_pinboard_message(self, input).await
    }

    pub async fn get_content(&self, content_id: String) -> Result<(), EldError> {
        crate::client::flows::get_content(self, content_id).await
    }

    pub async fn get_cado(&self, path: String) -> Result<(), EldError> {
        crate::client::flows::get_cado(self, path).await
    }

    pub async fn pinboard_get_post(
        &self,
        wallet: String,
        message_id: String,
    ) -> Result<(), EldError> {
        crate::client::flows::pinboard_get_post(self, wallet, message_id).await
    }

    pub async fn pinboard_list_by_wallet(
        &self,
        wallet: String,
        page: usize,
        page_size: usize,
    ) -> Result<(), EldError> {
        crate::client::flows::pinboard_list_by_wallet(self, wallet, page, page_size).await
    }

    pub async fn pinboard_list_by_tag(
        &self,
        tag: String,
        page: usize,
        page_size: usize,
    ) -> Result<(), EldError> {
        crate::client::flows::pinboard_list_by_tag(self, tag, page, page_size).await
    }

    pub async fn list_cados(&self, search_string: String) -> Result<(), EldError> {
        crate::client::flows::list_cados(self, search_string).await
    }

    pub async fn get_account_from_cado(&self, path: String) -> Result<Account, EldError> {
        crate::client::query::get_account_from_cado(&self.config, path).await
    }
}
