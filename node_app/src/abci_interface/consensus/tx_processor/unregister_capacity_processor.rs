use crate::abci_interface::ConsensusConnection;
use crate::errors::response_deliver_tx_error_sender_doesnt_exist;
use crate::storage::traits::ConsensusConnectionStorage;
use abci::types::{Event, ResponseDeliverTx};
use eld_common::{
    cado::{CadoPath, CadoPathKey, CadoType},
    tx::{create_event_attribute, UnregisterCapacityTx},
};
use tracing::{error, info};

pub async fn process_unregister_capacity_tx<S>(
    connection: &ConsensusConnection<S>,
    unregister_capacity_tx: UnregisterCapacityTx,
) -> ResponseDeliverTx
where
    S: ConsensusConnectionStorage,
{
    info!(
        sender = %unregister_capacity_tx.sender,
        unregister = unregister_capacity_tx.unregister,
        "Processing UnregisterCapacity transaction"
    );

    let sender_str = unregister_capacity_tx.sender.to_string();

    let mut current_state_lock = connection
        .current_state
        .lock()
        .expect("Failed to acquire current_state lock");
    let current_state = current_state_lock
        .as_mut()
        .expect("current_state lock is None");

    let sender_path = CadoPath::new(
        CadoType::Account,
        CadoPathKey::Address(unregister_capacity_tx.sender),
    )
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

    let before_count = current_state.envelope.capacity_validators.len();

    info!(
        sender = %sender_str,
        before_capacity_validators_count = before_count,
        "Checking capacity validator for removal from registry"
    );

    let initial_count = current_state.envelope.capacity_validators.len();
    current_state
        .envelope
        .capacity_validators
        .retain(|p| p.address != unregister_capacity_tx.sender);
    let removed = initial_count != current_state.envelope.capacity_validators.len();

    if removed {
        info!(
            sender = %sender_str,
            after_capacity_validators_count = current_state.envelope.capacity_validators.len(),
            "Removed capacity validator from registry"
        );
    } else {
        info!(
            sender = %sender_str,
            "Capacity validator not found in registry; unregistration is idempotent"
        );
    }

    let events = vec![Event {
        r#type: "UnregisterCapacity".into(),
        attributes: vec![
            create_event_attribute("sender".into(), unregister_capacity_tx.sender.to_string()),
            create_event_attribute(
                "unregister".into(),
                unregister_capacity_tx.unregister.to_string(),
            ),
        ],
    }];

    info!(
        sender = %sender_str,
        "UnregisterCapacity transaction processed successfully"
    );

    ResponseDeliverTx {
        code: 0,
        log: "UnregisterCapacity transaction processed successfully".to_string(),
        events,
        ..Default::default()
    }
}
