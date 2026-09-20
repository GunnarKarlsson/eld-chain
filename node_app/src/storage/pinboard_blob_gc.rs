use crate::abci_interface::chain_tip::ChainTip;
use crate::api::pagination::{PaginationParams, PrefixQueryOptions};
use crate::capacity::capacity_manager::CapacityManager;
use crate::storage::rocksdb::RocksDBStorage;
use crate::storage::traits::PinboardGlobalFeedOrder;
use eld_common::capacity::slot_allocator::CONTENT_ID_NOT_IN_SLOT_MAP;
use eld_common::error::EldError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info};

const PINBOARD_GC_TICK_SECS: u64 = 5;
const PINBOARD_GC_MAX_SCAN_PER_TICK: usize = 500;
const PINBOARD_GC_MAX_DELETES_PER_TICK: usize = 200;
const PINBOARD_GC_PAGE_SIZE: usize = 200;

pub fn start_pinboard_blob_gc_worker(
    storage: Arc<RocksDBStorage>,
    chain_tip: Arc<ChainTip>,
    capacity_manager: Arc<CapacityManager>,
) {
    tokio::spawn(async move {
        info!(
            tick_secs = PINBOARD_GC_TICK_SECS,
            max_scan = PINBOARD_GC_MAX_SCAN_PER_TICK,
            max_deletes = PINBOARD_GC_MAX_DELETES_PER_TICK,
            "Starting pinboard GC worker (capacity-slot mode)"
        );
        loop {
            if let Err(e) = run_gc_tick(
                storage.as_ref(),
                chain_tip.as_ref(),
                capacity_manager.as_ref(),
            )
            .await
            {
                error!(error = %e, "Pinboard GC tick failed");
            }
            sleep(Duration::from_secs(PINBOARD_GC_TICK_SECS)).await;
        }
    });
}

async fn run_gc_tick(
    storage: &RocksDBStorage,
    chain_tip: &ChainTip,
    capacity_manager: &CapacityManager,
) -> Result<(), anyhow::Error> {
    let current_height = chain_tip.committed_height_u64();
    let mut page = 0usize;
    let mut scanned = 0usize;
    let mut content_max_expires: HashMap<String, u64> = HashMap::new();

    while scanned < PINBOARD_GC_MAX_SCAN_PER_TICK {
        let options = PrefixQueryOptions::default().with_pagination(PaginationParams {
            page,
            page_size: PINBOARD_GC_PAGE_SIZE,
            cursor: None,
        });

        let ids = storage
            .get_pinboard_message_ids_global_secure(PinboardGlobalFeedOrder::Desc, options)?;
        if ids.items.is_empty() {
            break;
        }

        for message_id in ids.items {
            if scanned >= PINBOARD_GC_MAX_SCAN_PER_TICK {
                break;
            }
            scanned += 1;
            if let Some(meta) = storage.get_pinboard_metadata(&message_id)? {
                let entry = content_max_expires.entry(meta.content_key).or_insert(0);
                *entry = (*entry).max(meta.expires_height);
            }
        }

        if !ids.has_more {
            break;
        }
        page += 1;
    }

    let mut delete_candidates: Vec<(String, u64)> = content_max_expires
        .into_iter()
        .filter(|(_, max_expires_height)| current_height >= *max_expires_height)
        .collect();
    delete_candidates.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

    let mut deletes = 0usize;
    let mut last_deleted_expires_height = 0u64;
    for (content_key, max_expires_height) in delete_candidates
        .into_iter()
        .take(PINBOARD_GC_MAX_DELETES_PER_TICK)
    {
        match capacity_manager
            .delete_content_from_slots(&content_key)
            .await
        {
            Ok(()) => {
                if let Ok(refs) = storage.find_pinboard_metadata_refs_by_content_key(&content_key) {
                    for (wallet, message_id) in refs {
                        if let Err(e) = storage.append_pinboard_gc_deleted_item(
                            &wallet,
                            &message_id,
                            current_height,
                        ) {
                            error!(
                                error = %e,
                                wallet = %wallet,
                                message_id = %message_id,
                                "Pinboard GC failed appending deleted debug row"
                            );
                        }
                    }
                }
                deletes += 1;
                last_deleted_expires_height = max_expires_height;
            }
            Err(EldError::NotFoundError { resource_type, .. })
                if resource_type == CONTENT_ID_NOT_IN_SLOT_MAP =>
            {
                // Content is already gone from local slots; treat as success for GC progress.
            }
            Err(e) => {
                error!(
                    content_key = %content_key,
                    current_height,
                    max_expires_height,
                    error = %e,
                    "Pinboard GC failed deleting content from capacity slots"
                );
            }
        }
    }

    if let Err(e) = storage.increment_pinboard_gc_scanned_count_by(scanned as u64) {
        error!(error = %e, "Pinboard GC failed updating scanned metric");
    }
    if let Err(e) = storage.increment_pinboard_gc_deleted_count_by(deletes as u64) {
        error!(error = %e, "Pinboard GC failed updating deleted metric");
    }
    if let Err(e) = storage.set_pinboard_gc_last_seen_height(current_height) {
        error!(error = %e, "Pinboard GC failed updating last_seen_height metric");
    }
    if deletes > 0 {
        if let Err(e) =
            storage.set_pinboard_gc_last_deleted_expires_height(last_deleted_expires_height)
        {
            error!(
                error = %e,
                "Pinboard GC failed updating last_deleted_expires_height metric"
            );
        }
    }

    debug!(
        current_height,
        scanned, deletes, "Pinboard GC tick complete (capacity-slot mode)"
    );
    Ok(())
}
