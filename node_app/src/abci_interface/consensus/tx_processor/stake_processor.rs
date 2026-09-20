use crate::abci_interface::ConsensusConnection;
use crate::app_state::StakingAccountWithCadoHash;
use crate::errors::{
    response_deliver_tx_error_insufficient_funds,
    response_deliver_tx_error_invalid_public_key_format,
    response_deliver_tx_error_invalid_public_key_length, response_deliver_tx_error_minimum_stake,
    response_deliver_tx_error_sender_doesnt_exist, response_deliver_tx_error_validation_failed,
};
use crate::storage::traits::ConsensusConnectionStorage;
use abci::types::{Event, ResponseDeliverTx};
use bincode;
use eld_common::{
    account::Account,
    cado::{CADOMetadata, CadoBody, CadoPath, CadoPathKey, CadoType},
    coin::Coin,
    constants::{protocol::MIN_STAKE_AMOUNT, tx_type},
    staking_account::StakingAccount,
    tx::{create_event_attribute, StakeTx},
    validator::ValidatorInfo,
};
use hex;
use tracing::{debug, error};

pub async fn process_stake_tx<S>(
    connection: &ConsensusConnection<S>,
    stake_tx: StakeTx,
) -> ResponseDeliverTx
where
    S: ConsensusConnectionStorage,
{
    debug!("process_stake");

    let amount_coin = match Coin::new(stake_tx.amount.into()) {
        Ok(amount) => amount,
        Err(e) => {
            error!("Failed to create stake amount coin: {}", e);
            return response_deliver_tx_error_validation_failed(e.to_string());
        }
    };
    let sender_str = stake_tx.sender.to_string();

    let mut current_state_lock = connection
        .current_state
        .lock()
        .expect("Failed to acquire current_state lock");
    let current_state = current_state_lock
        .as_mut()
        .expect("current_state lock is None");

    // Get sender account
    let sender_path = CadoPath::new(CadoType::Account, CadoPathKey::Address(stake_tx.sender))
        .expect("Failed to create sender CadoPath");
    debug!("sender_path: {}", sender_path.as_str());
    let sender_account_with_hash = match current_state
        .envelope
        .get_account_from_cado(&*connection.storage, &sender_path)
    {
        Some(account_with_hash) => account_with_hash,
        None => return response_deliver_tx_error_sender_doesnt_exist(sender_str.clone()),
    };
    let sender_account = sender_account_with_hash.account;

    // Check sender has sufficient funds
    if sender_account.balance() < amount_coin {
        debug!("Sender has insufficient balance");
        return response_deliver_tx_error_insufficient_funds();
    }

    // Check state is >= minimum stake size (this means each stake tx needs to >= this amount)
    // TODO: Change this so initial amount >= minimum then can add smaller amounts on top
    debug!("stake: {}, min: {}", amount_coin, MIN_STAKE_AMOUNT);
    let min_stake_coin = match Coin::new(MIN_STAKE_AMOUNT) {
        Ok(amount) => amount,
        Err(e) => {
            error!("Failed to create minimum stake amount coin: {}", e);
            return response_deliver_tx_error_validation_failed(e.to_string());
        }
    };
    if amount_coin < min_stake_coin {
        debug!("Amount is less than min stake amount");
        return response_deliver_tx_error_minimum_stake();
    }

    // Update sender and store in cache
    let updated_sender = match sender_account.balance() - amount_coin {
        Ok(balance) => Account::new(*sender_account.address(), balance, sender_account.nonce()),
        Err(e) => {
            error!("Failed to subtract stake amount from sender balance: {}", e);
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
    let sender_path = CadoPath::new(CadoType::Account, CadoPathKey::Address(stake_tx.sender))
        .expect("Failed to create sender CadoPath");
    current_state
        .envelope
        .update_cado_cache(sender_path, sender_cado);

    // Update staking account
    // Get staking account
    let staking_account_path = CadoPath::new(
        CadoType::StakingAccount,
        CadoPathKey::Address(stake_tx.sender),
    )
    .expect("Failed to create staking account CadoPath");
    let staking_account_with_hash = match current_state
        .envelope
        .get_staking_account_from_cado(&*connection.storage, &staking_account_path)
    {
        Some(account_with_hash) => account_with_hash,
        None => {
            let sender_addr = stake_tx.sender;
            StakingAccountWithCadoHash {
                account: StakingAccount {
                    address: sender_addr,
                    originator: sender_addr, // Same as address for self-stake
                    stake_balance: Coin::zero(),
                },
                hash: [0; 32], // Initial hash for new account
            }
        }
    };
    let mut staking_account = staking_account_with_hash.account;

    // Update staking account using Coin arithmetic
    staking_account.stake_balance = match staking_account.stake_balance + amount_coin {
        Ok(coin) => coin,
        Err(e) => {
            error!("Failed to add stake amount: {}", e);
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
        CadoPathKey::Address(stake_tx.sender),
    )
    .expect("Failed to create staking account CadoPath");
    current_state
        .envelope
        .update_cado_cache(staking_path, staking_account_cado);

    // Only update validators list if we have a public key
    if let Some(public_key) = stake_tx.public_key.as_ref() {
        // Decode hex public key
        let public_key_bytes = match hex::decode(public_key) {
            Ok(bytes) => bytes,
            Err(_) => {
                debug!("Invalid public key format");
                return response_deliver_tx_error_invalid_public_key_format();
            }
        };

        // Verify public key length
        if public_key_bytes.len() != 32 {
            debug!("Invalid public key length");
            return response_deliver_tx_error_invalid_public_key_length();
        }

        let validator_exists = current_state
            .envelope
            .validators
            .iter()
            .any(|v| v.address == stake_tx.sender);

        if !validator_exists {
            // Add new validator to the list
            current_state.envelope.validators.push(ValidatorInfo {
                address: stake_tx.sender,
                stake: staking_account.stake_balance,
                public_key: public_key_bytes,
            });
        } else {
            // Update existing validator's stake
            for validator in current_state.envelope.validators.iter_mut() {
                if validator.address == stake_tx.sender {
                    validator.stake = staking_account.stake_balance;
                    // Update public key if it changed
                    if validator.public_key != public_key_bytes {
                        validator.public_key = public_key_bytes.clone();
                    }
                    break;
                }
            }
        }
    }

    // Create events
    let events = vec![Event {
        r#type: tx_type::TX_TYPE_STAKE.into(),
        attributes: vec![
            create_event_attribute("sender".into(), stake_tx.sender.to_string()),
            create_event_attribute("amount".into(), stake_tx.amount.to_string()),
        ],
    }];

    ResponseDeliverTx {
        code: 0,
        log: "Stake transaction processed successfully".to_string(),
        events,
        ..Default::default()
    }
}
