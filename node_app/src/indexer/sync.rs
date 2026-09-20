//! Tendermint RPC-backed transaction indexer sync loop.

use crate::indexer::{TransactionIndexer, TransactionStatus};
use eld_client::api::abci::{
    decode_eld_tx_from_block_tx_bytes, tm_events_to_abci_events, tm_tx_result_is_success,
    wire_bytes_to_tx_hash, AbciHttpApi,
};
use eld_common::error::EldError;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

const RETRY_SLEEP: Duration = Duration::from_secs(1);

/// Grep-friendly prefix for all indexer sync thread logs.
const IX: &str = "INDEXER-LOG:";

/// Spawn a dedicated thread that syncs indexed transactions from Tendermint RPC.
pub fn spawn_indexer_sync(indexer: Arc<TransactionIndexer>, tendermint_rpc_url: String) {
    std::thread::spawn(move || {
        info!(
            tendermint_rpc = %tendermint_rpc_url,
            "{} transaction indexer sync thread starting",
            IX
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("INDEXER-LOG: failed to build indexer Tokio runtime");
        rt.block_on(run_indexer_sync_loop(indexer, tendermint_rpc_url));
    });
}

async fn run_indexer_sync_loop(indexer: Arc<TransactionIndexer>, tendermint_rpc_url: String) {
    let api = match AbciHttpApi::new(tendermint_rpc_url) {
        Ok(api) => api,
        Err(e) => {
            error!(
                error = %e,
                "{} failed to create Tendermint RPC client; indexer sync will not start",
                IX
            );
            return;
        }
    };

    if let Err(e) = startup_repair_cursor_block(&indexer, &api).await {
        error!(error = %e, "{} indexer startup repair failed", IX);
    }

    match indexer.get_indexer_cursor() {
        Ok(None) => info!(
            "{} indexer sync: no persisted cursor; will begin at block height 1 (Tendermint has no block 0)",
            IX
        ),
        Ok(Some(h)) => info!(
            last_synced_height = h,
            next_target_height = h.saturating_add(1),
            "{} indexer sync: resuming after stored cursor",
            IX
        ),
        Err(e) => error!(error = %e, "{} indexer sync: could not read cursor after startup repair", IX),
    }

    loop {
        // `next_height` is always derived from the persisted cursor (`last_synced + 1`, or 1 if
        // none — Tendermint has no block 0). It is NOT advanced here — only `set_indexer_cursor`
        // updates RocksDB. So a failed
        // `get_block` (e.g. head not committed yet) retries the same height after sleep.
        let next_height = match indexer.get_indexer_cursor() {
            Ok(Some(h)) => h.saturating_add(1),
            // Tendermint RPC rejects height 0 ("height must be greater than 0").
            Ok(None) => 1,
            Err(e) => {
                error!("{} failed to read indexer cursor: {}", IX, e);
                tokio::time::sleep(RETRY_SLEEP).await;
                continue;
            }
        };

        let block_resp = match api.get_block(next_height).await {
            Ok(b) => b,
            Err(e) => {
                debug!(
                    height = next_height,
                    error = %e,
                    "{} get_block failed (often waiting for next commit); will retry after sleep",
                    IX
                );
                tokio::time::sleep(RETRY_SLEEP).await;
                continue;
            }
        };

        let tx_count = block_resp.block.data.len();
        info!(
            height = next_height,
            tx_count, "{} indexer fetched block from Tendermint", IX
        );

        let mut block_fully_written = true;
        for (block_index, block_tx_wire) in block_resp.block.data.iter().enumerate() {
            let block_index = block_index as u32;
            if let Err(e) = index_tx_from_block_and_tm_tx(
                &indexer,
                &api,
                block_tx_wire,
                next_height,
                block_index,
            )
            .await
            {
                error!(
                    height = next_height,
                    block_index,
                    error = %e,
                    "{} failed to index tx",
                    IX
                );
                block_fully_written = false;
            }
        }

        if !block_fully_written {
            warn!(
                height = next_height,
                "{} indexer block not fully written; retrying same height after sleep", IX
            );
            tokio::time::sleep(RETRY_SLEEP).await;
            continue;
        }

        match indexer.set_indexer_cursor(next_height) {
            Ok(()) => info!(
                height = next_height,
                tx_count, "{} indexer advanced cursor after storing block", IX
            ),
            Err(e) => {
                error!(
                    height = next_height,
                    error = %e,
                    "{} failed to persist indexer cursor; will retry same block after sleep",
                    IX
                );
                tokio::time::sleep(RETRY_SLEEP).await;
            }
        }
    }
}

async fn startup_repair_cursor_block(
    indexer: &TransactionIndexer,
    api: &AbciHttpApi,
) -> Result<(), EldError> {
    let Some(cursor_height) = indexer.get_indexer_cursor()? else {
        return Ok(());
    };

    if cursor_height == 0 {
        warn!(
            "{} indexer startup repair: skipping invalid cursor height 0 (no Tendermint block 0)",
            IX
        );
        return Ok(());
    }

    info!(
        cursor_height,
        "{} indexer startup repair: checking last synced block for gaps", IX
    );

    let block_resp = api.get_block(cursor_height).await?;
    for (block_index, block_tx_wire) in block_resp.block.data.iter().enumerate() {
        let block_index = block_index as u32;
        if indexer.block_position_indexed(cursor_height, block_index)? {
            continue;
        }
        warn!(
            "{} missing indexed tx at block {} index {}; fetching from Tendermint /tx",
            IX, cursor_height, block_index
        );
        if let Err(e) =
            index_tx_from_block_and_tm_tx(indexer, api, block_tx_wire, cursor_height, block_index)
                .await
        {
            error!(
                "{} startup repair failed for block {} index {}: {}",
                IX, cursor_height, block_index, e
            );
        }
    }
    info!(
        cursor_height,
        tx_slots = block_resp.block.data.len(),
        "{} indexer startup repair pass complete",
        IX
    );
    Ok(())
}

/// Decode the block tx list entry to locate the tx on Tendermint, then index from the `/tx` RPC response.
async fn index_tx_from_block_and_tm_tx(
    indexer: &TransactionIndexer,
    api: &AbciHttpApi,
    block_tx_wire: &[u8],
    expected_height: u64,
    expected_index: u32,
) -> Result<(), EldError> {
    let hash = wire_bytes_to_tx_hash(block_tx_wire);
    let tx_resp = api.get_tx_by_hash(hash).await?;

    let resp_height = tx_resp.height.value();
    let resp_index = tx_resp.index;
    if resp_height != expected_height || resp_index != expected_index {
        warn!(
            "{} TM /tx height/index ({}, {}) != block list ({}, {}); using block list position",
            IX, resp_height, resp_index, expected_height, expected_index
        );
    }

    // Index from the individual /tx response (authoritative wire + execution result).
    let eld_tx = decode_eld_tx_from_block_tx_bytes(&tx_resp.tx)?;
    let status = if tm_tx_result_is_success(tx_resp.tx_result.code) {
        TransactionStatus::Success
    } else {
        TransactionStatus::Failed
    };

    indexer.index_transaction(&eld_tx, expected_height, expected_index, status)?;

    let tx_id = indexer.calculate_tx_id(&eld_tx);
    debug!(
        height = expected_height,
        block_index = expected_index,
        tx_id = %tx_id,
        ?status,
        "{} indexer stored transaction and events",
        IX
    );
    for (event_index, event) in tm_events_to_abci_events(&tx_resp.tx_result.events)
        .into_iter()
        .enumerate()
    {
        if let Err(e) = indexer.index_event(
            &tx_id,
            event_index as u32,
            &event,
            expected_height,
            expected_index,
        ) {
            error!(
                "{} failed to index event {} for tx {} at block {} index {}: {}",
                IX, event_index, tx_id, expected_height, expected_index, e
            );
        }
    }
    Ok(())
}
