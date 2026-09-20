use super::info::{payload_ok, payload_ok_info, QueryProcessorResult};
use crate::storage::traits::ConsensusConnectionStorage;
use eld_common::address::Address;
use eld_common::cado::CadoType;
use eld_common::cado::{CadoPath, CadoPathKey};
use eld_common::error::EldError;
use eld_common::staking_account::StakingAccount;
use eld_common::utils::to_json_string;

pub(crate) fn process_staking_account_query<S>(
    storage: &S,
    _path: String,
    data: Vec<u8>,
) -> QueryProcessorResult
where
    S: ConsensusConnectionStorage,
{
    let address = match String::from_utf8(data.clone()) {
        Ok(addr) => addr,
        Err(e) => {
            return Err(EldError::ValidationError {
                field: "staking_address_utf8".to_string(),
                value: format!("{data:?}"),
                details: e.to_string(),
            });
        }
    };
    let addr = Address::parse_hex_str(&address)?;
    tracing::debug!(address = %addr, "staking account query");
    let cado_path = CadoPath::new(CadoType::StakingAccount, CadoPathKey::Address(addr))?;

    let staking_account = match storage.get_deserialized_cado_by_path::<StakingAccount>(cado_path) {
        Ok(account) => account,
        Err(EldError::NotFoundError { .. }) => {
            return Ok(payload_ok(vec![], "Staking Account not found"));
        }
        Err(e) => return Err(e),
    };

    let info = to_json_string(&staking_account)?;
    Ok(payload_ok_info(vec![], "Staking Account found", info))
}
