use crate::abci_interface::ConsensusConnection;
use crate::app_state::AccountWithCadoHash;
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
    nonce::Nonce,
    tx::{create_event_attribute, TransferTx},
};
use tracing::{error, info, warn};

pub async fn process_transfer_tx<S>(
    connection: &ConsensusConnection<S>,
    transfer_tx: TransferTx,
) -> ResponseDeliverTx
where
    S: ConsensusConnectionStorage,
{
    info!(
        sender = %transfer_tx.sender,
        recipient = %transfer_tx.recipient,
        amount = %transfer_tx.amount,
        "process_transfer_tx"
    );

    let amount_coin = match Coin::new(transfer_tx.amount.into()) {
        Ok(amount) => amount,
        Err(e) => {
            error!("Failed to create transfer amount coin: {}", e);
            return response_deliver_tx_error_validation_failed(e.to_string());
        }
    };

    let sender_str = transfer_tx.sender.to_string();
    let recipient_str = transfer_tx.recipient.to_string();

    let mut current_state_lock = connection
        .current_state
        .lock()
        .expect("Failed to acquire current_state lock");
    let current_state = current_state_lock
        .as_mut()
        .expect("current_state lock is None");

    // Get sender account
    let sender_path = CadoPath::new(CadoType::Account, CadoPathKey::Address(transfer_tx.sender))
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
    let sender_account = sender_account_with_hash.account;

    // Log OLD sender balance
    info!(
        sender = %sender_str,
        old_balance_raw = sender_account.balance().amount(),
        old_balance_display = %sender_account.balance(),
        "XVXV Transfer: Sender OLD balance"
    );

    // Get recipient account
    let recipient_path = CadoPath::new(
        CadoType::Account,
        CadoPathKey::Address(transfer_tx.recipient),
    )
    .expect("Failed to create recipient CadoPath");
    let recipient_account_with_hash = match current_state
        .envelope
        .get_account_from_cado(&*connection.storage, &recipient_path)
    {
        Some(account_with_hash) => account_with_hash,
        None => {
            warn!(
                "Recipient account not found, initializing: {}",
                recipient_str
            );
            AccountWithCadoHash {
                account: Account::new(transfer_tx.recipient, Coin::zero(), Nonce::new(Nonce::ZERO)),
                hash: [0; 32], // Initial hash for new account
            }
        }
    };
    let recipient_account = recipient_account_with_hash.account;

    // Log OLD recipient balance
    info!(
        recipient = %recipient_str,
        old_balance_raw = recipient_account.balance().amount(),
        old_balance_display = %recipient_account.balance(),
        "XVXV Transfer: Recipient OLD balance"
    );

    if sender_account.balance() < amount_coin {
        error!(
            "Sender has insufficient funds: {} < {}",
            sender_account.balance(),
            amount_coin
        );
        return response_deliver_tx_error_insufficient_funds();
    }

    // Update sender and store in cache
    let updated_sender = match sender_account.balance() - amount_coin {
        Ok(balance) => Account::new(*sender_account.address(), balance, sender_account.nonce()),
        Err(e) => {
            error!(
                "Failed to subtract transfer amount from sender balance: {}",
                e
            );
            return response_deliver_tx_error_validation_failed(e.to_string());
        }
    };

    // Log NEW sender balance
    info!(
        sender = %sender_str,
        new_balance_raw = updated_sender.balance().amount(),
        new_balance_display = %updated_sender.balance(),
        transfer_amount = amount_coin.amount(),
        "XVXV Transfer: Sender NEW balance (before write)"
    );

    let sender_serialized =
        bincode::serialize(&updated_sender).expect("Failed to serialize updated sender");
    let sender_meta = CADOMetadata::new(CadoType::Account, &sender_str);
    let sender_cado = CadoBody::mutable_updated(
        sender_account_with_hash.hash,
        sender_serialized,
        sender_meta,
    );
    let sender_path = CadoPath::new(CadoType::Account, CadoPathKey::Address(transfer_tx.sender))
        .expect("Failed to create sender CadoPath");
    current_state
        .envelope
        .update_cado_cache(sender_path, sender_cado);

    // Update recipient and store in cash
    let updated_recipient = match recipient_account.balance() + amount_coin {
        Ok(balance) => Account::new(
            *recipient_account.address(),
            balance,
            recipient_account.nonce(),
        ),
        Err(e) => {
            error!("Failed to add transfer amount to recipient balance: {}", e);
            return response_deliver_tx_error_validation_failed(e.to_string());
        }
    };

    // Log NEW recipient balance
    info!(
        recipient = %recipient_str,
        new_balance_raw = updated_recipient.balance().amount(),
        new_balance_display = %updated_recipient.balance(),
        transfer_amount = amount_coin.amount(),
        "XVXV Transfer: Recipient NEW balance (before write)"
    );

    let recipient_serialized =
        bincode::serialize(&updated_recipient).expect("Failed to serialize updated recipient");
    let recipient_meta = CADOMetadata::new(CadoType::Account, &recipient_str);
    let recipient_cado = CadoBody::mutable_updated(
        recipient_account_with_hash.hash,
        recipient_serialized,
        recipient_meta,
    );
    let recipient_path = CadoPath::new(
        CadoType::Account,
        CadoPathKey::Address(transfer_tx.recipient),
    )
    .expect("Failed to create recipient CadoPath");
    current_state
        .envelope
        .update_cado_cache(recipient_path, recipient_cado);

    // Create events
    let events = vec![Event {
        r#type: tx_type::TX_TYPE_TRANSFER.into(),
        attributes: vec![
            create_event_attribute("sender".into(), transfer_tx.sender.to_string()),
            create_event_attribute("recipient".into(), transfer_tx.recipient.to_string()),
            create_event_attribute("amount".into(), transfer_tx.amount.to_string()),
        ],
    }];

    info!(
        "Transfer processed: {} -> {} amount {}",
        sender_str, recipient_str, transfer_tx.amount
    );
    ResponseDeliverTx {
        code: 0,
        log: "Transfer transaction processed successfully".to_string(),
        events,
        ..Default::default()
    }
}
