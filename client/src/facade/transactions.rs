//! Transfer, stake, unstake, and transaction listing flows.

use super::submitted_tx::SubmittedTx;
use super::ChainClient;
use crate::api::abci::AbciHttpApi;
use eld_common::address::Address;
use eld_common::constants::tx_type;
use eld_common::error::{EldError, ErrorBuilder};
use eld_common::tx::{Payload, StakeTx, TransferTx, Tx, UnstakeTx};

pub(crate) async fn transfer(
    client: &ChainClient,
    wallet_name: String,
    recipient: String,
    amount: u128,
) -> Result<SubmittedTx, EldError> {
    eld_common::validation::validate_address(&recipient)?;

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

    let dynamic_fee =
        eld_common::fee::calculate_dynamic_fee(&tx, &client.fee_config).map_err(|e| {
            ErrorBuilder::transaction_error(
                tx_type::TX_TYPE_TRANSFER,
                &format!("Failed to calculate dynamic fee: {e}"),
            )
        })?;
    tx.fee = dynamic_fee.into();

    wallet.sign(&mut tx, &client.config.chain_id)?;
    let json = serde_json::to_string(&tx).map_err(|e| {
        ErrorBuilder::transaction_error(
            tx_type::TX_TYPE_TRANSFER,
            &format!("Failed to serialize transaction: {e}"),
        )
    })?;
    let hex = hex::encode(&json);

    if !wallet.verify(&tx, &client.config.chain_id)? {
        return Err(ErrorBuilder::transaction_error(
            "Transfer",
            "Transaction verification failed",
        ));
    }

    let response = client.send_tx_rpc(&hex).await?;
    SubmittedTx::from_broadcast(&hex, json, dynamic_fee, next_nonce, response)
}

pub(crate) async fn list_all_transactions(client: &ChainClient) -> Result<Vec<Tx>, EldError> {
    let api = AbciHttpApi::new(client.config.get_node_url()?)?;
    let abci_info = api.get_latest_abci_info().await?;
    let mut txs = Vec::new();
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
                txs.push(tx);
            }
        }
        current_block -= 1;
    }
    Ok(txs)
}

pub(crate) async fn list_transactions(
    client: &ChainClient,
    addr: String,
) -> Result<Vec<tendermint_rpc::endpoint::tx::Response>, EldError> {
    let api = AbciHttpApi::new(client.config.get_node_url()?)?;
    api.get_transactions_for_account(addr).await
}

pub(crate) async fn stake(
    client: &ChainClient,
    wallet_name: String,
    amount: u128,
) -> Result<SubmittedTx, EldError> {
    let wallet = super::util::require_wallet(client, &wallet_name).await?;
    let next_nonce = super::util::require_nonce(
        client
            .get_next_nonce_for_account(wallet.address.hex_with_prefix())
            .await,
    )?;

    let stake_tx = StakeTx::new(
        wallet.address,
        amount.into(),
        Some(hex::encode(wallet.public_key)),
    )?;

    let mut tx = Tx::new(next_nonce, Payload::new(stake_tx), wallet.verifying_key());

    let dynamic_fee =
        eld_common::fee::calculate_dynamic_fee(&tx, &client.fee_config).map_err(|e| {
            ErrorBuilder::transaction_error(
                tx_type::TX_TYPE_STAKE,
                &format!("Failed to calculate dynamic fee: {e}"),
            )
        })?;
    tx.fee = dynamic_fee.into();

    wallet.sign(&mut tx, &client.config.chain_id)?;
    let json = serde_json::to_string(&tx).map_err(|e| {
        ErrorBuilder::transaction_error(
            tx_type::TX_TYPE_STAKE,
            &format!("Failed to serialize transaction: {e}"),
        )
    })?;
    let hex = hex::encode(&json);

    if !wallet.verify(&tx, &client.config.chain_id)? {
        return Err(ErrorBuilder::transaction_error(
            "Stake",
            "Transaction verification failed",
        ));
    }

    let response = client.send_tx_rpc(&hex).await?;
    SubmittedTx::from_broadcast(&hex, json, dynamic_fee, next_nonce, response)
}

pub(crate) async fn unstake(
    client: &ChainClient,
    wallet_name: String,
    amount: u128,
) -> Result<SubmittedTx, EldError> {
    let wallet = super::util::require_wallet(client, &wallet_name).await?;
    let next_nonce = super::util::require_nonce(
        client
            .get_next_nonce_for_account(wallet.address.hex_with_prefix())
            .await,
    )?;

    let unstake_tx = UnstakeTx::new(wallet.address, amount.into())?;

    let mut tx = Tx::new(next_nonce, Payload::new(unstake_tx), wallet.verifying_key());

    let dynamic_fee =
        eld_common::fee::calculate_dynamic_fee(&tx, &client.fee_config).map_err(|e| {
            ErrorBuilder::transaction_error(
                "Unstake",
                &format!("Failed to calculate dynamic fee: {e}"),
            )
        })?;
    tx.fee = dynamic_fee.into();

    wallet.sign(&mut tx, &client.config.chain_id)?;
    let json = serde_json::to_string(&tx).map_err(|e| {
        ErrorBuilder::transaction_error("Unstake", &format!("Failed to serialize transaction: {e}"))
    })?;
    let hex = hex::encode(&json);

    if !wallet.verify(&tx, &client.config.chain_id)? {
        return Err(ErrorBuilder::transaction_error(
            "Unstake",
            "Transaction verification failed",
        ));
    }

    let response = client.send_tx_rpc(&hex).await?;
    SubmittedTx::from_broadcast(&hex, json, dynamic_fee, next_nonce, response)
}
