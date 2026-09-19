//! AddNamespace transaction submit, namespace lookup, and registry polling.

use crate::app_api::AppApi;
use crate::client::ChainClient;
use crate::constants::tx_type;
use crate::error::{EldError, ErrorBuilder};
use crate::namespace::normalize_namespace_slug;
use crate::namespace_api::NamespaceRegisteredResponse;
use crate::tx::{AddNamespaceTx, Payload, Tx};
use std::time::Duration;
use tracing::{info, warn};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_POLLS: u32 = 60;

/// Query `GET /v1/namespace/{namespace_slug}` and print registry details.
pub(crate) async fn get_namespace(
    client: &ChainClient,
    namespace_slug: String,
) -> Result<(), EldError> {
    let canonical = normalize_namespace_slug(&namespace_slug)?;

    let app_api = AppApi::new(client.config.get_app_base_url()?)?;
    match app_api.get_namespace(&canonical).await? {
        Some(resp) => print_registered(&resp),
        None => {
            println!("registered: false");
            println!("namespace_slug: {canonical}");
        }
    }
    Ok(())
}

/// Submit `AddNamespace`, then poll `GET /v1/namespace/{namespace_slug}` until registered.
pub(crate) async fn add_namespace(
    client: &ChainClient,
    wallet_name: String,
    namespace_slug: String,
    registration_fee: u128,
) -> Result<(), EldError> {
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

    let dynamic_fee = crate::fee::calculate_dynamic_fee(&tx, &client.fee_config).map_err(|e| {
        ErrorBuilder::transaction_error(
            tx_type::TX_TYPE_ADD_NAMESPACE,
            &format!("Failed to calculate dynamic fee: {e}"),
        )
    })?;
    tx.fee = dynamic_fee.into();

    info!(
        namespace_slug = %canonical,
        registration_fee,
        dynamic_fee = dynamic_fee.amount(),
        "Submitting AddNamespace transaction"
    );

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
    info!("AddNamespace transaction sent successfully");

    poll_namespace_registered(client, &canonical).await
}

async fn poll_namespace_registered(
    client: &ChainClient,
    namespace_slug: &str,
) -> Result<(), EldError> {
    let app_api = AppApi::new(client.config.get_app_base_url()?)?;
    info!(
        namespace_slug,
        "Polling namespace registry until registered"
    );

    for attempt in 1..=MAX_POLLS {
        match app_api.get_namespace(namespace_slug).await {
            Ok(Some(resp)) => {
                print_registered(&resp);
                return Ok(());
            }
            Ok(None) => {
                info!(
                    attempt,
                    max_attempts = MAX_POLLS,
                    "Namespace not registered yet"
                );
            }
            Err(e) => {
                warn!(%e, attempt, "Namespace lookup failed; retrying");
            }
        }

        if attempt < MAX_POLLS {
            tokio::time::sleep(POLL_INTERVAL).await;
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

fn print_registered(resp: &NamespaceRegisteredResponse) {
    info!(
        namespace_slug = %resp.namespace_slug,
        owner = %resp.owner,
        registered_height = resp.registered_height,
        registry_path = %resp.registry_path,
        "Namespace registered"
    );
    println!("registered: {}", resp.registered);
    println!("namespace_slug: {}", resp.namespace_slug);
    println!("scope: {}", resp.scope);
    println!("owner: {}", resp.owner);
    println!("registered_height: {}", resp.registered_height);
    println!("registry_path: {}", resp.registry_path);
}
