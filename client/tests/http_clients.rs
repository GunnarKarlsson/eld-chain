//! HTTP integration tests for ABCI, app REST, and `ChainClient` (wiremock).

use eld_client::api::abci::AbciHttpApi;
use eld_client::api::rest::faucet::request_faucet;
use eld_client::api::rest::AppApi;
use eld_client::config::ClientConfig;
use eld_client::ChainClient;
use eld_common::fee::FeeConfig;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn mock_client_config(rpc_uri: &str, app_uri: &str, faucet_uri: &str) -> ClientConfig {
    ClientConfig {
        node_host: "127.0.0.1".into(),
        node_port: "26657".into(),
        faucet_host: "127.0.0.1".into(),
        faucet_port: "8080".into(),
        faucet_end_point: "/faucet/request".into(),
        faucet_url: Some(format!("{faucet_uri}/")),
        app_port: "9001".into(),
        node_url: Some(format!("{rpc_uri}/")),
        app_url: Some(format!("{app_uri}/")),
        chain_id: "test-chain".into(),
    }
}

fn tendermint_abci_info_response(app_version: u64, height: u64) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": -1,
        "result": {
            "response": {
                "data": "",
                "version": "0.40.0",
                "app_version": app_version.to_string(),
                "last_block_height": height.to_string(),
                "last_block_app_hash": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
            }
        }
    })
}

fn tendermint_abci_query_empty(height: u64) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": -1,
        "result": {
            "response": {
                "code": 0,
                "log": "",
                "info": "",
                "index": "0",
                "key": "",
                "value": "",
                "proofOps": null,
                "height": height.to_string(),
                "codespace": ""
            }
        }
    })
}

#[tokio::test]
async fn abci_http_api_fetches_latest_info() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("abci_info"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(tendermint_abci_info_response(7, 42)),
        )
        .mount(&server)
        .await;

    let api = AbciHttpApi::new(format!("{}/", server.uri())).unwrap();
    let info = api.get_latest_abci_info().await.unwrap();

    assert_eq!(info.app_version, 7);
    assert_eq!(info.last_block_height.value(), 42);
    assert_eq!(info.version, "0.40.0");
}

#[tokio::test]
async fn abci_http_api_returns_none_for_missing_account() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("abci_query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tendermint_abci_query_empty(10)))
        .mount(&server)
        .await;

    let api = AbciHttpApi::new(format!("{}/", server.uri())).unwrap();
    let account = api
        .get_account_by_address("0x1234567890123456789012345678901234567890")
        .await
        .unwrap();

    assert!(account.is_none());
}

#[tokio::test]
async fn app_api_get_namespace_registered_and_missing() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/namespace/peter"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "registered": true,
            "namespace_slug": "peter",
            "scope": "@peter",
            "owner": "0xe17404c417fa10cc04fdf73604fcacca8d0a687c",
            "registered_height": 42,
            "registry_path": "cado/account/0xe17404c417fa10cc04fdf73604fcacca8d0a687c"
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v1/namespace/missing"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "registered": false,
            "namespace_slug": "missing"
        })))
        .mount(&server)
        .await;

    let api = AppApi::new(format!("{}/", server.uri())).unwrap();

    let registered = api.get_namespace("peter").await.unwrap();
    assert!(registered.is_some());
    assert_eq!(registered.unwrap().namespace_slug, "peter");

    let missing = api.get_namespace("missing").await.unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn request_faucet_posts_address_and_returns_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/faucet/request"))
        .and(body_string_contains(
            "0x1234567890123456789012345678901234567890",
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"success":true,"message":"tx submit"}"#),
        )
        .mount(&server)
        .await;

    let config = mock_client_config(&server.uri(), &server.uri(), &server.uri());
    let body = request_faucet(&config, "0x1234567890123456789012345678901234567890".into())
        .await
        .unwrap();

    assert!(body.contains("success"));
}

#[tokio::test]
async fn chain_client_get_abci_info_and_account() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(body_string_contains("abci_info"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(tendermint_abci_info_response(1, 99)),
        )
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(body_string_contains("abci_query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tendermint_abci_query_empty(99)))
        .mount(&server)
        .await;

    let config = mock_client_config(&server.uri(), &server.uri(), &server.uri());
    let client = ChainClient::new(config, FeeConfig::default());

    let info = client.get_abci_info().await.unwrap();
    assert_eq!(info.last_block_height.value(), 99);

    let account = client
        .get_account("0x1234567890123456789012345678901234567890".into())
        .await
        .unwrap();
    assert!(account.is_none());
}

#[tokio::test]
async fn chain_client_get_namespace_via_rest() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/namespace/demo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "registered": true,
            "namespace_slug": "demo",
            "scope": "@demo",
            "owner": "0xe17404c417fa10cc04fdf73604fcacca8d0a687c",
            "registered_height": 1,
            "registry_path": "cado/demo"
        })))
        .mount(&server)
        .await;

    let config = mock_client_config(&server.uri(), &server.uri(), &server.uri());
    let client = ChainClient::new(config, FeeConfig::default());

    let lookup = client.get_namespace("demo".into()).await.unwrap();
    assert_eq!(lookup.canonical_slug, "demo");
    assert!(lookup.registered.is_some());
}
