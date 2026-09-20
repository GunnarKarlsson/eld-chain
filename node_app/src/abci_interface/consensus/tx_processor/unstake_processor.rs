use crate::abci_interface::ConsensusConnection;
use crate::errors::{
    response_deliver_tx_error_insufficient_stake, response_deliver_tx_error_sender_doesnt_exist,
    response_deliver_tx_error_validation_failed,
};
use crate::storage::traits::ConsensusConnectionStorage;
use abci::types::{Event, ResponseDeliverTx};
use bincode;
use eld_common::{
    account::Account,
    cado::{CADOMetadata, CadoBody, CadoPath, CadoPathKey, CadoType},
    coin::Coin,
    tx::{create_event_attribute, UnstakeTx},
};
use tracing::{debug, error};

pub async fn process_unstake_tx<S>(
    connection: &ConsensusConnection<S>,
    unstake_tx: UnstakeTx,
) -> ResponseDeliverTx
where
    S: ConsensusConnectionStorage,
{
    debug!("process_unstake");

    let amount_coin = match Coin::new(unstake_tx.amount.into()) {
        Ok(amount) => amount,
        Err(e) => {
            error!("Failed to create unstake amount coin: {}", e);
            return response_deliver_tx_error_validation_failed(e.to_string());
        }
    };
    let sender_str = unstake_tx.sender.to_string();
    debug!("sender_address: {}", sender_str);

    let mut current_state_lock = connection
        .current_state
        .lock()
        .expect("Failed to acquire current_state lock");
    let current_state = current_state_lock
        .as_mut()
        .expect("current_state lock is None");

    // Get sender account
    let sender_path = CadoPath::new(CadoType::Account, CadoPathKey::Address(unstake_tx.sender))
        .expect("Failed to create sender CadoPath");
    let sender_account_with_hash = match current_state
        .envelope
        .get_account_from_cado(&*connection.storage, &sender_path)
    {
        Some(account_with_hash) => account_with_hash,
        None => return response_deliver_tx_error_sender_doesnt_exist(sender_str.clone()),
    };
    let sender_account = sender_account_with_hash.account;

    // Get staking account
    let staking_account_path = CadoPath::new(
        CadoType::StakingAccount,
        CadoPathKey::Address(unstake_tx.sender),
    )
    .expect("Failed to create staking account CadoPath");
    let staking_account_with_hash = match current_state
        .envelope
        .get_staking_account_from_cado(&*connection.storage, &staking_account_path)
    {
        Some(account_with_hash) => account_with_hash,
        None => return response_deliver_tx_error_insufficient_stake(),
    };
    let mut staking_account = staking_account_with_hash.account;

    // Check if sender has sufficient stake
    if staking_account.stake_balance < amount_coin {
        debug!("Sender has insufficient stake");
        return response_deliver_tx_error_insufficient_stake();
    }

    // Update staking account using Coin arithmetic
    staking_account.stake_balance = match staking_account.stake_balance - amount_coin {
        Ok(coin) => coin,
        Err(e) => {
            error!("Failed to subtract unstake amount: {}", e);
            return response_deliver_tx_error_validation_failed(e.to_string());
        }
    };

    let staking_account_serialized = staking_account
        .serialize_bin()
        .expect("Failed to serialize staking account");
    let staking_meta = CADOMetadata::new(CadoType::StakingAccount, &sender_str);
    let staking_account_cado = CadoBody::mutable_updated(
        staking_account_with_hash.hash,
        staking_account_serialized,
        staking_meta,
    );
    let staking_path = CadoPath::new(
        CadoType::StakingAccount,
        CadoPathKey::Address(unstake_tx.sender),
    )
    .expect("Failed to create staking account CadoPath");
    current_state
        .envelope
        .update_cado_cache(staking_path, staking_account_cado);

    // Update sender account
    let updated_sender = match sender_account.balance() + amount_coin {
        Ok(balance) => Account::new(*sender_account.address(), balance, sender_account.nonce()),
        Err(e) => {
            error!("Failed to add unstake amount to sender balance: {}", e);
            return response_deliver_tx_error_validation_failed(e.to_string());
        }
    };

    let sender_serialized =
        bincode::serialize(&updated_sender).expect("Failed to serialize updated sender");
    let sender_meta = CADOMetadata::new(CadoType::Account, &sender_str);
    let sender_cado = CadoBody::mutable_updated(
        sender_account_with_hash.hash,
        sender_serialized,
        sender_meta,
    );
    let sender_path = CadoPath::new(CadoType::Account, CadoPathKey::Address(unstake_tx.sender))
        .expect("Failed to create sender CadoPath");
    current_state
        .envelope
        .update_cado_cache(sender_path, sender_cado);

    // Create events
    let events = vec![Event {
        r#type: "Unstake".into(),
        attributes: vec![
            create_event_attribute("sender".into(), unstake_tx.sender.to_string()),
            create_event_attribute("amount".into(), unstake_tx.amount.to_string()),
        ],
    }];

    ResponseDeliverTx {
        code: 0,
        log: "Unstake transaction processed successfully".to_string(),
        events,
        ..Default::default()
    }
}
