//! Validator set and epoch information display.

use crate::abci_api::AbciHttpApi;
use crate::client::ChainClient;
use crate::logging::SanitizedLog;
use tracing::{error, info};

pub(crate) async fn view_active_validators(client: &ChainClient) {
    let api = AbciHttpApi::new(client.config.get_node_url().to_owned());
    info!(
        "Fetching active validators from {}...",
        client.config.get_node_url()
    );

    match api.get_active_validators().await {
        Ok(Some(active_validators)) => {
            if active_validators.validators.is_empty() {
                info!("No active validators found in the current epoch");
                return;
            }

            info!("Current Epoch: {}", active_validators.current_epoch);
            info!("Total Stake: {}", active_validators.total_stake);
            info!(
                "\nActive Validators (sorted by voting power): {}",
                active_validators.validators.len()
            );

            for (i, validator) in active_validators.validators.iter().enumerate() {
                info!("\nValidator #{}", i + 1);
                info!(
                    "  Address: {}",
                    SanitizedLog::as_address(&validator.address)
                );
                info!("  Stake (Voting Power): {}", validator.stake);

                match api
                    .get_account_by_address(&validator.address.to_string())
                    .await
                {
                    Ok(Some(account)) => {
                        info!("  Liquid Balance: {}", account.balance());
                    }
                    Ok(None) => {
                        info!("  Liquid Balance: Account not found");
                    }
                    Err(e) => {
                        error!("  Liquid Balance: Error fetching account: {}", e);
                    }
                }

                info!(
                    "  Public Key: {}",
                    SanitizedLog::as_public_key(hex::encode(&validator.public_key))
                );
            }
        }
        Ok(None) => {
            info!("No active validators information available");
        }
        Err(e) => {
            error!("Error fetching active validators: {}", e.to_string());
        }
    }
}

pub(crate) async fn view_epoch_info(client: &ChainClient) {
    let api = AbciHttpApi::new(client.config.get_node_url().to_owned());
    let epoch_info = api.get_epoch_info().await.unwrap().unwrap();

    info!("Current Epoch: {}", epoch_info.current_epoch);
    info!("Current Block: {}", epoch_info.current_block);
    info!("Blocks Per Epoch: {}", epoch_info.blocks_per_epoch);
    info!("Validators Per Epoch: {}", epoch_info.validators_per_epoch);
    info!(
        "Blocks Until Next Epoch: {}",
        epoch_info.blocks_until_next_epoch
    );
}

pub(crate) async fn view_epoch(client: &ChainClient) {
    let api = AbciHttpApi::new(client.config.get_node_url().to_owned());

    let epoch_info_future = api.get_epoch_info();
    let active_validators_future = api.get_active_validators();

    let (epoch_info_result, active_validators_result) =
        tokio::join!(epoch_info_future, active_validators_future);

    let epoch_info = epoch_info_result.unwrap().unwrap();
    let active_validators = active_validators_result.unwrap().unwrap();

    info!("╔══════════════════════════════════════════╗");
    info!("║             EPOCH INFORMATION            ║");
    info!("╚══════════════════════════════════════════╝");
    info!("  Current Epoch: {}", epoch_info.current_epoch);
    info!("  Current Block: {}", epoch_info.current_block);
    info!(
        "  Blocks Until Next Epoch: {}",
        epoch_info.blocks_until_next_epoch
    );
    info!("");

    info!("╔══════════════════════════════════════════╗");
    info!(
        "║      ACTIVE VALIDATORS (EPOCH {})      ║",
        epoch_info.current_epoch
    );
    info!("╚══════════════════════════════════════════╝");
    info!("  Total Stake: {}", active_validators.total_stake);
    info!(
        "  Validators per Epoch: {}",
        epoch_info.validators_per_epoch
    );
    info!("");

    for (i, validator) in active_validators.validators.iter().enumerate() {
        let address_display = validator.address.to_string();
        info!(
            "  Validator #{} - {}",
            i + 1,
            &address_display[0..address_display.len().min(12)]
        );
        info!("  ├─ Address: {}", address_display);
        let percentage = validator
            .stake
            .ratio(active_validators.total_stake)
            .map(|r| r * 100.0)
            .unwrap_or(0.0);
        info!(
            "  ├─ Stake: {} ({:.2}% of total)",
            validator.stake, percentage
        );
        let pk_hex = hex::encode(&validator.public_key);
        info!("  └─ Public Key: {}...", &pk_hex[0..pk_hex.len().min(16)]);
        info!("");
    }

    info!("╔══════════════════════════════════════════╗");
    info!("║               EPOCH TIMER                ║");
    info!("╚══════════════════════════════════════════╝");
    info!(
        "  Next validator rotation in {} blocks",
        epoch_info.blocks_until_next_epoch
    );

    let progress = ((epoch_info.blocks_per_epoch - epoch_info.blocks_until_next_epoch) as f64
        / epoch_info.blocks_per_epoch as f64)
        * 100.0;

    let bar_width = 50;
    let filled_width = (progress / 100.0 * bar_width as f64) as usize;

    info!("  [");
    for i in 0..bar_width {
        if i < filled_width {
            info!("█");
        } else {
            info!("░");
        }
    }
    info!("] {:.1}%", progress);
}
