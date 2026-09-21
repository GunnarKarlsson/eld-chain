//! CADO path listing and CADO fetch.

use super::ChainClient;
use crate::api::abci::AbciHttpApi;
use eld_common::error::EldError;

pub(crate) async fn get_cado(
    client: &ChainClient,
    path: String,
) -> Result<serde_json::Value, EldError> {
    let api = AbciHttpApi::new(client.config.get_node_url()?)?;
    api.get_cado(path).await
}

pub(crate) async fn list_cados(
    client: &ChainClient,
    search_string: String,
) -> Result<Vec<String>, EldError> {
    let api = AbciHttpApi::new(client.config.get_node_url()?)?;
    api.get_cado_paths(search_string).await
}
