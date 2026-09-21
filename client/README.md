# eld-client

HTTP and RPC client library for [Eld](https://github.com/eldnetwork/eld-chain) nodes: query chain state over Tendermint RPC / ABCI, call the node app REST API (pinboard, namespaces, CADO), sign transactions from local wallet files, and broadcast via `broadcast_tx_commit`.

Protocol types (`Account`, `Tx`, addresses, fees) live in [`eld-common`](https://github.com/eldnetwork/eld-chain/tree/main/common). This crate is experimental and not on crates.io yet (`publish = false`).

## Add to your project

From the same workspace as this repo:

```toml
eld_common = { path = "../common", package = "eld-common" }
eld_client = { path = "../client", package = "eld-client" }
```

From a sibling checkout (as the `eld` monorepo does):

```toml
eld_common = { path = "../../../eld-chain/common", package = "eld-common" }
eld_client = { path = "../../../eld-chain/client", package = "eld-client" }
```

Rust imports use the underscore crate name: `eld_client`.

## Quick start

Point at a running node with `config/config.json` (copy [config/config.json.example](config/config.json.example)), then query chain state or submit a transfer:

```rust,no_run
use eld_client::api::abci::AbciHttpApi;
use eld_client::config::{get_client_setup, ClientConfig, WALLETS_PATH};
use eld_client::ChainClient;
use eld_common::fee::FeeConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Low-level RPC: Tendermint / ABCI queries
    let config = ClientConfig::from_file("config/config.json")?;
    let abci = AbciHttpApi::new(config.get_node_url()?)?;
    let info = abci.get_latest_abci_info().await?;
    println!("block height {}", info.last_block_height);

    // High-level facade: account lookup + signed transfer (needs wallets.json)
    let setup = get_client_setup()?;
    let client = ChainClient::with_wallets(setup.config, setup.fee_config, WALLETS_PATH)?;
    if let Some(account) = client
        .get_account("0x1234567890123456789012345678901234567890".into())
        .await?
    {
        println!("balance={}", account.balance().amount());
    }
    let submitted = client
        .transfer("my-wallet".into(), "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd".into(), 1_000)
        .await?;
    println!("committed tx {}", submitted.tx_hash);

    Ok(())
}
```

Runnable examples (from a directory with `config/config.json`):

```sh
cargo run -p eld-client --example query_account -- 0xYourAddress
cargo run -p eld-client --example broadcast_transfer -- my-wallet 0xRecipient 1000
```

See [`examples/`](examples/) for full source.

## Config file

Default path: `config/config.json`. Node binaries may share this file; keys such as `p2p_tcp_port` or `indexer` are ignored by the client library.

| Field | Purpose |
|-------|---------|
| `node_host`, `node_port` | Tendermint RPC when `node_url` is unset |
| `node_url` | Optional full RPC base URL (overrides host/port) |
| `app_port` | Node app REST port when `app_url` is unset |
| `app_url` | Optional full app REST base URL |
| `faucet_host`, `faucet_port`, `faucet_end_point` | Faucet when `faucet_url` is unset |
| `faucet_url` | Optional full faucet base URL |
| `chain_id` | Chain ID (often merged from `consensus_config.json` by binaries) |

Fee settings for signing live in `config/consensus_config.json` (`ClientSetup` / `get_client_setup` load both files).

## Wallets and security

Local wallets are **plaintext JSON** files (`wallets/wallets.json` by default) containing hex-encoded Ed25519 **private keys**. There is no encryption at rest.

- Do **not** commit wallet files or any JSON containing `private_key`.
- Do **not** log serialized wallets or signed transaction JSON in production.
- Treat any key from tests or examples as compromised once published.

On Unix only, the library sets wallet files to mode `0600` when writing. On Windows, restrict access to the wallet directory yourself.

Report security issues via [GitHub Security Advisories](https://github.com/eldnetwork/eld-chain/security/advisories/new) or email the maintainer (see workspace [SECURITY.md](https://github.com/eldnetwork/eld-chain/blob/main/SECURITY.md)).

## Crate map

- `api::abci` — `AbciHttpApi`, ABCI queries, `broadcast_tx_commit`
- `api::rest` — `AppApi`, pinboard/namespace DTOs, dev faucet HTTP
- `facade` — `ChainClient`, `SubmittedTx`, command-style helpers
- `config` — `ClientConfig`, `ClientSetup`, CWD JSON loaders
- `wallet_store_config` — paths and I/O for `wallets.json`

Hex and ID conventions: [`eld-common` TYPE_DESIGN](../common/TYPE_DESIGN.md).

## Dependencies

Always-on: `tendermint-rpc`, `reqwest`, `tokio`. Not feature-gated yet so downstream workspaces keep a single path dependency.

## License

MIT. Copyright Eld network. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
