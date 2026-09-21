# eld-client

Off-chain HTTP client, CWD JSON config, and wallet-file I/O for Eld nodes, CLI, and faucet.

This crate is **not** published to crates.io yet (`publish = false`). Protocol types live in [`eld-common`](https://github.com/eldnetwork/eld-chain/tree/main/common).

## Crate map

- `api::abci` — Tendermint RPC / ABCI (`AbciHttpApi`, queries, `broadcast_tx_commit`)
- `api::rest` — node app REST (`AppApi`, pinboard/namespace JSON DTOs) and the dev faucet
- `facade` — `ChainClient`, mixed command wrappers
- `config` — CWD JSON (`ClientConfig`, `ClientSetup`, `get_client_setup`, `WALLETS_PATH`)
- `logging` — sanitizers re-exported from `eld-common` (`init_default_logging` lives in `eld` binaries)
- `wallet_store_config` — `wallets.json` paths; identity types are `eld_common::wallet::Wallet`

Hex and ID conventions: [TYPE_DESIGN.md](TYPE_DESIGN.md).

## docs.rs / dependencies

This crate always depends on `tendermint-rpc` (HTTP client), `reqwest`, and `tokio` (runtime, time, macros). That makes docs.rs heavier than `eld-common`. HTTP is not feature-gated yet so `eld` can keep a single path dependency.

## Usage

Path-depend from a workspace sibling (this repo):

```toml
eld_common = { path = "../common", package = "eld-common" }
eld_client = { path = "../client", package = "eld-client" }
```

The `eld` node, CLI, and faucet use:

```toml
eld_common = { path = "../../../eld-chain/common", package = "eld-common" }
eld_client = { path = "../../../eld-chain/client", package = "eld-client" }
```

```rust
use eld_client::ChainClient;

fn _holds(client: ChainClient) -> ChainClient {
    client
}
```

### Config file

Loaders read JSON from the process CWD (default: `config/config.json`). Copy [config/config.json.example](config/config.json.example) and adjust endpoints for your node.

`ClientConfig` uses these fields (node-only keys such as `p2p_tcp_port` or `indexer` may appear in the same file when shared with a node binary; the client library ignores them):

| Field | Purpose |
|-------|---------|
| `node_host`, `node_port` | Tendermint RPC when `node_url` is unset |
| `node_url` | Optional full RPC base URL (overrides host/port) |
| `app_port` | Node app REST port when `app_url` is unset |
| `app_url` | Optional full app REST base URL |
| `faucet_host`, `faucet_port`, `faucet_end_point` | Faucet when `faucet_url` is unset |
| `faucet_url` | Optional full faucet base URL |
| `chain_id` | Chain ID (often filled from `consensus_config.json` by binaries) |

For local dev, omit the `*_url` fields and use loopback host/port values. Wallet files hold unencrypted Ed25519 keys; see the workspace [SECURITY.md](https://github.com/eldnetwork/eld-chain/blob/main/SECURITY.md).

Rust imports use the underscore crate name `eld_client`.

## License

MIT. Copyright Eld network. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
