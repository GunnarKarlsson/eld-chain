//! Transfer, stake, unstake, and transaction listing flows.

use crate::abci_api::AbciHttpApi;
use crate::address::Address;
use crate::client::ChainClient;
use crate::constants::tx_type;
use crate::error::{EldError, ErrorBuilder};
use crate::logging::SanitizedLog;
use crate::tx::{Payload, StakeTx, TransferTx, Tx, UnstakeTx};
use tracing::info;

pub(crate) async fn transfer(
    client: &ChainClient,
    wallet_name: String,
    recipient: String,
    amount: u128,
) -> Result<(), EldError> {
    crate::validation::validate_address(&recipient)?;

    let recipient_address = Address::parse_hex_str(&recipient).map_err(|e| {
        ErrorBuilder::validation_error("recipient address", &recipient, &e.to_string())
    })?;

    let wallet = super::util::require_wallet(client, &wallet_name).await?;
    let next_nonce = super::util::require_nonce(
        client
            .get_next_nonce_for_account_cado(wallet.address.hex_with_prefix())
            .await,
    )?;

    let transfer = TransferTx::new(wallet.address, recipient_address, amount.into())?;
    let mut tx = Tx::new(next_nonce, Payload::new(transfer), wallet.verifying_key());

    let dynamic_fee = crate::fee::calculate_dynamic_fee(&tx, &client.fee_config).map_err(|e| {
        ErrorBuilder::transaction_error(
            tx_type::TX_TYPE_TRANSFER,
            &format!("Failed to calculate dynamic fee: {e}"),
        )
    })?;
    tx.fee = dynamic_fee.into();

    info!("Calculated dynamic fee: {} units", dynamic_fee);

    wallet.sign(&mut tx, &client.config.chain_id)?;
    let json = serde_json::to_string(&tx).map_err(|e| {
        ErrorBuilder::transaction_error(
            tx_type::TX_TYPE_TRANSFER,
            &format!("Failed to serialize transaction: {e}"),
        )
    })?;
    info!("Rust JSON: {}", json);
    let hex = hex::encode(&json);

    if !wallet.verify(&tx, &client.config.chain_id)? {
        return Err(ErrorBuilder::transaction_error(
            "Transfer",
            "Transaction verification failed",
        ));
    }

    client.send_tx_rpc(&hex).await?;
    info!("Transfer transaction sent successfully");
    Ok(())
}

pub(crate) async fn list_all_transactions(client: &ChainClient) -> Result<(), EldError> {
    let api = AbciHttpApi::new(client.config.get_node_url()?)?;
    let abci_info = api.get_latest_abci_info().await?;
    info!("Transactions:\n");
    let mut current_block = abci_info.last_block_height.value();
    while current_block > 0 {
        let block = api.get_block(current_block).await?;
        if !block.block.data.is_empty() {
            for item in block.block.data {
                let hex_str = String::from_utf8(item).map_err(|e| {
                    ErrorBuilder::validation_error(
                        "block_tx_bytes",
                        &current_block.to_string(),
                        &e.to_string(),
                    )
                })?;
                let decoded_bytes = hex::decode(&hex_str).map_err(|e| {
                    ErrorBuilder::validation_error("block_tx_hex", &hex_str, &e.to_string())
                })?;
                let decoded_json = String::from_utf8(decoded_bytes).map_err(|e| {
                    ErrorBuilder::validation_error("block_tx_json", &hex_str, &e.to_string())
                })?;
                let tx: Tx = serde_json::from_str(&decoded_json).map_err(|e| {
                    ErrorBuilder::validation_error("block_tx", &decoded_json, &e.to_string())
                })?;
                info!("tx: {}", SanitizedLog::new(tx));
            }
        }
        current_block -= 1;
    }
    Ok(())
}

pub(crate) async fn list_transactions(client: &ChainClient, addr: String) -> Result<(), EldError> {
    let api = AbciHttpApi::new(client.config.get_node_url()?)?;
    let txs = api.get_transactions_for_account(addr).await?;
    info!("\nTxs:\n");
    for tx_response in txs {
        info!(
            "tx hash: {}",
            SanitizedLog::as_hash(tx_response.hash.to_string())
        );
        for event in tx_response.tx_result.events {
            info!("event kind: {}", event.kind);
            for attribute in event.attributes {
                let (key, value) = super::util::decode_event_attribute(attribute)?;
                info!("{key}:{value}");
            }
        }
        info!("\n");
    }
    Ok(())
}

pub(crate) async fn stake(
    client: &ChainClient,
    wallet_name: String,
    amount: u128,
) -> Result<(), EldError> {
    info!("Stake");
    let wallet = super::util::require_wallet(client, &wallet_name).await?;
    let next_nonce = super::util::require_nonce(
        client
            .get_next_nonce_for_account(wallet.address.hex_with_prefix())
            .await,
    )?;
    info!("next_nonce: {}", next_nonce);

    let stake_tx = StakeTx::new(
        wallet.address,
        amount.into(),
        Some(hex::encode(wallet.public_key)),
    )?;

    let mut tx = Tx::new(next_nonce, Payload::new(stake_tx), wallet.verifying_key());

    let dynamic_fee = crate::fee::calculate_dynamic_fee(&tx, &client.fee_config).map_err(|e| {
        ErrorBuilder::transaction_error(
            tx_type::TX_TYPE_STAKE,
            &format!("Failed to calculate dynamic fee: {e}"),
        )
    })?;
    tx.fee = dynamic_fee.into();

    info!("Calculated dynamic fee: {} units", dynamic_fee);

    wallet.sign(&mut tx, &client.config.chain_id)?;
    let json = serde_json::to_string(&tx).map_err(|e| {
        ErrorBuilder::transaction_error(
            tx_type::TX_TYPE_STAKE,
            &format!("Failed to serialize transaction: {e}"),
        )
    })?;
    info!("Rust JSON: {}", json);
    let hex = hex::encode(&json);

    if !wallet.verify(&tx, &client.config.chain_id)? {
        return Err(ErrorBuilder::transaction_error(
            "Stake",
            "Transaction verification failed",
        ));
    }

    client.send_tx_rpc(&hex).await?;
    info!("Stake transaction sent successfully");
    Ok(())
}

pub(crate) async fn unstake(
    client: &ChainClient,
    wallet_name: String,
    amount: u128,
) -> Result<(), EldError> {
    info!("Unstake");
    let wallet = super::util::require_wallet(client, &wallet_name).await?;
    let next_nonce = super::util::require_nonce(
        client
            .get_next_nonce_for_account(wallet.address.hex_with_prefix())
            .await,
    )?;

    let unstake_tx = UnstakeTx::new(wallet.address, amount.into())?;

    let mut tx = Tx::new(next_nonce, Payload::new(unstake_tx), wallet.verifying_key());

    let dynamic_fee = crate::fee::calculate_dynamic_fee(&tx, &client.fee_config).map_err(|e| {
        ErrorBuilder::transaction_error("Unstake", &format!("Failed to calculate dynamic fee: {e}"))
    })?;
    tx.fee = dynamic_fee.into();

    info!("Calculated dynamic fee: {} units", dynamic_fee);

    wallet.sign(&mut tx, &client.config.chain_id)?;
    let json = serde_json::to_string(&tx).map_err(|e| {
        ErrorBuilder::transaction_error("Unstake", &format!("Failed to serialize transaction: {e}"))
    })?;
    info!("Rust JSON: {}", json);
    let hex = hex::encode(&json);

    if !wallet.verify(&tx, &client.config.chain_id)? {
        return Err(ErrorBuilder::transaction_error(
            "Unstake",
            "Transaction verification failed",
        ));
    }

    client.send_tx_rpc(&hex).await?;
    info!("Unstake transaction sent successfully");
    Ok(())
}
