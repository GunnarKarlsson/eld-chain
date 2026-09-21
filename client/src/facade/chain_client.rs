//! [`ChainClient`] — high-level Eld node client (RPC, REST, wallets, transactions).

#![warn(missing_docs)]

use crate::api::abci::AbciInfoWrapper;
use crate::api::rest::{NamespaceRegisteredResponse, PostMessageSubmitResponse};
use crate::config::client_config::{ClientConfig, FeeConfig};
use crate::facade::namespace::NamespaceLookup;
use crate::facade::submitted_tx::SubmittedTx;
use crate::wallet_store_config::WalletStoreConfig;
use eld_common::account::Account;
use eld_common::error::EldError;
use eld_common::nonce::Nonce;
use eld_common::staking_account::StakingAccount;
use eld_common::tx::Tx;
use eld_common::validator::{ActiveValidatorsInfo, EpochInfo};
use eld_common::wallet::Wallet;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use tendermint_rpc::endpoint::block::Response;

/// High-level client for Eld node RPC, app REST, local wallets, and signed transactions.
///
/// Construct with [`Self::new`] when you only need read-only queries, or [`Self::with_wallets`]
/// when calling wallet-dependent methods (`transfer`, `stake`, etc.).
#[derive(Clone)]
pub struct ChainClient {
    pub(crate) config: ClientConfig,
    pub(crate) fee_config: FeeConfig,
    wallet_store: Option<Arc<WalletStoreConfig>>,
}

impl ChainClient {
    /// Create a client from endpoint and fee config (no wallet file I/O).
    pub fn new(config: ClientConfig, fee_config: FeeConfig) -> Self {
        Self {
            config,
            fee_config,
            wallet_store: None,
        }
    }

    /// Create a client and bind a wallet JSON file at `wallet_path`.
    pub fn with_wallets(
        config: ClientConfig,
        fee_config: FeeConfig,
        wallet_path: impl AsRef<Path>,
    ) -> Result<Self, EldError> {
        Ok(Self {
            config,
            fee_config,
            wallet_store: Some(Arc::new(WalletStoreConfig::new(
                wallet_path.as_ref().to_path_buf(),
            )?)),
        })
    }

    fn wallet_store(&self) -> Result<&WalletStoreConfig, EldError> {
        self.wallet_store
            .as_deref()
            .ok_or_else(|| EldError::InitializationError {
                component: "ChainClient".to_string(),
                details: "wallet store not configured; use ChainClient::with_wallets".to_string(),
            })
    }

    fn wallet_store_config_from_path(wallet_path: impl AsRef<Path>) -> WalletStoreConfig {
        WalletStoreConfig::at_path(wallet_path.as_ref())
    }

    /// Request test funds from the configured faucet HTTP endpoint.
    pub async fn request_faucet(&self, address: String) -> Result<String, EldError> {
        crate::api::rest::faucet::request_faucet(&self.config, address).await
    }

    /// Chain ID from config (often set from `consensus_config.json` at startup).
    pub fn get_chain_id(&self) -> &str {
        &self.config.chain_id
    }

    /// Fee parameters used when signing transactions.
    pub fn get_fee_config(&self) -> &FeeConfig {
        &self.fee_config
    }

    /// Look up an account by hex address via ABCI (`None` if missing).
    pub async fn get_account_by_address(
        &self,
        address: String,
    ) -> Result<Option<Account>, EldError> {
        crate::api::abci::query::get_account_by_address(&self.config, address).await
    }

    /// Broadcast pre-encoded transaction bytes (hex) via Tendermint RPC.
    pub async fn send_tx_rpc(&self, hex_encoded: &str) -> Result<Value, EldError> {
        crate::api::abci::tx_broadcast::send_tx_rpc(&self.config, hex_encoded).await
    }

    /// Fetch a committed block by height.
    pub async fn get_block(&self, height: u64) -> Result<Response, EldError> {
        crate::api::abci::query::get_block(&self.config, height).await
    }

    /// Latest ABCI / Tendermint node info (sync status, block height).
    pub async fn get_abci_info(&self) -> Result<AbciInfoWrapper, EldError> {
        crate::facade::get_abci_info(self).await
    }

    /// Account balance and nonce by address (`None` if the account does not exist).
    pub async fn get_account(&self, address: String) -> Result<Option<Account>, EldError> {
        crate::facade::get_account(self, address).await
    }

    /// Staking account for a validator address, if present.
    pub async fn get_staking_account(
        &self,
        address: String,
    ) -> Result<Option<StakingAccount>, EldError> {
        crate::facade::get_staking_account(self, address).await
    }

    /// Next nonce to use when signing for `address` (account nonce + 1).
    pub async fn get_next_nonce_for_account(
        &self,
        address: String,
    ) -> Result<Option<Nonce>, EldError> {
        crate::api::abci::query::get_next_nonce_for_account(&self.config, address).await
    }

    /// Whether `provider_address` appears in on-chain `capacity_validators`.
    pub async fn is_capacity_provider_registered(
        &self,
        provider_address: &str,
    ) -> Result<bool, EldError> {
        crate::api::abci::query::is_capacity_provider_registered(&self.config, provider_address)
            .await
    }

    /// Next nonce derived from the CADO account path (legacy layout).
    pub async fn get_next_nonce_for_account_cado(
        &self,
        address: String,
    ) -> Result<Option<Nonce>, EldError> {
        crate::api::abci::query::get_next_nonce_for_account_cado(&self.config, address).await
    }

    /// Generate a new wallet and persist it to the bound wallet store.
    pub async fn create_wallet(&self, name: String) -> Result<Wallet, EldError> {
        crate::facade::wallets::create_wallet_with_store_config(name, self.wallet_store()?).await
    }

    /// Generate a new wallet using an explicit [`WalletStoreConfig`].
    pub async fn create_wallet_with_store_config(
        &self,
        name: String,
        wallet_store_config: &WalletStoreConfig,
    ) -> Result<Wallet, EldError> {
        crate::facade::wallets::create_wallet_with_store_config(name, wallet_store_config).await
    }

    /// Generate a new wallet and write to `wallet_path`.
    pub async fn create_wallet_from_path(
        &self,
        name: String,
        wallet_path: impl AsRef<Path>,
    ) -> Result<Wallet, EldError> {
        let wallet_store_config = Self::wallet_store_config_from_path(wallet_path);
        self.create_wallet_with_store_config(name, &wallet_store_config)
            .await
    }

    /// List wallet names from the bound wallet store.
    pub async fn list_wallets(&self) -> Result<Vec<Wallet>, EldError> {
        crate::facade::wallets::list_wallets_with_store_config(self.wallet_store()?).await
    }

    /// List wallets from an explicit [`WalletStoreConfig`].
    pub async fn list_wallets_with_store_config(
        &self,
        wallet_store_config: &WalletStoreConfig,
    ) -> Result<Vec<Wallet>, EldError> {
        crate::facade::wallets::list_wallets_with_store_config(wallet_store_config).await
    }

    /// List wallets stored at `wallet_path`.
    pub async fn list_wallets_from_path(
        &self,
        wallet_path: impl AsRef<Path>,
    ) -> Result<Vec<Wallet>, EldError> {
        self.get_wallets_from_path(wallet_path).await
    }

    /// Load all wallets from the bound wallet store.
    pub async fn get_wallets(&self) -> Result<Vec<Wallet>, EldError> {
        crate::facade::wallets::get_wallets_with_store_config(self.wallet_store()?).await
    }

    /// Load all wallets from `wallet_path` without binding a store on the client.
    pub async fn get_wallets_from_path(
        &self,
        wallet_path: impl AsRef<Path>,
    ) -> Result<Vec<Wallet>, EldError> {
        WalletStoreConfig::load_wallets_from_path(wallet_path)
    }

    /// Remove a wallet by name from the bound store (`true` if it existed).
    pub async fn remove_wallet(&self, name: String) -> Result<bool, EldError> {
        crate::facade::wallets::remove_wallet_with_store_config(name, self.wallet_store()?).await
    }

    /// Remove a wallet using an explicit [`WalletStoreConfig`].
    pub async fn remove_wallet_with_store_config(
        &self,
        name: String,
        wallet_store_config: &WalletStoreConfig,
    ) -> Result<bool, EldError> {
        crate::facade::wallets::remove_wallet_with_store_config(name, wallet_store_config).await
    }

    /// Remove a wallet from `wallet_path`.
    pub async fn remove_wallet_from_path(
        &self,
        name: String,
        wallet_path: impl AsRef<Path>,
    ) -> Result<bool, EldError> {
        let wallet_store_config = Self::wallet_store_config_from_path(wallet_path);
        self.remove_wallet_with_store_config(name, &wallet_store_config)
            .await
    }

    /// Look up a wallet by name in the bound store.
    pub async fn get_wallet_by_name(&self, name: String) -> Result<Option<Wallet>, EldError> {
        crate::facade::wallets::get_wallet_by_name_with_store_config(name, self.wallet_store()?)
            .await
    }

    /// Look up a wallet by name with an explicit [`WalletStoreConfig`].
    pub async fn get_wallet_by_name_with_store_config(
        &self,
        name: String,
        wallet_store_config: &WalletStoreConfig,
    ) -> Result<Option<Wallet>, EldError> {
        crate::facade::wallets::get_wallet_by_name_with_store_config(name, wallet_store_config)
            .await
    }

    /// Look up a wallet by name in `wallet_path`.
    pub async fn get_wallet_by_name_from_path(
        &self,
        name: String,
        wallet_path: impl AsRef<Path>,
    ) -> Result<Option<Wallet>, EldError> {
        let wallets = self.get_wallets_from_path(wallet_path).await?;
        Ok(wallets.into_iter().find(|w| w.name == name))
    }

    /// Hex-encoded `0x` address for a wallet name (used as capacity provider id on nodes).
    pub async fn get_provider_id_for_capacity(
        &self,
        wallet_name: &str,
    ) -> Result<String, EldError> {
        crate::facade::get_provider_id_for_capacity(self, wallet_name).await
    }

    /// Find a wallet in the bound store by hex address.
    pub async fn get_wallet_by_address(&self, address: &str) -> Result<Option<Wallet>, EldError> {
        crate::facade::wallets::get_wallet_by_address_with_store_config(
            address,
            self.wallet_store()?,
        )
        .await
    }

    /// Sign and broadcast a transfer; waits for commit via `broadcast_tx_commit`.
    pub async fn transfer(
        &self,
        wallet_name: String,
        recipient: String,
        amount: u128,
    ) -> Result<SubmittedTx, EldError> {
        crate::facade::transfer(self, wallet_name, recipient, amount).await
    }

    /// Scan every block from tip to genesis and decode Eld transactions (expensive; debug tooling).
    pub async fn list_all_transactions(&self) -> Result<Vec<Tx>, EldError> {
        crate::facade::list_all_transactions(self).await
    }

    /// Tendermint tx search results for transactions involving `addr`.
    pub async fn list_transactions(
        &self,
        addr: String,
    ) -> Result<Vec<tendermint_rpc::endpoint::tx::Response>, EldError> {
        crate::facade::list_transactions(self, addr).await
    }

    /// Stake `amount` from `wallet_name` into the staking module.
    pub async fn stake(&self, wallet_name: String, amount: u128) -> Result<SubmittedTx, EldError> {
        crate::facade::stake(self, wallet_name, amount).await
    }

    /// Unstake `amount` from `wallet_name`.
    pub async fn unstake(
        &self,
        wallet_name: String,
        amount: u128,
    ) -> Result<SubmittedTx, EldError> {
        crate::facade::unstake(self, wallet_name, amount).await
    }

    /// Active validator set for the current epoch.
    pub async fn view_active_validators(&self) -> Result<Option<ActiveValidatorsInfo>, EldError> {
        crate::facade::view_active_validators(self).await
    }

    /// Current epoch metadata.
    pub async fn view_epoch_info(&self) -> Result<EpochInfo, EldError> {
        crate::facade::view_epoch_info(self).await
    }

    /// Epoch info plus active validators in one call.
    pub async fn view_epoch(&self) -> Result<(EpochInfo, ActiveValidatorsInfo), EldError> {
        crate::facade::view_epoch(self).await
    }

    /// Register a namespace slug on-chain (signed tx + poll until REST shows registration).
    pub async fn add_namespace(
        &self,
        wallet_name: String,
        namespace_slug: String,
        registration_fee: u128,
    ) -> Result<NamespaceRegisteredResponse, EldError> {
        crate::facade::add_namespace(self, wallet_name, namespace_slug, registration_fee).await
    }

    /// Query namespace registration via app REST.
    pub async fn get_namespace(&self, namespace_slug: String) -> Result<NamespaceLookup, EldError> {
        crate::facade::get_namespace(self, namespace_slug).await
    }

    /// Post a pinboard message (content upload + signed `PostMessage` tx).
    pub async fn post_pinboard_message(
        &self,
        input: crate::api::rest::PinboardMessageParams,
    ) -> Result<PostMessageSubmitResponse, EldError> {
        crate::facade::post_pinboard_message(self, input).await
    }

    /// Fetch pinboard content bytes by content id (hex hash).
    pub async fn get_content(&self, content_id: String) -> Result<String, EldError> {
        crate::facade::get_content(self, content_id).await
    }

    /// Read a CADO path via ABCI query (JSON value).
    pub async fn get_cado(&self, path: String) -> Result<Value, EldError> {
        crate::facade::get_cado(self, path).await
    }

    /// Fetch one pinboard post by wallet address and message id.
    pub async fn pinboard_get_post(
        &self,
        wallet: String,
        message_id: String,
    ) -> Result<Value, EldError> {
        crate::facade::pinboard_get_post(self, wallet, message_id).await
    }

    /// Paginated pinboard posts for a wallet.
    pub async fn pinboard_list_by_wallet(
        &self,
        wallet: String,
        page: usize,
        page_size: usize,
    ) -> Result<Value, EldError> {
        crate::facade::pinboard_list_by_wallet(self, wallet, page, page_size).await
    }

    /// Paginated pinboard posts by tag.
    pub async fn pinboard_list_by_tag(
        &self,
        tag: String,
        page: usize,
        page_size: usize,
    ) -> Result<Value, EldError> {
        crate::facade::pinboard_list_by_tag(self, tag, page, page_size).await
    }

    /// List CADO path keys matching a prefix search string.
    pub async fn list_cados(&self, search_string: String) -> Result<Vec<String>, EldError> {
        crate::facade::list_cados(self, search_string).await
    }

    /// Load an account stored at a CADO path (legacy account layout).
    pub async fn get_account_from_cado(&self, path: String) -> Result<Account, EldError> {
        crate::api::abci::query::get_account_from_cado(&self.config, path).await
    }
}
