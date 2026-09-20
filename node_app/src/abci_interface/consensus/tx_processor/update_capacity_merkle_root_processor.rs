use crate::abci_interface::ConsensusConnection;
use crate::errors::{
    response_deliver_tx_error_sender_doesnt_exist, response_deliver_tx_error_validation_failed,
};
use crate::storage::traits::ConsensusConnectionStorage;
use abci::types::{Event, ResponseDeliverTx};
use eld_common::{
    cado::{CadoPath, CadoPathKey, CadoType},
    tx::{create_event_attribute, UpdateCapacityMerkleRootTx},
};
use hex;
use tracing::{error, info};

pub async fn process_update_capacity_merkle_root_tx<S>(
    connection: &ConsensusConnection<S>,
    update_tx: UpdateCapacityMerkleRootTx,
) -> ResponseDeliverTx
where
    S: ConsensusConnectionStorage,
{
    info!(
        sender = %update_tx.sender,
        merkle_root = hex::encode(update_tx.merkle_root),
        "Processing UpdateCapacityMerkleRoot transaction"
    );

    let sender_str = update_tx.sender.to_string();

    let mut current_state_lock = connection
        .current_state
        .lock()
        .expect("Failed to acquire current_state lock");
    let current_state = current_state_lock
        .as_mut()
        .expect("current_state lock is None");

    // Get sender account to verify it exists
    let sender_path = CadoPath::new(CadoType::Account, CadoPathKey::Address(update_tx.sender))
        .expect("Failed to create sender CadoPath");
    let sender_account_with_hash = match current_state
        .envelope
        .get_account_from_cado(&*connection.storage, &sender_path)
    {
        Some(account_with_hash) => account_with_hash,
        None => {
            error!("Sender account not found: {}", sender_str);
            return response_deliver_tx_error_sender_doesnt_exist(sender_str.clone());
        }
    };
    let _sender_account = sender_account_with_hash.account;

    // Find the capacity provider in the list
    let provider_exists = current_state
        .envelope
        .capacity_validators
        .iter()
        .any(|p| p.address == update_tx.sender);

    if !provider_exists {
        error!(
            sender = %sender_str,
            "Capacity provider not found for merkle root update"
        );
        return response_deliver_tx_error_validation_failed(format!(
            "Capacity provider '{sender_str}' not registered. Must register capacity first."
        ));
    }

    let current_block_height = current_state.envelope.block_height;

    // Update existing capacity provider's merkle root
    let mut found = false;
    for provider in current_state.envelope.capacity_validators.iter_mut() {
        if provider.address == update_tx.sender {
            // Update merkle root and timestamp
            provider.merkle_root = Some(update_tx.merkle_root);
            provider.last_merkle_root_update = Some(current_block_height as u64);
            found = true;
            break;
        }
    }

    if !found {
        error!(
            "Capacity provider found in list but could not be updated: {}",
            sender_str
        );
        return response_deliver_tx_error_validation_failed(
            "Failed to update capacity provider merkle root".to_string(),
        );
    }

    // Create events
    let events = vec![Event {
        r#type: "UpdateCapacityMerkleRoot".into(),
        attributes: vec![
            create_event_attribute("sender".into(), update_tx.sender.to_string()),
            create_event_attribute("merkle_root".into(), hex::encode(update_tx.merkle_root)),
            create_event_attribute("block_height".into(), current_block_height.to_string()),
        ],
    }];

    info!(
        sender = %sender_str,
        "UpdateCapacityMerkleRoot transaction processed successfully"
    );

    ResponseDeliverTx {
        code: 0,
        log: "UpdateCapacityMerkleRoot transaction processed successfully".to_string(),
        events,
        ..Default::default()
    }
}
