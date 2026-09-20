use super::info::{payload_ok_info, AccountView, QueryProcessorResult};
use crate::storage::traits::ConsensusConnectionStorage;
use eld_common::account::Account;
use eld_common::address::Address;
use eld_common::cado::CadoType;
use eld_common::cado::{CadoPath, CadoPathKey};
use eld_common::error::EldError;
use eld_common::utils::to_json_string;

pub(crate) fn process_account_view_query<S>(
    storage: &S,
    _path: String,
    data: Vec<u8>,
) -> QueryProcessorResult
where
    S: ConsensusConnectionStorage,
{
    let address = String::from_utf8(data.clone()).map_err(|e| EldError::ValidationError {
        field: "account_address_utf8".to_string(),
        value: format!("{data:?}"),
        details: e.to_string(),
    })?;
    let addr = Address::parse_hex_str(&address)?;
    let account_path = CadoPath::new(CadoType::Account, CadoPathKey::Address(addr))?;
    let account = storage.get_deserialized_cado_by_path::<Account>(account_path)?;

    let account_view = AccountView::from_parts(
        addr,
        account.balance().amount(),
        account.nonce().to_tx_nonce(),
    );

    Ok(payload_ok_info(
        vec![],
        "Account view retrieved".to_string(),
        to_json_string(&account_view)?,
    ))
}
