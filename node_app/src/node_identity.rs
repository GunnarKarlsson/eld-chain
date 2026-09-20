//! Process-local node identity: TM consensus validator + capacity-validator app wallet.

use std::sync::{Arc, RwLock};

use eld_common::address::Address;
use eld_common::error::EldError;
use eld_common::wallet::Wallet;
use serde::Serialize;
use tendermint_rpc::{Client, HttpClient};
use tracing::{info, warn};

/// Env var: wallet name in `wallets.json` for this process's capacity-validator role.
pub const ELD_CAPACITY_VALIDATOR_WALLET_NAME_ENV: &str = "ELD_CAPACITY_VALIDATOR_WALLET_NAME";

/// Process-local identity: TM consensus validator + optional capacity-validator wallet address.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LocalNodeIdentity {
    /// Consensus validator address from Tendermint `/status` (20-byte Eld address).
    pub consensus_validator_address: Option<Address>,
    /// Capacity-validator wallet address from `ELD_CAPACITY_VALIDATOR_WALLET_NAME` + `wallets.json`.
    pub capacity_validator_address: Option<Address>,
}

/// REST view of [`LocalNodeIdentity`] plus related local addresses.
///
/// Wire fields remain hex strings (`0x` + lowercase) for JSON schema stability.
#[derive(Debug, Clone, Serialize)]
pub struct NodeIdentityResponse {
    pub consensus_validator_address: Option<String>,
    pub capacity_validator_address: Option<String>,
    pub capacity_provider_address: String,
}

impl LocalNodeIdentity {
    /// Returns true when the consensus validator address has been loaded from Tendermint.
    pub fn is_configured(&self) -> bool {
        self.consensus_validator_address.is_some()
    }

    /// Returns true when this process's capacity-validator wallet address is loaded.
    pub fn is_capacity_validator_configured(&self) -> bool {
        self.capacity_validator_address.is_some()
    }

    /// Compare this node's consensus address to another wire/TM form.
    pub fn matches_address(&self, other: &str) -> bool {
        match &self.consensus_validator_address {
            Some(addr) => addr.matches_str(other),
            None => false,
        }
    }

    /// True when this process's capacity-validator wallet is the epoch's selected validator.
    pub fn matches_capacity_validator_wallet(&self, selected_validator: &Address) -> bool {
        match &self.capacity_validator_address {
            Some(addr) => addr == selected_validator,
            None => false,
        }
    }
}

impl NodeIdentityResponse {
    /// Builds the REST DTO; formats typed addresses only at this wire boundary.
    pub fn from_identity(
        identity: &LocalNodeIdentity,
        capacity_provider_address: &Address,
    ) -> Self {
        Self {
            consensus_validator_address: identity
                .consensus_validator_address
                .map(|a| a.hex_with_prefix()),
            capacity_validator_address: identity
                .capacity_validator_address
                .map(|a| a.hex_with_prefix()),
            capacity_provider_address: capacity_provider_address.hex_with_prefix(),
        }
    }
}

/// Resolve capacity-validator wallet name from process env.
/// Missing or empty env is a fatal initialization error (no default wallet name).
pub fn resolve_capacity_validator_wallet_name_from_env() -> Result<String, EldError> {
    match std::env::var(ELD_CAPACITY_VALIDATOR_WALLET_NAME_ENV) {
        Err(_) => Err(EldError::InitializationError {
            component: "capacity_validator".into(),
            details: format!(
                "Environment variable {ELD_CAPACITY_VALIDATOR_WALLET_NAME_ENV} must be set to a wallet name from wallets.json"
            ),
        }),
        Ok(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                Err(EldError::InitializationError {
                    component: "capacity_validator".into(),
                    details: format!(
                        "{ELD_CAPACITY_VALIDATOR_WALLET_NAME_ENV} is set but empty; set it to a wallet name from wallets.json"
                    ),
                })
            } else {
                Ok(trimmed.to_string())
            }
        }
    }
}

/// Address for a capacity-validator wallet loaded from `wallets.json`.
pub fn capacity_validator_address_from_wallet(wallet: &Wallet) -> Address {
    wallet.address
}

/// Load local validator identity from the paired Tendermint node's `/status` RPC.
pub async fn fetch_from_tendermint_status(
    rpc_base_url: &str,
) -> Result<LocalNodeIdentity, EldError> {
    let client = HttpClient::new(rpc_base_url).map_err(|e| EldError::NetworkError {
        operation: "fetch_node_identity".to_string(),
        details: format!("Failed to create Tendermint HTTP client: {e}"),
    })?;

    let status = client.status().await.map_err(|e| EldError::NetworkError {
        operation: "fetch_node_identity".to_string(),
        details: format!("Tendermint status RPC failed: {e}"),
    })?;

    let address_raw = status.validator_info.address.to_string();
    let consensus_validator_address =
        Address::parse_hex_str(&address_raw).map_err(|e| EldError::NetworkError {
            operation: "fetch_node_identity".to_string(),
            details: format!("Invalid Tendermint validator address '{address_raw}': {e}"),
        })?;

    Ok(LocalNodeIdentity {
        consensus_validator_address: Some(consensus_validator_address),
        capacity_validator_address: None,
    })
}

/// Fetch `/status` and store TM consensus identity when not yet configured.
/// Preserves an already-set `capacity_validator_address` on the existing identity.
pub async fn ensure_identity_from_status(
    local_identity: &Arc<RwLock<LocalNodeIdentity>>,
    rpc_url: &str,
) {
    if local_identity
        .read()
        .map(|g| g.is_configured())
        .unwrap_or(false)
    {
        return;
    }

    match fetch_from_tendermint_status(rpc_url).await {
        Ok(fetched) => {
            info!(
                consensus_validator_address = ?fetched.consensus_validator_address,
                rpc_url = %rpc_url,
                "Local TM validator identity loaded from /status"
            );
            match local_identity.write() {
                Ok(mut guard) => {
                    guard.consensus_validator_address = fetched.consensus_validator_address;
                }
                Err(e) => warn!(
                    error = %e,
                    "Failed to store local TM validator identity (RwLock poisoned)"
                ),
            }
        }
        Err(e) => {
            warn!(
                error = %e,
                rpc_url = %rpc_url,
                "Could not load local TM validator identity from /status; will retry on next begin_block"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use eld_common::wallet::Wallet;

    const TM_RAW: &str = "8B81CC2BA41D4C29D0A0F90123396993491C3EA6";
    const TM_CANONICAL: &str = "0x8b81cc2ba41d4c29d0a0f90123396993491c3ea6";

    #[test]
    fn parse_consensus_address_tm_style() {
        let addr = Address::parse_hex_str(TM_RAW).expect("tm-style address");
        assert_eq!(addr.hex_with_prefix(), TM_CANONICAL);
    }

    #[test]
    fn parse_consensus_address_already_prefixed() {
        let addr = Address::parse_hex_str("0x8B81CC2BA41D4C29D0A0F90123396993491C3EA6")
            .expect("prefixed address");
        assert_eq!(addr.hex_with_prefix(), TM_CANONICAL);
    }

    #[test]
    fn matches_address_ignores_case_and_prefix() {
        let id = LocalNodeIdentity {
            consensus_validator_address: Some(Address::parse_hex_str(TM_RAW).expect("address")),
            capacity_validator_address: None,
        };
        assert!(id.matches_address(TM_CANONICAL));
        assert!(id.matches_address(TM_RAW));
        assert!(id.matches_address("0x8B81CC2BA41D4C29D0A0F90123396993491C3EA6"));
        assert!(!id.matches_address("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
        assert!(!LocalNodeIdentity::default().matches_address(TM_CANONICAL));
    }

    #[test]
    fn node_identity_response_formats_addresses_as_prefixed_hex() {
        let consensus = Address::parse_hex_str(TM_RAW).expect("consensus");
        let capacity =
            Address::parse_hex_str("0x014bb5f2250c33ddbb488d9d16258f70bf252883").expect("capacity");
        let provider =
            Address::parse_hex_str("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").expect("provider");

        let response = NodeIdentityResponse::from_identity(
            &LocalNodeIdentity {
                consensus_validator_address: Some(consensus),
                capacity_validator_address: Some(capacity),
            },
            &provider,
        );

        assert_eq!(
            response.consensus_validator_address.as_deref(),
            Some(TM_CANONICAL)
        );
        assert_eq!(
            response.capacity_validator_address.as_deref(),
            Some("0x014bb5f2250c33ddbb488d9d16258f70bf252883")
        );
        assert_eq!(
            response.capacity_provider_address,
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
    }

    #[test]
    fn matches_capacity_validator_wallet_compares_addresses() {
        let local =
            Address::parse_hex_str("0x014bb5f2250c33ddbb488d9d16258f70bf252883").expect("address");
        let id = LocalNodeIdentity {
            consensus_validator_address: None,
            capacity_validator_address: Some(local),
        };
        assert!(id.matches_capacity_validator_wallet(&local));
        assert!(!id.matches_capacity_validator_wallet(
            &Address::parse_hex_str("0x8b81cc2ba41d4c29d0a0f90123396993491c3ea6").expect("address")
        ));
        assert!(!LocalNodeIdentity::default().matches_capacity_validator_wallet(&local));
    }

    #[test]
    fn resolve_capacity_validator_wallet_name_errors_when_unset() {
        let prior = std::env::var(ELD_CAPACITY_VALIDATOR_WALLET_NAME_ENV).ok();
        std::env::remove_var(ELD_CAPACITY_VALIDATOR_WALLET_NAME_ENV);
        let err = resolve_capacity_validator_wallet_name_from_env().expect_err("must require env");
        assert!(matches!(err, EldError::InitializationError { .. }));
        if let Some(value) = prior {
            std::env::set_var(ELD_CAPACITY_VALIDATOR_WALLET_NAME_ENV, value);
        }
    }

    #[test]
    fn capacity_validator_address_from_wallet_returns_wallet_address() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let wallet =
            Wallet::from_signing_key("wallet-capacity-validator-1".to_string(), signing_key);
        let address = capacity_validator_address_from_wallet(&wallet);
        assert_eq!(address, wallet.address);
    }

    #[test]
    fn is_capacity_validator_configured_tracks_address() {
        assert!(!LocalNodeIdentity::default().is_capacity_validator_configured());
        let id = LocalNodeIdentity {
            consensus_validator_address: None,
            capacity_validator_address: Some(
                Address::parse_hex_str("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                    .expect("address"),
            ),
        };
        assert!(id.is_capacity_validator_configured());
    }
}
