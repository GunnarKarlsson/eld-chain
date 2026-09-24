use crate::abci_interface::ConsensusConnection;
use crate::errors::response_deliver_tx_error_validation_failed;
use crate::storage::traits::ConsensusConnectionStorage;
use abci::types::ResponseDeliverTx;
use eld_common::namespace::validate_namespace_upload_authorization;
use eld_common::pinboard::PinboardMessageMetadata;
use eld_common::tx::PostMessageTx;
use tracing::{debug, error, info, warn};

pub async fn process_post_message_tx<S>(
    connection: &ConsensusConnection<S>,
    post_message_tx: PostMessageTx,
) -> ResponseDeliverTx
where
    S: ConsensusConnectionStorage,
{
    debug!("process_post_message_tx");

    let has_temp_blob = match connection
        .storage
        .get_pinboard_temp_blob(&post_message_tx.content_key)
    {
        Ok(v) => v.is_some(),
        Err(e) => {
            error!(error = %e, "Failed reading temp pinboard blob presence");
            false
        }
    };

    if !has_temp_blob {
        // Keep consensus liveness: missing off-chain bytes must not fail DeliverTx for a valid tx.
        warn!(
            message_id = %post_message_tx.message_id,
            content_key = %post_message_tx.content_key,
            "PostMessage blob missing locally; requesting via P2P"
        );
        connection
            .p2p_sync_coordinator
            .broadcast_content_request(post_message_tx.content_key.clone());
    }

    let mut current_state_lock = connection
        .current_state
        .lock()
        .expect("Failed to acquire current_state lock");
    let current_state = current_state_lock
        .as_mut()
        .expect("current_state lock is None");

    if let Err(e) = validate_namespace_upload_authorization(
        post_message_tx.namespace.clone(),
        post_message_tx.original_signer,
        |slug| current_state.envelope.resolve_namespace(&slug),
    ) {
        return response_deliver_tx_error_validation_failed(e.to_string());
    }

    // deliver_tx runs within the next block; commit() uses `current_state.envelope.block_height + 1`
    // as the new height, so align metadata ordering with that.
    let committed_height = (current_state.envelope.block_height + 1) as u64;

    // `post_message_tx.expires_height` is interpreted as a TTL duration (in blocks).
    // The node converts it to an absolute metadata expiry height: `committed_height + ttl`.
    let ttl_blocks = if post_message_tx.expires_height == 0 {
        connection
            .protocol_constants()
            .default_pinboard_post_ttl_blocks
    } else {
        post_message_tx.expires_height
    };
    let effective_expires_height = committed_height.saturating_add(ttl_blocks);

    let meta = PinboardMessageMetadata {
        message_id: post_message_tx.message_id.clone(),
        original_signer: post_message_tx.original_signer,
        content_key: post_message_tx.content_key.clone(),
        content_type: post_message_tx.content_type.clone(),
        expires_height: effective_expires_height,
        visibility: post_message_tx.visibility.clone(),
        topic: post_message_tx.topic.clone(),
        tags: post_message_tx.tags.clone(),
        committed_height,
        received_timestamp: post_message_tx.received_timestamp,
        namespace: post_message_tx.namespace.clone(),
    };

    // Stage metadata (by message_id).
    current_state
        .envelope
        .pinboard_meta_cache
        .insert(meta.message_id.clone(), meta.clone());

    // Stage secondary indexes.
    let wallet = meta.original_signer.hex_with_prefix();
    current_state.envelope.pinboard_idx_wallet_add.push((
        wallet,
        committed_height,
        meta.message_id.clone(),
    ));

    for tag in &meta.tags {
        current_state.envelope.pinboard_idx_tag_add.push((
            tag.clone(),
            committed_height,
            meta.message_id.clone(),
        ));
    }

    current_state
        .envelope
        .pinboard_idx_expiry_add
        .push((meta.expires_height, meta.message_id.clone()));

    current_state
        .envelope
        .pinboard_idx_commit_add
        .push((committed_height, meta.message_id.clone()));

    // Stage blob refcount increment (blob bytes are handled out-of-band by the upload subsystem).
    *current_state
        .envelope
        .pinboard_refcount_deltas
        .entry(meta.content_key.clone())
        .or_insert(0) += 1;

    info!(
        message_id = %post_message_tx.message_id,
        content_key = %post_message_tx.content_key,
        original_signer = %post_message_tx.original_signer,
        expires_height = effective_expires_height,
        tag_count = post_message_tx.tags.len(),
        "PostMessage staged"
    );
    ResponseDeliverTx {
        code: 0,
        ..Default::default()
    }
}
