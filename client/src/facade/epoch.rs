//! Validator set and epoch information queries.

use super::ChainClient;
use crate::api::abci::AbciHttpApi;
use eld_common::error::{EldError, ErrorBuilder};
use eld_common::validator::{ActiveValidatorsInfo, EpochInfo};

pub(crate) async fn view_active_validators(
    client: &ChainClient,
) -> Result<Option<ActiveValidatorsInfo>, EldError> {
    let api = AbciHttpApi::new(client.config.get_node_url()?)?;
    api.get_active_validators().await
}

pub(crate) async fn view_epoch_info(client: &ChainClient) -> Result<EpochInfo, EldError> {
    let api = AbciHttpApi::new(client.config.get_node_url()?)?;
    api.get_epoch_info()
        .await?
        .ok_or_else(|| ErrorBuilder::not_found_error("EpochInfo", "current"))
}

pub(crate) async fn view_epoch(
    client: &ChainClient,
) -> Result<(EpochInfo, ActiveValidatorsInfo), EldError> {
    let api = AbciHttpApi::new(client.config.get_node_url()?)?;

    let epoch_info_future = api.get_epoch_info();
    let active_validators_future = api.get_active_validators();

    let (epoch_info_result, active_validators_result) =
        tokio::join!(epoch_info_future, active_validators_future);

    let epoch_info =
        epoch_info_result?.ok_or_else(|| ErrorBuilder::not_found_error("EpochInfo", "current"))?;
    let active_validators = active_validators_result?
        .ok_or_else(|| ErrorBuilder::not_found_error("ActiveValidators", "current"))?;
    Ok((epoch_info, active_validators))
}
