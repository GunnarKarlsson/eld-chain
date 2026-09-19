//! CADO path listing and structured CADO display.

use crate::abci_api::AbciHttpApi;
use crate::account::Account;
use crate::client::ChainClient;
use crate::constants::cado::{
    PATH_PREFIX_ACCOUNT, PATH_PREFIX_ACCOUNT_CONTENT, PATH_PREFIX_APP_STATE_SNAPSHOT,
    PATH_PREFIX_CADO_MAP, PATH_PREFIX_STAKING_ACCOUNT,
};
use crate::error::EldError;
use crate::staking_account::StakingAccount;
use tracing::info;

pub(crate) async fn get_cado(client: &ChainClient, path: String) -> Result<(), EldError> {
    let api = AbciHttpApi::new(client.config.get_node_url()?)?;
    let response = api.get_cado(path.clone()).await?;
    {
        info!("CADO query response:");
        let cado = if let Some(mutable) = response.get("Mutable") {
            Some(("Mutable", mutable))
        } else {
            response
                .get("Immutable")
                .map(|immutable| ("Immutable", immutable))
        };

        if let Some((cado_type, cado)) = cado {
            info!("\nCADO Type: {}", cado_type);

            if let Some(metadata) = cado.get("metadata") {
                info!("\nMetadata:");
                info!(
                    "  Type:\t\t {}",
                    metadata.get("type_").unwrap_or(&serde_json::Value::Null)
                );
                info!(
                    "  Owner:\t {}",
                    metadata.get("owner").unwrap_or(&serde_json::Value::Null)
                );
            }

            if let Some(hash) = cado.get("hash") {
                if let Some(hash_array) = hash.as_array() {
                    let hash_hex = hex::encode(
                        hash_array
                            .iter()
                            .map(|v| v.as_u64().unwrap_or(0) as u8)
                            .collect::<Vec<u8>>(),
                    );
                    info!("\nHash:\t\t 0x{}", hash_hex);
                }
            }

            if cado_type == "Mutable" {
                if let Some(latest_hash) = cado.get("latest_hash") {
                    if let Some(hash_array) = latest_hash.as_array() {
                        let hash_hex = hex::encode(
                            hash_array
                                .iter()
                                .map(|v| v.as_u64().unwrap_or(0) as u8)
                                .collect::<Vec<u8>>(),
                        );
                        info!("Latest Hash:\t 0x{}", hash_hex);
                    }
                }
            }

            if let Some(data) = cado.get("data") {
                if let Some(data_array) = data.as_array() {
                    let data_bytes: Vec<u8> = data_array
                        .iter()
                        .map(|v| v.as_u64().unwrap_or(0) as u8)
                        .collect();

                    if path.starts_with(PATH_PREFIX_STAKING_ACCOUNT) {
                        match StakingAccount::deserialize_bin(&data_bytes) {
                            Ok(account) => {
                                info!("\nAccount Details:");
                                info!("  Originator:\t {}", account.originator);
                                info!("  Balance:\t {}", account.stake_balance);
                            }
                            Err(e) => {
                                info!("\nFailed to parse account data: {}", e);
                                info!("Raw data (hex):");
                                info!("  0x{}", hex::encode(data_bytes));
                            }
                        }
                    } else if path.starts_with(PATH_PREFIX_ACCOUNT)
                        || path.starts_with(PATH_PREFIX_CADO_MAP)
                    {
                        match Account::deserialize_bin(&data_bytes) {
                            Ok(account) => {
                                info!("\nAccount Details:");
                                info!("  Address: {}", account.address());
                                info!("  Balance: {}", account.balance());
                                info!("  Nonce: {}", account.nonce());
                            }
                            Err(e) => {
                                info!("\nFailed to parse account data: {}", e);
                                info!("Raw data (hex):");
                                info!("  0x{}", hex::encode(data_bytes));
                            }
                        }
                    } else if path.starts_with(PATH_PREFIX_ACCOUNT_CONTENT) {
                        match bincode::deserialize::<Vec<String>>(&data_bytes) {
                            Ok(manifest_ids) => {
                                info!("\nAccount Content Summary:");
                                info!(
                                    "  Address:\t {}",
                                    path.split('/').next_back().unwrap_or("unknown")
                                );
                                info!("  Total Content Manifests:\t {}", manifest_ids.len());

                                if manifest_ids.is_empty() {
                                    info!("  No content manifests found");
                                } else {
                                    info!("  Content Manifest IDs:");
                                    for (index, manifest_id) in manifest_ids.iter().enumerate() {
                                        info!("     {}. {}", index + 1, manifest_id);
                                    }
                                }
                            }
                            Err(e) => {
                                info!("\nFailed to parse account content data: {}", e);
                                info!("Raw data (hex):");
                                info!("  0x{}", hex::encode(data_bytes));
                            }
                        }
                    } else if path.starts_with(PATH_PREFIX_APP_STATE_SNAPSHOT) {
                        info!("App State Snapshot Details:\n");

                        info!("* Snapshot Path: \t{}", path);
                        info!(
                            "* Total Size: \t{} bytes ({:.2} KB)",
                            data_bytes.len(),
                            data_bytes.len() as f64 / 1024.0
                        );

                        if data_bytes.len() >= 8 {
                            let block_height_bytes = &data_bytes[0..8];
                            if let Ok(block_height_array) = block_height_bytes.try_into() {
                                let block_height = i64::from_le_bytes(block_height_array);
                                info!("* Block Height: \t{}", block_height);
                            }
                        }

                        if data_bytes.len() >= 40 {
                            let root_hash = &data_bytes[8..40];
                            info!("* Root Hash: \t0x{}", hex::encode(root_hash));
                        }

                        if data_bytes.len() >= 48 {
                            let node_count_bytes = &data_bytes[40..48];
                            if let Ok(node_count_array) = node_count_bytes.try_into() {
                                let node_count = usize::from_le_bytes(node_count_array);
                                info!("* Node Count: \t{}", node_count);
                                if node_count > 0 {
                                    info!(
                                        "* Avg bytes/node: \t{:.2}",
                                        data_bytes.len() as f64 / node_count as f64
                                    );
                                }
                            }
                        }

                        if data_bytes.len() >= 56 {
                            let timestamp_bytes = &data_bytes[48..56];
                            if let Ok(timestamp_array) = timestamp_bytes.try_into() {
                                let timestamp = u64::from_le_bytes(timestamp_array);
                                info!("* Timestamp: \t{} (Unix)", timestamp);
                            }
                        }

                        info!("Trie snapshot found and can be restored on node startup");
                        info!("Use './start_app_with_db_data.sh' to restore from this snapshot");
                    } else if let Ok(str_data) = String::from_utf8(data_bytes.clone()) {
                        info!("Data (as string):");
                        info!("* {}", str_data);
                    } else {
                        info!("Data (as hex):");
                        info!("* 0x{}", hex::encode(data_bytes));
                    }
                }
            }
        } else {
            info!("No CADO found at path: {}", path);
        }
    }
    Ok(())
}

pub(crate) async fn list_cados(
    client: &ChainClient,
    search_string: String,
) -> Result<(), EldError> {
    let api = AbciHttpApi::new(client.config.get_node_url()?)?;
    let paths = api.get_cado_paths(search_string.clone()).await?;
    info!(
        "Found {} CADO paths matching '{}':",
        paths.len(),
        search_string
    );
    for (i, path) in paths.iter().enumerate() {
        info!("{}. {}", i + 1, path);
    }
    Ok(())
}
