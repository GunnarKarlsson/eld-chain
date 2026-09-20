use crate::abci_interface::ConsensusConnection;
use crate::errors::{
    response_deliver_tx_error_invalid_public_key_format,
    response_deliver_tx_error_sender_doesnt_exist, response_deliver_tx_error_validation_failed,
};
use crate::storage::traits::ConsensusConnectionStorage;
use abci::types::{Event, ResponseDeliverTx};
use eld_common::{
    cado::{CadoPath, CadoPathKey, CadoType},
    coin::Coin,
    constants::protocol::DEFAULT_REGISTRATION_DURATION_BLOCKS,
    public_key::PublicKey,
    tx::{create_event_attribute, RegisterCapacityTx, TxPublicKey},
    validator::CapacityValidatorInfo,
};
use hex;
use tracing::{error, info};

pub async fn process_register_capacity_tx<S>(
    connection: &ConsensusConnection<S>,
    register_capacity_tx: RegisterCapacityTx,
    outer_public_key: TxPublicKey,
) -> ResponseDeliverTx
where
    S: ConsensusConnectionStorage,
{
    info!(
        sender = %register_capacity_tx.sender,
        capacity_bytes = register_capacity_tx.capacity_bytes,
        chunk_count = register_capacity_tx.chunk_count,
        "Processing RegisterCapacity transaction"
    );

    let sender = register_capacity_tx.sender;

    let public_key = match PublicKey::try_from(&outer_public_key) {
        Ok(pk) => pk,
        Err(_) => {
            error!(sender = %sender, "Invalid outer tx public_key on RegisterCapacity");
            return response_deliver_tx_error_invalid_public_key_format();
        }
    };
    if !register_capacity_tx
        .sender
        .is_from_public_key(public_key.as_bytes())
    {
        error!(
            sender = %sender,
            "RegisterCapacity sender does not match outer tx public_key"
        );
        return response_deliver_tx_error_validation_failed(
            "RegisterCapacity sender does not match outer tx public_key".to_string(),
        );
    }

    let mut current_state_lock = connection
        .current_state
        .lock()
        .expect("Failed to acquire current_state lock");
    let current_state = current_state_lock
        .as_mut()
        .expect("current_state lock is None");

    let sender_path = CadoPath::new(
        CadoType::Account,
        CadoPathKey::Address(register_capacity_tx.sender),
    )
    .expect("Failed to create sender CadoPath");
    let sender_account_with_hash = match current_state
        .envelope
        .get_account_from_cado(&*connection.storage, &sender_path)
    {
        Some(account_with_hash) => account_with_hash,
        None => {
            error!("Sender account not found: {}", sender);
            return response_deliver_tx_error_sender_doesnt_exist(sender.to_string());
        }
    };
    let _sender_account = sender_account_with_hash.account;

    let provider_exists = current_state
        .envelope
        .capacity_validators
        .iter()
        .any(|p| p.address == sender);

    let current_block_height = current_state.envelope.block_height;

    if !provider_exists {
        info!(
            sender = %sender,
            "Creating new capacity validator entry for capacity registration"
        );
        let registered_block = (current_block_height + 1) as u64;
        current_state
            .envelope
            .capacity_validators
            .push(CapacityValidatorInfo {
                address: sender,
                stake: Coin::zero(),
                public_key,
                storage_capacity: register_capacity_tx.capacity_bytes,
                merkle_root: Some(register_capacity_tx.merkle_root),
                seed: Some(register_capacity_tx.seed),
                chunk_count: Some(register_capacity_tx.chunk_count),
                registered_at: Some(current_block_height as u64),
                last_merkle_root_update: Some(current_block_height as u64),
                registered_block,
                registration_duration: DEFAULT_REGISTRATION_DURATION_BLOCKS,
            });
    } else {
        info!(
            sender = %sender,
            "Updating existing capacity validator with capacity proof"
        );
        let mut found = false;
        let registered_block = (current_block_height + 1) as u64;
        for provider in current_state.envelope.capacity_validators.iter_mut() {
            if provider.address == sender {
                if provider.public_key != public_key {
                    error!(
                        sender = %sender,
                        "RegisterCapacity rejected: capacity_validators public key conflict"
                    );
                    return response_deliver_tx_error_validation_failed(
                        "capacity_validators public key conflict for address".to_string(),
                    );
                }

                provider.merkle_root = Some(register_capacity_tx.merkle_root);
                provider.seed = Some(register_capacity_tx.seed);
                provider.chunk_count = Some(register_capacity_tx.chunk_count);
                provider.storage_capacity = register_capacity_tx.capacity_bytes;

                if provider.registered_at.is_none() {
                    provider.registered_at = Some(current_block_height as u64);
                }

                provider.last_merkle_root_update = Some(current_block_height as u64);
                provider.registered_block = registered_block;
                provider.registration_duration = DEFAULT_REGISTRATION_DURATION_BLOCKS;

                found = true;
                break;
            }
        }

        if !found {
            error!(
                "Capacity validator found in list but could not be updated: {}",
                sender
            );
            return response_deliver_tx_error_validation_failed(
                "Failed to update capacity validator".to_string(),
            );
        }
    }

    let events = vec![Event {
        r#type: "RegisterCapacity".into(),
        attributes: vec![
            create_event_attribute("sender".into(), register_capacity_tx.sender.to_string()),
            create_event_attribute(
                "capacity_bytes".into(),
                register_capacity_tx.capacity_bytes.to_string(),
            ),
            create_event_attribute(
                "chunk_count".into(),
                register_capacity_tx.chunk_count.to_string(),
            ),
            create_event_attribute(
                "merkle_root".into(),
                hex::encode(register_capacity_tx.merkle_root),
            ),
            create_event_attribute("seed".into(), hex::encode(register_capacity_tx.seed)),
            create_event_attribute("registered_at".into(), current_block_height.to_string()),
        ],
    }];

    info!(
        sender = %sender,
        "RegisterCapacity transaction processed successfully"
    );

    ResponseDeliverTx {
        code: 0,
        log: "RegisterCapacity transaction processed successfully".to_string(),
        events,
        ..Default::default()
    }
}
