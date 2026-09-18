//! Transfer, stake, unstake, and transaction listing flows.

use crate::abci_api::AbciHttpApi;
use crate::address::Address;
use crate::client::ChainClient;
use crate::constants::tx_type;
use crate::error::ErrorBuilder;
use crate::logging::SanitizedLog;
use crate::tx::{Payload, StakeTx, TransferTx, Tx, UnstakeTx};
use tracing::{error, info};

pub(crate) async fn transfer(
    client: &ChainClient,
    wallet_name: String,
    recipient: String,
    amount: u128,
) {
    if let Err(e) = crate::validation::validate_address(&recipient) {
        error!(%e);
        return;
    }

    let recipient_address = match Address::parse_hex_str(&recipient) {
        Ok(addr) => addr,
        Err(e) => {
            error!(error = %(ErrorBuilder::validation_error("recipient address", &recipient, &e.to_string())));
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

    let transfer = match TransferTx::new(wallet.address, recipient_address, amount.into()) {
        Ok(t) => t,
        Err(e) => {
            error!(%e);
            return;
        }
    };
    let mut tx = Tx::new(
        next_nonce,
        Payload::new(transfer),
        hex::encode(wallet.public_key),
    );

    let dynamic_fee = match crate::fee::calculate_dynamic_fee(&tx, &client.fee_config) {
        Ok(fee) => fee,
        Err(e) => {
            error!(error = %(ErrorBuilder::transaction_error(
                tx_type::TX_TYPE_TRANSFER,
                &format!("Failed to calculate dynamic fee: {e}"),
            )));
            return;
        }
    };
    tx.fee = dynamic_fee.into();

    info!("Calculated dynamic fee: {} units", dynamic_fee);

    wallet.sign(&mut tx, &client.config.chain_id);
    let json = match serde_json::to_string(&tx) {
        Ok(json) => json,
        Err(e) => {
            error!(error = %(ErrorBuilder::transaction_error(
                tx_type::TX_TYPE_TRANSFER,
                &format!("Failed to serialize transaction: {e}"),
            )));
            return;
        }
    };
    info!("Rust JSON: {}", json);
    let hex = hex::encode(&json);

    if !wallet.verify(&tx, &client.config.chain_id) {
        error!(error = %(ErrorBuilder::transaction_error("Transfer", "Transaction verification failed")));
        return;
    }

    match client.send_tx_rpc(&hex).await {
        Ok(_response) => info!("✅ Transfer transaction sent successfully"),
        Err(e) => error!(%e),
    }
}

pub(crate) async fn list_all_transactions(client: &ChainClient) {
    let api = AbciHttpApi::new(client.config.get_node_url().to_owned());
    let abci_info = api
        .get_latest_abci_info()
        .await
        .expect("Failed to get latest ABCI info");
    info!("Transactions:\n");
    let mut current_block = abci_info.last_block_height.value();
    while current_block > 0 {
        let block = api
            .get_block(current_block)
            .await
            .expect("Failed to get block");
        if !block.block.data.is_empty() {
            let data_list = block.block.data;
            for item in data_list {
                let hex_str =
                    String::from_utf8(item).expect("Failed to convert item to UTF-8 string");
                let decoded_json =
                    String::from_utf8(hex::decode(&hex_str).expect("Failed to decode hex string"))
                        .expect("Failed to convert decoded bytes to UTF-8 string");
                let tx: Tx = serde_json::from_str(&decoded_json)
                    .expect("Failed to deserialize JSON to transaction");
                info!("tx: {}", SanitizedLog::new(tx));
            }
        }
        current_block -= 1;
    }
}

pub(crate) async fn list_transactions(client: &ChainClient, addr: String) {
    let api = AbciHttpApi::new(client.config.get_node_url().to_owned());
    let txs = api.get_transactions_for_account(addr).await;
    info!("\nTxs:\n");
    for tx_response in txs {
        info!(
            "tx hash: {}",
            SanitizedLog::as_hash(tx_response.hash.to_string())
        );
        for event in tx_response.tx_result.events {
            info!("event kind: {}", event.kind.to_string());
            for attribute in event.attributes {
                let (key, value) = super::util::decode_event_attribute(attribute);
                info!("{key}:{value}");
            }
        }
        info!("\n");
    }
}

pub(crate) async fn stake(client: &ChainClient, wallet_name: String, amount: u128) {
    info!("Stake");
    let wallet = client
        .get_wallet_by_name(wallet_name)
        .await
        .expect("Can get selected wallet");

    let next_nonce = client
        .get_next_nonce_for_account(wallet.address.hex_with_prefix())
        .await
        .expect("Can get nonce for selected wallet/account");
    info!("next_nonce: {}", next_nonce);

    let stake_tx = match StakeTx::new(
        wallet.address,
        amount.into(),
        Some(hex::encode(wallet.public_key)),
    ) {
        Ok(t) => t,
        Err(e) => {
            error!(%e);
            return;
        }
    };

    let mut tx = Tx::new(
        next_nonce,
        Payload::new(stake_tx),
        hex::encode(wallet.public_key),
    );

    let dynamic_fee = match crate::fee::calculate_dynamic_fee(&tx, &client.fee_config) {
        Ok(fee) => fee,
        Err(e) => {
            error!(error = %(ErrorBuilder::transaction_error(
                tx_type::TX_TYPE_STAKE,
                &format!("Failed to calculate dynamic fee: {e}"),
            )));
            return;
        }
    };
    tx.fee = dynamic_fee.into();

    info!("Calculated dynamic fee: {} units", dynamic_fee);

    wallet.sign(&mut tx, &client.config.chain_id);
    let json = match serde_json::to_string(&tx) {
        Ok(json) => json,
        Err(e) => {
            error!(error = %(ErrorBuilder::transaction_error(
                tx_type::TX_TYPE_STAKE,
                &format!("Failed to serialize transaction: {e}"),
            )));
            return;
        }
    };
    info!("Rust JSON: {}", json);
    let hex = hex::encode(&json);

    if !wallet.verify(&tx, &client.config.chain_id) {
        error!(error = %(ErrorBuilder::transaction_error("Stake", "Transaction verification failed")));
        return;
    }

    match client.send_tx_rpc(&hex).await {
        Ok(_response) => info!("✅ Stake transaction sent successfully"),
        Err(e) => error!("Error sending stake transaction: {}", e.to_string()),
    }
}

pub(crate) async fn unstake(client: &ChainClient, wallet_name: String, amount: u128) {
    info!("Unstake");
    let wallet = client
        .get_wallet_by_name(wallet_name)
        .await
        .expect("Can get selected wallet");

    let next_nonce = client
        .get_next_nonce_for_account(wallet.address.hex_with_prefix())
        .await
        .expect("Can get nonce for selected wallet/account");

    let unstake_tx = match UnstakeTx::new(wallet.address, amount.into()) {
        Ok(t) => t,
        Err(e) => {
            error!(%e);
            return;
        }
    };

    let mut tx = Tx::new(
        next_nonce,
        Payload::new(unstake_tx),
        hex::encode(wallet.public_key),
    );

    let dynamic_fee = match crate::fee::calculate_dynamic_fee(&tx, &client.fee_config) {
        Ok(fee) => fee,
        Err(e) => {
            error!(error = %(ErrorBuilder::transaction_error(
                "Unstake",
                &format!("Failed to calculate dynamic fee: {e}"),
            )));
            return;
        }
    };
    tx.fee = dynamic_fee.into();

    info!("Calculated dynamic fee: {} units", dynamic_fee);

    wallet.sign(&mut tx, &client.config.chain_id);
    let json = match serde_json::to_string(&tx) {
        Ok(json) => json,
        Err(e) => {
            error!(error = %(ErrorBuilder::transaction_error(
                "Unstake",
                &format!("Failed to serialize transaction: {e}"),
            )));
            return;
        }
    };
    info!("Rust JSON: {}", json);
    let hex = hex::encode(&json);

    if !wallet.verify(&tx, &client.config.chain_id) {
        error!(error = %(ErrorBuilder::transaction_error("Unstake", "Transaction verification failed")));
        return;
    }

    match client.send_tx_rpc(&hex).await {
        Ok(_response) => info!("✅ Unstake transaction sent successfully"),
        Err(e) => error!(%e),
    }
}
