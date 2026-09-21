# eld-client

Types and helpers for talking to an [Eld](https://github.com/eldnetwork/eld-chain) node over its two HTTP surfaces: Tendermint RPC (ABCI) and the node app REST API.

On-chain domain types (`Account`, `Tx`, addresses, fees) live in [`eld-common`](../common/README.md). This crate adds wire clients, JSON DTOs, config loading, wallet file I/O, and a high-level `ChainClient`. Experimental; not on crates.io yet (`publish = false`).

## Modules

| Module | Main types | Role |
|---|---|---|
| `api::abci` | `AbciHttpApi` | Tendermint JSON-RPC — `abci_query`, blocks, tx search, CADO/pinboard queries |
| | `tx_broadcast` | `broadcast_tx_commit`, deliver-tx event parsing |
| `api::rest` | `AppApi` | Node app REST — CADO, pinboard submit, namespace lookup, health |
| | `faucet` | Dev faucet HTTP (test funds) |
| | `pinboard`, `namespace` | Request/response DTOs for REST endpoints |
| `facade` | `ChainClient` | ABCI + REST + wallets — accounts, transfers, stake, pinboard, namespaces, CADO |
| | `SubmittedTx`, `NamespaceLookup` | Typed results from facade calls |

- **`api::*`** — direct HTTP; pick ABCI or REST per call.
- **`facade::ChainClient`** — one client that signs transactions and reads `wallets.json`.

### Supporting modules

| Module | Purpose |
|---|---|
| `config` | `ClientConfig`, `ClientSetup`, CWD JSON loading |
| `wallet_store_config` | Paths and I/O for `wallets.json` |

## Add to your project

From the same workspace as this repo:

```toml
eld_common = { path = "../common", package = "eld-common" }
eld_client = { path = "../client", package = "eld-client" }
```

From git:

```toml
eld_common = { git = "https://github.com/eldnetwork/eld-chain", package = "eld-common" }
eld_client = { git = "https://github.com/eldnetwork/eld-chain", package = "eld-client" }
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
|---|---|
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

Report security issues via the workspace [SECURITY.md](../SECURITY.md).

Hex and ID conventions: [`eld-common` TYPE_DESIGN](../common/TYPE_DESIGN.md).

## Documentation

| File | Purpose |
|---|---|
| [CHANGELOG.md](CHANGELOG.md) | Crate release notes |
| [TYPE_DESIGN.md](TYPE_DESIGN.md) | Pointer to `eld-common` hex/ID rules |
| [config/config.json.example](config/config.json.example) | Sample client config |
| [examples/](examples/) | `query_account`, `broadcast_transfer` |

## Dependencies

Always-on: `tendermint-rpc`, `reqwest`, `tokio`. Not feature-gated yet so downstream workspaces keep a single path dependency.

## License

MIT. Copyright Eld network. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
