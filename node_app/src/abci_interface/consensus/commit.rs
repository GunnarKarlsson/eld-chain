use super::connection::ConsensusConnection;
use crate::app_state::app_state_snapshot::AppStateSnapshot;
use crate::app_state::AppStateTip;
use crate::errors::handle_fatal_eld_error;
use crate::storage::traits::ConsensusConnectionStorage;
use abci::types::*;
use eld_common::cado::{CADOMetadata, CadoBody, CadoPath, CadoPathKey, CadoType};
use eld_common::constants::cado::LATEST;
use eld_common::constants::pinboard::MAX_CHUNK_SIZE;
use eld_common::constants::protocol::BLOCKS_PER_EPOCH;
use eld_common::error::EldError;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tracing::{debug, error, info, warn};

impl<S> ConsensusConnection<S>
where
    S: ConsensusConnectionStorage,
{
    pub(crate) async fn commit_inner(&self, _commit_request: RequestCommit) -> ResponseCommit {
        // Calculate time since last commit
        let mut last_time = self.last_commit_time.lock().unwrap();
        let elapsed = last_time.elapsed().as_secs_f64();
        info!("COMMIT_METRICS: Time since last commit: {:.3}s", elapsed);
        *last_time = Instant::now();

        let current_state_lock = match self.current_state.lock() {
            Ok(lock) => lock,
            Err(e) => {
                handle_fatal_eld_error(e.into());
            }
        };

        let mut current_state = match current_state_lock.as_ref() {
            Some(state) => state.clone(),
            None => {
                handle_fatal_eld_error(EldError::InitializationError {
                    component: "current state".to_string(),
                    details: "state is None in commit".to_string(),
                });
            }
        };

        // Check if we should signal readiness (block_height > 0 means first consensus commit completed)
        if current_state.envelope.block_height > 0 {
            let mut ready_tx_guard = match self.ready_tx.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    return ResponseCommit {
                        data: vec![],
                        retain_height: 0,
                    }
                }
            };
            if let Some(tx) = ready_tx_guard.take() {
                drop(ready_tx_guard); // Release lock before sending
                let _ = tx.send(());
                info!(
                    block_height = current_state.envelope.block_height,
                    "Node is READY: First consensus commit completed, block height > 0"
                );
            }
        }

        // Update CADO type counts
        let mut type_counts = self.cado_type_counts.lock().unwrap();
        for cado in current_state.envelope.cado_cache.values() {
            let cado_type = match cado {
                CadoBody::Mutable(cado_mut) => cado_mut.metadata().type_().to_string(),
                CadoBody::Immutable(cado) => cado.metadata().type_().to_string(),
            };
            *type_counts.entry(cado_type).or_insert(0) += 1;
        }

        // Validate app_hash is not empty before proceeding
        if current_state.app_hash().is_empty() {
            handle_fatal_eld_error(EldError::ValidationError {
                field: "app_hash".to_string(),
                value: "empty".to_string(),
                details: "App hash cannot be empty during commit".to_string(),
            });
        }

        let storage = &self.storage;

        // Check if we need to select new validators for the next epoch
        let new_block_height = current_state.envelope.block_height + 1;
        let new_epoch = new_block_height / BLOCKS_PER_EPOCH;
        let current_epoch = current_state.envelope.current_epoch;

        if new_epoch > current_epoch {
            // Select new validators for the next epoch
            self.select_validators_for_epoch(&mut current_state);
        }

        // Begin a transaction for atomic storage writes.
        // Commit tx order: deletes → puts → pinboard → tip/snapshot.
        let tx = storage.begin_transaction();

        let pending_deletes: Vec<_> = current_state.envelope.cado_cache_to_delete.to_vec();

        for cado_to_delete in &pending_deletes {
            if let Err(e) = storage.system_delete_cado_with_tx(
                cado_to_delete.cado_path.clone(),
                &cado_to_delete.owner,
                &tx,
            ) {
                handle_fatal_eld_error(e);
            }
        }

        for cado_to_delete in &pending_deletes {
            let path_bytes = cado_to_delete.cado_path.as_str().as_bytes();
            current_state
                .envelope
                .cado_cache
                .remove(cado_to_delete.cado_path.as_str());
            current_state
                .envelope
                .committed_cado_cache
                .remove(path_bytes);
            current_state.envelope.state_trie.remove(path_bytes);
        }
        current_state.envelope.cado_cache_to_delete.clear();

        // Write all CADO cache entries to storage explicitly
        let mut failed_cados = Vec::new();
        let _cado_cache_size = current_state.envelope.cado_cache.len();

        for (key, cado) in &current_state.envelope.cado_cache {
            if eld_common::cado::is_infrastructure_cado_path(key) {
                warn!(
                    cado_path = %key,
                    "Skipping infrastructure CADO during commit storage write"
                );
                continue;
            }
            let cado_path = match CadoPath::parse(key) {
                Ok(path) => path,
                Err(e) => {
                    warn!(
                        cado_path = %key,
                        error = %e,
                        "Failed to create CadoPath for cache item, will remove from cache"
                    );
                    failed_cados.push(key.clone());
                    continue;
                }
            };

            if let Err(e) = storage.put_cado_type_with_tx(cado_path.clone(), cado.clone(), &tx) {
                warn!(
                    cado_path = %cado_path,
                    error = %e,
                    "Failed to write CADO to storage, will remove from cache"
                );
                failed_cados.push(key.clone());
                continue;
            }
        }

        // Remove failed CADOs from cache
        if !failed_cados.is_empty() {
            warn!(
                failed_count = failed_cados.len(),
                "Found failed CADOs during commit that will be removed from cache"
            );
            for key in failed_cados {
                current_state.envelope.cado_cache.remove(&key);
            }
        }

        // --------------------------------------------------------------------
        // Pinboard (PostMessage) staged writes
        // --------------------------------------------------------------------
        if !current_state.envelope.pinboard_meta_cache.is_empty()
            || !current_state.envelope.pinboard_idx_wallet_add.is_empty()
            || !current_state.envelope.pinboard_idx_tag_add.is_empty()
            || !current_state.envelope.pinboard_idx_expiry_add.is_empty()
            || !current_state.envelope.pinboard_idx_commit_add.is_empty()
            || !current_state.envelope.pinboard_refcount_deltas.is_empty()
        {
            info!(
                pinboard_meta = current_state.envelope.pinboard_meta_cache.len(),
                pinboard_wallet_idx = current_state.envelope.pinboard_idx_wallet_add.len(),
                pinboard_tag_idx = current_state.envelope.pinboard_idx_tag_add.len(),
                pinboard_expiry_idx = current_state.envelope.pinboard_idx_expiry_add.len(),
                pinboard_commit_idx = current_state.envelope.pinboard_idx_commit_add.len(),
                pinboard_refcount_keys = current_state.envelope.pinboard_refcount_deltas.len(),
                "COMMIT_METRICS: pinboard staged writes"
            );
        }

        let mut pinboard_capacity_blobs: HashMap<String, Vec<u8>> = HashMap::new();
        let mut pinboard_temp_blob_keys_to_delete: HashSet<String> = HashSet::new();

        // Pre-validate temp blobs before any pinboard writes to keep commit all-or-nothing.
        for (message_id, meta) in current_state.envelope.pinboard_meta_cache.iter() {
            match storage.get_pinboard_temp_blob(&meta.content_key) {
                Ok(Some(temp_bytes)) => {
                    pinboard_capacity_blobs.insert(meta.content_key.clone(), temp_bytes.clone());
                    pinboard_temp_blob_keys_to_delete.insert(meta.content_key.clone());
                }
                Ok(None) => {
                    handle_fatal_eld_error(EldError::StorageError {
                        operation: "prevalidate_pinboard_temp_blob".to_string(),
                        details: format!(
                            "Missing pinboard temp blob for message_id={} content_key={}",
                            message_id, meta.content_key
                        ),
                    });
                }
                Err(e) => {
                    handle_fatal_eld_error(EldError::StorageError {
                        operation: "prevalidate_pinboard_temp_blob".to_string(),
                        details: format!(
                            "Failed reading pinboard temp blob for message_id={} content_key={}: {}",
                            message_id, meta.content_key, e
                        ),
                    });
                }
            }
        }

        for (message_id, meta) in current_state.envelope.pinboard_meta_cache.iter() {
            if let Err(e) = storage.put_pinboard_metadata_with_tx(message_id, meta, &tx) {
                handle_fatal_eld_error(e);
            }
        }

        for (wallet, committed_height, message_id) in
            current_state.envelope.pinboard_idx_wallet_add.iter()
        {
            if let Err(e) = storage.put_pinboard_wallet_index_with_tx(
                wallet,
                *committed_height,
                message_id,
                &tx,
            ) {
                handle_fatal_eld_error(e);
            }
        }

        for (tag, committed_height, message_id) in
            current_state.envelope.pinboard_idx_tag_add.iter()
        {
            if let Err(e) =
                storage.put_pinboard_tag_index_with_tx(tag, *committed_height, message_id, &tx)
            {
                handle_fatal_eld_error(e);
            }
        }

        for (expires_height, message_id) in current_state.envelope.pinboard_idx_expiry_add.iter() {
            if let Err(e) =
                storage.put_pinboard_expiry_index_with_tx(*expires_height, message_id, &tx)
            {
                handle_fatal_eld_error(e);
            }
        }

        for (committed_height, message_id) in current_state.envelope.pinboard_idx_commit_add.iter()
        {
            if let Err(e) =
                storage.put_pinboard_commit_index_with_tx(*committed_height, message_id, &tx)
            {
                handle_fatal_eld_error(e);
            }
        }

        for (content_key, delta) in current_state.envelope.pinboard_refcount_deltas.iter() {
            if let Err(e) = storage.update_pinboard_refcount_with_tx(content_key, *delta, &tx) {
                handle_fatal_eld_error(e);
            }
        }

        for content_key in pinboard_temp_blob_keys_to_delete {
            if let Err(e) = storage.delete_pinboard_temp_blob_with_tx(&content_key, &tx) {
                warn!(
                    content_key = %content_key,
                    error = %e,
                    "COMMIT_WARN: failed to delete pinboard temp blob (non-fatal)"
                );
            }
        }

        current_state.envelope.pinboard_meta_cache.clear();
        current_state.envelope.pinboard_idx_wallet_add.clear();
        current_state.envelope.pinboard_idx_tag_add.clear();
        current_state.envelope.pinboard_idx_expiry_add.clear();
        current_state.envelope.pinboard_idx_commit_add.clear();
        current_state.envelope.pinboard_refcount_deltas.clear();

        for challenge_id in current_state.envelope.verified_proof_rewarded_cache.iter() {
            if let Err(e) = storage.put_verified_proof_challenge_rewarded_with_tx(challenge_id, &tx)
            {
                handle_fatal_eld_error(e);
            }
        }
        current_state.envelope.verified_proof_rewarded_cache.clear();

        // --------------------------------------------------------------------
        // Namespace registry (AddNamespace) staged writes
        // --------------------------------------------------------------------
        if !current_state.envelope.namespace_registry_cache.is_empty() {
            info!(
                namespace_count = current_state.envelope.namespace_registry_cache.len(),
                "COMMIT_METRICS: namespace registry staged writes"
            );

            let namespace_entries: Vec<_> = current_state
                .envelope
                .namespace_registry_cache
                .iter()
                .map(|(slug, record)| (slug.clone(), record.clone()))
                .collect();

            for (namespace_slug, record) in namespace_entries {
                let path = match eld_common::namespace::slug_to_namespace_cadopath(&namespace_slug)
                {
                    Ok(path) => path,
                    Err(e) => {
                        error!(
                            namespace_slug = %namespace_slug,
                            error = %e,
                            "COMMIT_ERROR: invalid namespace registry path (skipping)"
                        );
                        continue;
                    }
                };

                let payload = match record.serialize_bin() {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        error!(
                            namespace_slug = %namespace_slug,
                            error = %e,
                            "COMMIT_ERROR: namespace record serialize failed (skipping)"
                        );
                        continue;
                    }
                };

                let cado = CadoBody::immutable(
                    payload,
                    CADOMetadata::new(CadoType::Namespace, &namespace_slug),
                );
                let path_str = path.as_str();
                let path_bytes = path_str.as_bytes();

                if let Err(e) = storage.put_cado_type_with_tx(path.clone(), cado.clone(), &tx) {
                    error!(
                        registry_path = %path_str,
                        error = %e,
                        "COMMIT_ERROR: namespace registry CADO write failed (skipping)"
                    );
                    continue;
                }

                current_state
                    .envelope
                    .committed_cado_cache
                    .insert(path_bytes, cado.clone());
                current_state
                    .envelope
                    .state_trie
                    .insert(path_bytes, &cado.content_hash());
                current_state
                    .envelope
                    .namespace_registry_index
                    .insert(namespace_slug, record);
            }

            current_state.envelope.namespace_registry_cache.clear();
        }

        // Update eld trie with CADO updates
        let updates: Vec<(Vec<u8>, CadoBody)> = current_state
            .envelope
            .cado_cache
            .iter()
            .map(|(key, value)| (key.as_bytes().to_vec(), value.clone()))
            .collect();

        for (key_bytes, value) in &updates {
            let Ok(key_str) = std::str::from_utf8(key_bytes) else {
                continue;
            };
            if eld_common::cado::is_infrastructure_cado_path(key_str) {
                warn!(
                    path = %key_str,
                    "Skipping infrastructure CADO during commit merge into committed_cado_cache"
                );
                continue;
            }
            current_state
                .envelope
                .committed_cado_cache
                .insert(key_bytes, value.clone());
        }
        let epoch_index_paths: Vec<String> =
            current_state.envelope.cado_cache.keys().cloned().collect();
        for key_str in epoch_index_paths {
            current_state
                .envelope
                .insert_epoch_records_index_from_path(&key_str);
        }

        // Merkle Patricia Trie (incremental root for AppStateTip)
        for (key_str, cado) in &current_state.envelope.cado_cache {
            if eld_common::cado::is_infrastructure_cado_path(key_str) {
                continue;
            }
            let value_hash = cado.content_hash();
            current_state
                .envelope
                .state_trie
                .insert(key_str.as_bytes(), &value_hash);
        }

        let trie_root_hash = current_state.envelope.state_trie.root_hash();

        let app_state_tip = AppStateTip {
            block_height: current_state.envelope.block_height,
            cado_root_hash: trie_root_hash, // Store trie hash in cado_root_hash field
            app_hash: match current_state.app_hash().bytes() {
                Ok(bytes) => bytes,
                Err(e) => handle_fatal_eld_error(e),
            },
        };
        let app_state_tip_serialized = match bincode::serialize(&app_state_tip) {
            Ok(serialized) => serialized,
            Err(e) => {
                handle_fatal_eld_error(EldError::StorageError {
                    operation: "serialize_app_state_tip".to_string(),
                    details: e.to_string(),
                });
            }
        };
        let app_state_tip_meta = CADOMetadata::new(CadoType::AppStateTip, "system");
        let app_state_tip_cado =
            CadoBody::immutable(app_state_tip_serialized.clone(), app_state_tip_meta);
        let app_state_tip_hash = Sha256::digest(&app_state_tip_serialized);

        // Store at hash-based path
        let app_state_tip_id = format!("0x{}", hex::encode(app_state_tip_hash));
        let app_state_tip_path =
            match CadoPath::new(CadoType::AppStateTip, CadoPathKey::Name(&app_state_tip_id)) {
                Ok(path) => path,
                Err(e) => {
                    handle_fatal_eld_error(e);
                }
            };

        // Store at fixed "latest" path
        let latest_app_state_tip_path =
            match CadoPath::new(CadoType::AppStateTip, CadoPathKey::Name(LATEST)) {
                Ok(path) => path,
                Err(e) => {
                    handle_fatal_eld_error(e);
                }
            };

        // Store at both paths
        if let Err(e) =
            storage.put_cado_type_with_tx(app_state_tip_path, app_state_tip_cado.clone(), &tx)
        {
            handle_fatal_eld_error(e);
        }
        if let Err(e) =
            storage.put_cado_type_with_tx(latest_app_state_tip_path, app_state_tip_cado, &tx)
        {
            handle_fatal_eld_error(e);
        }

        // Persist trie snapshot once per epoch (at epoch-start block heights).
        let should_create_epoch_snapshot =
            current_state.envelope.block_height % BLOCKS_PER_EPOCH == 0;
        if should_create_epoch_snapshot {
            let snapshot_start = std::time::Instant::now();

            let app_state_snapshot = match AppStateSnapshot::new(&current_state) {
                Ok(snapshot) => snapshot,
                Err(e) => handle_fatal_eld_error(e),
            };
            let app_state_snapshot_cado = match app_state_snapshot.to_cado() {
                Ok(cado) => cado,
                Err(e) => handle_fatal_eld_error(e),
            };
            let app_state_snapshot_path = match AppStateSnapshot::latest_path() {
                Ok(path) => path,
                Err(e) => handle_fatal_eld_error(e),
            };
            if let Err(e) = storage.put_cado_type_with_tx(
                app_state_snapshot_path.clone(),
                app_state_snapshot_cado,
                &tx,
            ) {
                error!(
                    "COMMIT_ERROR: Failed to store app state snapshot at {}: {}",
                    app_state_snapshot_path.as_str(),
                    e
                );
                handle_fatal_eld_error(e);
            }

            info!(
                "COMMIT_METRICS: app_state_snapshot_persist_ms={:.3} block_height={}",
                snapshot_start.elapsed().as_secs_f64() * 1000.0,
                current_state.envelope.block_height,
            );
        }

        // Announce pinboard blobs that were staged in this block (before clearing cache)
        self.p2p_sync_coordinator
            .find_pinboard_and_broadcast_announce(&current_state.envelope.pinboard_meta_cache);

        // Clear caches
        current_state.envelope.cado_cache.clear();

        // Commit the transaction to persist all changes
        if let Err(e) = tx.commit() {
            handle_fatal_eld_error(EldError::StorageError {
                operation: "commit_transaction".to_string(),
                details: e.to_string(),
            });
        }

        if should_create_epoch_snapshot {
            let snapshot_manager = self.snapshot_manager.clone();
            let snapshot_height = current_state.envelope.block_height;
            tokio::spawn(async move {
                if let Err(e) = snapshot_manager
                    .create_snapshot_from_latest_state(snapshot_height)
                    .await
                {
                    error!(
                        block_height = snapshot_height,
                        error = %e,
                        "Failed to create persisted-state snapshot"
                    );
                }
            });
        }

        let blobs = pinboard_capacity_blobs;
        if !blobs.is_empty() {
            info!(
                count = blobs.len(),
                "Commit: scheduling write of pinboard content to capacity slots after RocksDB commit"
            );
        }
        let cm = self.capacity_manager.clone();
        tokio::spawn(async move {
            for (content_key, bytes) in blobs {
                match cm.get_content_from_slots(&content_key).await {
                    Ok(_) => {
                        debug!(
                            content_key = %content_key,
                            "Pinboard capacity mirror: content already in slots, skipping store_content_chunks"
                        );
                    }
                    Err(_) => {
                        let chunks: Vec<Vec<u8>> =
                            bytes.chunks(MAX_CHUNK_SIZE).map(|c| c.to_vec()).collect();
                        match cm.store_content_chunks(content_key.clone(), chunks).await {
                            Ok(_) => {
                                info!(
                                    content_key = %content_key,
                                    byte_len = bytes.len(),
                                    "PINBOARD_CAPACITY_WRITE_OK: pinboard content written into registered capacity slots after RocksDB commit"
                                );
                            }
                            Err(e) => {
                                error!(
                                    content_key = %content_key,
                                    error = %e,
                                    "Pinboard capacity write failed"
                                );
                            }
                        }
                    }
                }
            }
        });

        let mut committed_state = match self.committed_state.lock() {
            Ok(lock) => lock,
            Err(e) => {
                handle_fatal_eld_error(e.into());
            }
        };
        *committed_state = current_state.clone();
        self.chain_tip.update_from_app_state(&committed_state);

        ResponseCommit {
            data: committed_state.app_hash().to_vec(),
            retain_height: 0,
        }
    }
}
