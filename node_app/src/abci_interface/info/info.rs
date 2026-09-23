use super::abci_rate_limiting::{
    AbciQueryType, GlobalAbciRateLimiter, QueryBasedRateLimiter, ResultLimiter,
};
use super::{
    account_count_query_processor, account_view_query_processor, active_validators_query_processor,
    cado_list_query_processor, cado_paths_query_processor, cado_query_processor,
    capacity_provider_query_processor, capacity_validators_query_processor,
    epoch_info_query_processor, estimate_fee_query_processor, pinboard_feed_query_processor,
    pinboard_query_processor, staking_account_query_processor,
};
use crate::abci_interface::chain_tip::ChainTip;
use crate::app_state::AppState;
use crate::config::ConsensusConfig;
use crate::storage::traits::ConsensusConnectionStorage;
use abci::{async_api::Info, async_trait, types::*};
use eld_common::address::Address;
use eld_common::constants::{
    abci_query,
    protocol::{BLOCKS_PER_EPOCH, VALIDATORS_PER_EPOCH},
};
use eld_common::error::EldError;
use serde::{Deserialize, Serialize};
use std::str;
use std::sync::{Arc, Mutex};
use tracing::{error, info, warn};

/// Deploy/image version from `ELD_APP_VERSION`, or `"0.0.0"` if unset/empty.
pub fn resolve_eld_app_version() -> String {
    match std::env::var("ELD_APP_VERSION") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => "0.0.0".to_string(),
    }
}

#[derive(Serialize)]
pub struct StateData {
    /// Cached at process start from [`resolve_eld_app_version`].
    pub eld_app_version: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AccountCount {
    pub count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MetadataCount {
    pub count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AccountView {
    /// Wire hex form (`0x` + lowercase); keep `String` for ABCI JSON schema stability.
    pub address: String,
    pub balance: u128,
    pub nonce: u32,
}

impl AccountView {
    /// Builds the ABCI response DTO from a typed address at the query edge.
    pub fn from_parts(address: Address, balance: u128, nonce: u32) -> Self {
        Self {
            address: address.hex_with_prefix(),
            balance,
            nonce,
        }
    }
}

#[derive(Debug, Clone)]
pub enum EldAbciQuery {
    EstimateFee { path: String, data: Vec<u8> },
    StakingAccount { path: String, data: Vec<u8> },
    ActiveValidators { path: String, data: Vec<u8> },
    CapacityValidators { path: String, data: Vec<u8> },
    CapacityProvider { path: String, data: Vec<u8> },
    EpochInfo { path: String, data: Vec<u8> },
    AccountCount { path: String, data: Vec<u8> },
    Cado { path: String, data: Vec<u8> },
    CadoList { path: String, data: Vec<u8> },
    CadoPaths { path: String, data: Vec<u8> },
    AccountView { path: String, data: Vec<u8> },
    Pinboard { path: String, data: Vec<u8> },
    PinboardFeed { path: String, data: Vec<u8> },
    Unknown { path: String, _data: Vec<u8> },
}

#[derive(Debug, Clone)]
pub struct QueryResponsePayload {
    pub code: u32,
    pub value: Vec<u8>,
    pub log: String,
    pub info: String,
}

pub type QueryProcessorResult = Result<QueryResponsePayload, EldError>;

pub(crate) fn payload_ok(value: Vec<u8>, log: impl Into<String>) -> QueryResponsePayload {
    QueryResponsePayload {
        code: 0,
        value,
        log: log.into(),
        info: String::new(),
    }
}

pub(crate) fn payload_ok_info(
    value: Vec<u8>,
    log: impl Into<String>,
    info: impl Into<String>,
) -> QueryResponsePayload {
    QueryResponsePayload {
        code: 0,
        value,
        log: log.into(),
        info: info.into(),
    }
}

impl EldAbciQuery {
    pub fn from_abci_query_request(request: RequestQuery) -> Self {
        let path = request.path;
        let data = request.data;

        match path.as_str() {
            abci_query::ESTIMATE_FEE => Self::EstimateFee { path, data },
            abci_query::STAKING_ACCOUNT => Self::StakingAccount { path, data },
            abci_query::ACTIVE_VALIDATORS => Self::ActiveValidators { path, data },
            abci_query::ACTIVE_CAPACITY_VALIDATORS => Self::CapacityValidators { path, data },
            abci_query::CAPACITY_PROVIDER => Self::CapacityProvider { path, data },
            abci_query::EPOCH_INFO => Self::EpochInfo { path, data },
            abci_query::ACCOUNT_COUNT => Self::AccountCount { path, data },
            abci_query::CADO => Self::Cado { path, data },
            abci_query::CADO_LIST => Self::CadoList { path, data },
            abci_query::CADO_PATHS => Self::CadoPaths { path, data },
            abci_query::ACCOUNT_VIEW => Self::AccountView { path, data },
            abci_query::PINBOARD => Self::Pinboard { path, data },
            abci_query::PINBOARD_FEED => Self::PinboardFeed { path, data },
            _ => Self::Unknown { path, _data: data },
        }
    }

    pub fn path(&self) -> &str {
        match self {
            Self::EstimateFee { path, .. }
            | Self::StakingAccount { path, .. }
            | Self::ActiveValidators { path, .. }
            | Self::CapacityValidators { path, .. }
            | Self::CapacityProvider { path, .. }
            | Self::EpochInfo { path, .. }
            | Self::AccountCount { path, .. }
            | Self::Cado { path, .. }
            | Self::CadoList { path, .. }
            | Self::CadoPaths { path, .. }
            | Self::AccountView { path, .. }
            | Self::Pinboard { path, .. }
            | Self::PinboardFeed { path, .. }
            | Self::Unknown { path, .. } => path,
        }
    }

    /// Processes the incoming ABCI Info Query.
    pub fn process<S>(self, conn: &InfoConnection<S>) -> QueryProcessorResult
    where
        S: ConsensusConnectionStorage,
    {
        match self {
            Self::EstimateFee { path, data } => {
                estimate_fee_query_processor::process_estimate_fee_query(
                    &conn.consensus_config,
                    path,
                    data,
                )
            }
            Self::StakingAccount { path, data } => {
                staking_account_query_processor::process_staking_account_query::<S>(
                    conn.storage.as_ref(),
                    path,
                    data,
                )
            }
            Self::ActiveValidators { path, data } => {
                active_validators_query_processor::process_active_validators_query(
                    &conn.state,
                    path,
                    data,
                )
            }
            Self::CapacityValidators { path, data } => {
                capacity_validators_query_processor::process_capacity_validators_query(
                    &conn.state,
                    path,
                    data,
                )
            }
            Self::CapacityProvider { path, data } => {
                capacity_provider_query_processor::process_capacity_provider_query(
                    &conn.state,
                    path,
                    data,
                )
            }
            Self::EpochInfo { path, data } => epoch_info_query_processor::process_epoch_info_query(
                &conn.state,
                path,
                data,
                BLOCKS_PER_EPOCH,
                VALIDATORS_PER_EPOCH,
            ),
            Self::AccountCount { path, data } => {
                account_count_query_processor::process_account_count_query::<S>(
                    conn.storage.as_ref(),
                    path,
                    data,
                )
            }
            Self::Cado { path, data } => {
                cado_query_processor::process_cado_query::<S>(conn.storage.as_ref(), path, data)
            }
            Self::CadoList { path, data } => {
                cado_list_query_processor::process_cado_list_query::<S>(
                    conn.storage.as_ref(),
                    conn.result_limiter.as_ref(),
                    AbciQueryType::CadoList,
                    path,
                    data,
                )
            }
            Self::CadoPaths { path, data } => {
                cado_paths_query_processor::process_cado_paths_query::<S>(
                    conn.storage.as_ref(),
                    conn.result_limiter.as_ref(),
                    AbciQueryType::CadoList,
                    path,
                    data,
                )
            }
            Self::AccountView { path, data } => {
                account_view_query_processor::process_account_view_query::<S>(
                    conn.storage.as_ref(),
                    path,
                    data,
                )
            }
            Self::Pinboard { path, data } => {
                let current_height = conn.chain_tip.committed_height_u64();
                pinboard_query_processor::process_pinboard_query::<S>(
                    conn.storage.as_ref(),
                    current_height,
                    path,
                    data,
                )
            }
            Self::PinboardFeed { path, data } => {
                pinboard_feed_query_processor::process_pinboard_feed_query::<S>(
                    conn.storage.as_ref(),
                    path,
                    data,
                )
            }
            Self::Unknown { path, _data } => {
                warn!("unknown path: {path}");
                Err(EldError::ValidationError {
                    field: "query_path".to_string(),
                    value: path,
                    details: "Unknown query path".to_string(),
                })
            }
        }
    }
}

pub struct InfoConnection<S>
where
    S: ConsensusConnectionStorage,
{
    state: Arc<Mutex<AppState>>,
    chain_tip: Arc<ChainTip>,
    storage: Arc<S>,
    consensus_config: Arc<Mutex<ConsensusConfig>>,
    /// Process-lifetime cache of [`resolve_eld_app_version`] (read once at construction).
    eld_app_version: String,
    global_rate_limiter: Arc<GlobalAbciRateLimiter>,
    result_limiter: Arc<ResultLimiter>,
    query_rate_limiter: Arc<QueryBasedRateLimiter>,
}

impl<S> InfoConnection<S>
where
    S: ConsensusConnectionStorage,
{
    pub fn new(
        state: Arc<Mutex<AppState>>,
        chain_tip: Arc<ChainTip>,
        storage: Arc<S>,
        consensus_config: Arc<Mutex<ConsensusConfig>>,
    ) -> Self {
        let eld_app_version = resolve_eld_app_version();
        info!(eld_app_version = %eld_app_version, "Cached ELD_APP_VERSION for abci_info");
        Self {
            state,
            chain_tip,
            storage,
            consensus_config,
            eld_app_version,
            global_rate_limiter: Arc::new(GlobalAbciRateLimiter::new()),
            result_limiter: Arc::new(ResultLimiter::new()),
            query_rate_limiter: Arc::new(QueryBasedRateLimiter::new()),
        }
    }

    pub(crate) fn to_query_response(
        path: impl Into<String>,
        key: Vec<u8>,
        payload: QueryResponsePayload,
    ) -> ResponseQuery {
        ResponseQuery {
            code: payload.code,
            log: payload.log,
            key,
            value: payload.value,
            height: 0,
            codespace: path.into(),
            info: payload.info,
            index: 0,
            proof_ops: None,
        }
    }

    pub(crate) fn map_query_error(
        path: impl Into<String>,
        key: Vec<u8>,
        err: EldError,
    ) -> ResponseQuery {
        let normalized = err.to_string().replace('\n', " ").trim().to_string();
        ResponseQuery {
            code: 1,
            log: normalized,
            key,
            value: vec![],
            height: 0,
            codespace: path.into(),
            info: String::new(),
            index: 0,
            proof_ops: None,
        }
    }
}

#[async_trait]
impl<S> Info for InfoConnection<S>
where
    S: ConsensusConnectionStorage,
{
    // Info will be called by tendermint on start
    // If we started with init-data flag, the state below will include the latest saved data
    async fn info(&self, _info_request: RequestInfo) -> ResponseInfo {
        let state = self.state.lock().expect("Failed to lock state for info");

        let state_data = StateData {
            eld_app_version: self.eld_app_version.clone(),
        };

        ResponseInfo {
            data: serde_json::to_string(&state_data).expect("Can serialize info data"),
            version: self.eld_app_version.clone(),
            app_version: Default::default(),
            last_block_height: state.envelope.block_height,
            last_block_app_hash: state.app_hash().to_vec(),
        }
    }

    async fn query(&self, query_request: RequestQuery) -> ResponseQuery {
        info!("path: {}", query_request.path.as_str());

        // Check global rate limit
        if let Some(error_response) = self.global_rate_limiter.check_rate_limit(&query_request) {
            error!("error_response: {:?}", error_response);
            return error_response;
        }

        // Check query-based rate limit
        if let Some(error_response) = self.query_rate_limiter.check_rate_limit(&query_request) {
            error!("error_response: {:?}", error_response);
            return error_response;
        }

        let path = query_request.path.clone();
        let key = query_request.data.clone();
        let query = EldAbciQuery::from_abci_query_request(query_request);
        info!("path.as_str(): {}", query.path());

        match query.process(self) {
            Ok(payload) => Self::to_query_response(path, key, payload),
            Err(err) => Self::map_query_error(path, key, err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_view_from_parts_uses_canonical_hex() {
        let addr =
            Address::parse_hex_str("1234567890123456789012345678901234567890").expect("address");
        let view = AccountView::from_parts(addr, 100, 3);
        assert_eq!(view.address, "0x1234567890123456789012345678901234567890");
        assert_eq!(view.balance, 100);
        assert_eq!(view.nonce, 3);

        let json = serde_json::to_string(&view).expect("serialize");
        assert!(json.contains("0x1234567890123456789012345678901234567890"));
    }

    #[test]
    fn state_data_serializes_eld_app_version_for_abci_info() {
        let data = StateData {
            eld_app_version: "0.0.39".to_string(),
        };
        let json = serde_json::to_string(&data).expect("serialize");
        assert_eq!(json, r#"{"eld_app_version":"0.0.39"}"#);
    }

    #[test]
    fn resolve_eld_app_version_is_non_empty() {
        let version = resolve_eld_app_version();
        assert!(!version.is_empty());
        // When ELD_APP_VERSION is unset in the test process, expect the default.
        if std::env::var("ELD_APP_VERSION").is_err() {
            assert_eq!(version, "0.0.0");
        }
    }
}
