//! AddNamespace transaction submit, namespace lookup, and registry polling.

use crate::app_api::AppApi;
use crate::client::ChainClient;
use crate::constants::tx_type;
use crate::error::ErrorBuilder;
use crate::namespace::normalize_namespace_slug;
use crate::namespace_api::NamespaceRegisteredResponse;
use crate::tx::{AddNamespaceTx, Payload, Tx};
use std::time::Duration;
use tracing::{error, info, warn};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_POLLS: u32 = 60;

/// Query `GET /v1/namespace/{namespace_slug}` and print registry details.
pub(crate) async fn get_namespace(client: &ChainClient, namespace_slug: String) {
    let canonical = match normalize_namespace_slug(&namespace_slug) {
        Ok(s) => s,
        Err(e) => {
            error!(%e, "Invalid namespace_slug");
            return;
        }
    };

    let app_api = AppApi::new(client.config.get_app_base_url());
    match app_api.get_namespace(&canonical).await {
        Ok(Some(resp)) => print_registered(&resp),
        Ok(None) => {
            println!("registered: false");
            println!("namespace_slug: {canonical}");
        }
        Err(e) => error!(%e, "Namespace lookup failed"),
    }
}

/// Submit `AddNamespace`, then poll `GET /v1/namespace/{namespace_slug}` until registered.
pub(crate) async fn add_namespace(
    client: &ChainClient,
    wallet_name: String,
    namespace_slug: String,
    registration_fee: u128,
) {
    let canonical = match normalize_namespace_slug(&namespace_slug) {
        Ok(s) => s,
        Err(e) => {
            error!(%e, "Invalid namespace_slug");
            return;
        }
    };

    let wallet = match client.get_wallet_by_name(wallet_name.clone()).await {
        Some(w) => w,
        None => {
            error!(error = %(ErrorBuilder::wallet_error("retrieval", &wallet_name, "Wallet not found")));
            return;
        }
    };

    let next_nonce = match client
        .get_next_nonce_for_account_cado(wallet.address.hex_with_prefix())
        .await
    {
        Some(nonce) => nonce,
        None => {
            error!(error = %(ErrorBuilder::network_error("nonce retrieval", "Failed to get account nonce")));
            return;
        }
    };

    let add_namespace_tx =
        match AddNamespaceTx::new(wallet.address, canonical.clone(), registration_fee.into()) {
            Ok(t) => t,
            Err(e) => {
                error!(%e);
                return;
            }
        };

    let mut tx = Tx::new(
        next_nonce,
        Payload::new(add_namespace_tx),
        hex::encode(wallet.public_key),
    );

    let dynamic_fee = match crate::fee::calculate_dynamic_fee(&tx, &client.fee_config) {
        Ok(fee) => fee,
        Err(e) => {
            error!(error = %(ErrorBuilder::transaction_error(
                tx_type::TX_TYPE_ADD_NAMESPACE,
                &format!("Failed to calculate dynamic fee: {e}"),
            )));
            return;
        }
    };
    tx.fee = dynamic_fee.into();

    info!(
        namespace_slug = %canonical,
        registration_fee,
        dynamic_fee = dynamic_fee.amount(),
        "Submitting AddNamespace transaction"
    );

    wallet.sign(&mut tx, &client.config.chain_id);
    let json = match serde_json::to_string(&tx) {
        Ok(json) => json,
        Err(e) => {
            error!(error = %(ErrorBuilder::transaction_error(
                tx_type::TX_TYPE_ADD_NAMESPACE,
                &format!("Failed to serialize transaction: {e}"),
            )));
            return;
        }
    };
    let hex = hex::encode(&json);

    if !wallet.verify(&tx, &client.config.chain_id) {
        error!(error = %(ErrorBuilder::transaction_error(
            tx_type::TX_TYPE_ADD_NAMESPACE,
            "Transaction verification failed"
        )));
        return;
    }

    match client.send_tx_rpc(&hex).await {
        Ok(_response) => info!("AddNamespace transaction sent successfully"),
        Err(e) => {
            error!(%e, "Failed to broadcast AddNamespace transaction");
            return;
        }
    }

    poll_namespace_registered(client, &canonical).await;
}

async fn poll_namespace_registered(client: &ChainClient, namespace_slug: &str) {
    let app_api = AppApi::new(client.config.get_app_base_url());
    info!(
        namespace_slug,
        "Polling namespace registry until registered"
    );

    for attempt in 1..=MAX_POLLS {
        match app_api.get_namespace(namespace_slug).await {
            Ok(Some(resp)) => {
                print_registered(&resp);
                return;
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

    error!(
        namespace_slug,
        timeout_secs = POLL_INTERVAL.as_secs() * u64::from(MAX_POLLS),
        "Timed out waiting for namespace registration"
    );
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
