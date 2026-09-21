//! AddNamespace transaction submit, namespace lookup, and registry polling.

use super::ChainClient;
use crate::api::rest::AppApi;
use crate::api::rest::NamespaceRegisteredResponse;
use eld_common::constants::tx_type;
use eld_common::error::{EldError, ErrorBuilder};
use eld_common::namespace::normalize_namespace_slug;
use eld_common::tx::{AddNamespaceTx, Payload, Tx};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_POLLS: u32 = 60;

/// Canonical slug plus optional on-chain registry row.
pub struct NamespaceLookup {
    pub canonical_slug: String,
    pub registered: Option<NamespaceRegisteredResponse>,
}

/// Query `GET /v1/namespace/{namespace_slug}`.
pub(crate) async fn get_namespace(
    client: &ChainClient,
    namespace_slug: String,
) -> Result<NamespaceLookup, EldError> {
    let canonical = normalize_namespace_slug(&namespace_slug)?;

    let app_api = AppApi::new(client.config.get_app_base_url()?)?;
    let registered = app_api.get_namespace(&canonical).await?;
    Ok(NamespaceLookup {
        canonical_slug: canonical,
        registered,
    })
}

/// Submit `AddNamespace`, then poll `GET /v1/namespace/{namespace_slug}` until registered.
pub(crate) async fn add_namespace(
    client: &ChainClient,
    wallet_name: String,
    namespace_slug: String,
    registration_fee: u128,
) -> Result<NamespaceRegisteredResponse, EldError> {
    let canonical = normalize_namespace_slug(&namespace_slug)?;

    let wallet = super::util::require_wallet(client, &wallet_name).await?;
    let next_nonce = super::util::require_nonce(
        client
            .get_next_nonce_for_account_cado(wallet.address.hex_with_prefix())
            .await,
    )?;

    let add_namespace_tx =
        AddNamespaceTx::new(wallet.address, canonical.clone(), registration_fee.into())?;

    let mut tx = Tx::new(
        next_nonce,
        Payload::new(add_namespace_tx),
        wallet.verifying_key(),
    );

    let dynamic_fee =
        eld_common::fee::calculate_dynamic_fee(&tx, &client.fee_config).map_err(|e| {
            ErrorBuilder::transaction_error(
                tx_type::TX_TYPE_ADD_NAMESPACE,
                &format!("Failed to calculate dynamic fee: {e}"),
            )
        })?;
    tx.fee = dynamic_fee.into();

    wallet.sign(&mut tx, &client.config.chain_id)?;
    let json = serde_json::to_string(&tx).map_err(|e| {
        ErrorBuilder::transaction_error(
            tx_type::TX_TYPE_ADD_NAMESPACE,
            &format!("Failed to serialize transaction: {e}"),
        )
    })?;
    let hex = hex::encode(&json);

    if !wallet.verify(&tx, &client.config.chain_id)? {
        return Err(ErrorBuilder::transaction_error(
            tx_type::TX_TYPE_ADD_NAMESPACE,
            "Transaction verification failed",
        ));
    }

    client.send_tx_rpc(&hex).await?;
    poll_namespace_registered(client, &canonical).await
}

async fn poll_namespace_registered(
    client: &ChainClient,
    namespace_slug: &str,
) -> Result<NamespaceRegisteredResponse, EldError> {
    let app_api = AppApi::new(client.config.get_app_base_url()?)?;

    for attempt in 1..=MAX_POLLS {
        match app_api.get_namespace(namespace_slug).await {
            Ok(Some(resp)) => return Ok(resp),
            Ok(None) | Err(_) => {
                if attempt < MAX_POLLS {
                    tokio::time::sleep(POLL_INTERVAL).await;
                }
            }
        }
    }

    Err(ErrorBuilder::network_error(
        "poll namespace registration",
        &format!(
            "Timed out waiting for namespace {namespace_slug} after {} seconds",
            POLL_INTERVAL.as_secs() * u64::from(MAX_POLLS)
        ),
    ))
}
