use crate::abci_interface::ConsensusConnection;
use crate::errors::{
    response_deliver_tx_error_insufficient_funds, response_deliver_tx_error_sender_doesnt_exist,
    response_deliver_tx_error_validation_failed,
};
use crate::storage::traits::ConsensusConnectionStorage;
use abci::types::{Event, ResponseDeliverTx};
use bincode;
use eld_common::{
    account::Account,
    cado::{CADOMetadata, CadoBody, CadoPath, CadoPathKey, CadoType},
    coin::Coin,
    constants::tx_type,
    namespace::{slug_to_namespace_cadopath, NamespaceRecord},
    tx::{create_event_attribute, AddNamespaceTx},
};
use tracing::{debug, error, info};

pub async fn process_add_namespace_tx<S>(
    connection: &ConsensusConnection<S>,
    add_namespace_tx: AddNamespaceTx,
) -> ResponseDeliverTx
where
    S: ConsensusConnectionStorage,
{
    debug!(
        sender = %add_namespace_tx.sender,
        namespace_slug = %add_namespace_tx.namespace_slug,
        registration_fee = %add_namespace_tx.registration_fee,
        "process_add_namespace_tx"
    );

    let registration_fee = match Coin::new(add_namespace_tx.registration_fee.into()) {
        Ok(fee) => fee,
        Err(e) => {
            error!("Failed to parse registration_fee: {}", e);
            return response_deliver_tx_error_validation_failed(e.to_string());
        }
    };

    let canonical_slug = add_namespace_tx.namespace_slug.clone();
    let sender = add_namespace_tx.sender;
    let sender_str = sender.to_string();

    let mut current_state_lock = connection
        .current_state
        .lock()
        .expect("Failed to acquire current_state lock");
    let current_state = current_state_lock
        .as_mut()
        .expect("current_state lock is None");

    if let Err(e) = current_state
        .envelope
        .validate_add_namespace_not_taken(&canonical_slug)
    {
        return response_deliver_tx_error_validation_failed(e.to_string());
    }

    let registry_path = match slug_to_namespace_cadopath(&canonical_slug) {
        Ok(path) => path,
        Err(e) => return response_deliver_tx_error_validation_failed(e.to_string()),
    };

    if registration_fee == Coin::zero() {
        return response_deliver_tx_error_validation_failed(
            "registration_fee must be greater than zero".to_string(),
        );
    }

    let sender_path =
        CadoPath::new(CadoType::Account, CadoPathKey::Address(sender)).expect("sender path");
    let sender_account_with_hash = match current_state
        .envelope
        .get_account_from_cado(&*connection.storage, &sender_path)
    {
        Some(account_with_hash) => account_with_hash,
        None => return response_deliver_tx_error_sender_doesnt_exist(sender_str.clone()),
    };

    if sender_account_with_hash.account.balance() < registration_fee {
        return response_deliver_tx_error_insufficient_funds();
    }

    let updated_balance = match sender_account_with_hash.account.balance() - registration_fee {
        Ok(balance) => balance,
        Err(e) => return response_deliver_tx_error_validation_failed(e.to_string()),
    };

    let updated_sender = Account::new(
        sender,
        updated_balance,
        sender_account_with_hash.account.nonce(),
    );
    let sender_serialized = bincode::serialize(&updated_sender)
        .expect("Failed to serialize sender after registration_fee deduction");
    let sender_cado = CadoBody::mutable_updated(
        sender_account_with_hash.hash,
        sender_serialized,
        CADOMetadata::new(CadoType::Account, &sender_str),
    );
    current_state
        .envelope
        .update_cado_cache(sender_path, sender_cado);

    let registered_height = (current_state.envelope.block_height + 1) as u64;
    let record = NamespaceRecord {
        namespace_slug: canonical_slug.clone(),
        owner: sender,
        registered_height,
    };

    current_state
        .envelope
        .namespace_registry_cache
        .insert(canonical_slug.clone(), record);

    info!(
        namespace_slug = %canonical_slug,
        owner = %sender,
        registry_path = %registry_path.as_str(),
        registered_height,
        "AddNamespace staged"
    );

    let events = vec![Event {
        r#type: tx_type::TX_TYPE_ADD_NAMESPACE.into(),
        attributes: vec![
            create_event_attribute("namespace".into(), canonical_slug),
            create_event_attribute("owner".into(), sender_str),
            create_event_attribute("registry_path".into(), registry_path.as_str().to_string()),
        ],
    }];

    ResponseDeliverTx {
        code: 0,
        log: "AddNamespace transaction staged successfully".to_string(),
        events,
        ..Default::default()
    }
}
