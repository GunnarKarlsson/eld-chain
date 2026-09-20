# eld-client

Off-chain HTTP client, CWD JSON config, and wallet-file I/O for Eld nodes, CLI, and faucet.

This crate is **not** published to crates.io yet (`publish = false`). Protocol types live in [`eld-common`](https://github.com/eldnetwork/eld-chain/tree/main/common).

## Crate map

- `api::abci` — Tendermint RPC / ABCI (`AbciHttpApi`, queries, `broadcast_tx_commit`)
- `api::rest` — node app REST (`AppApi`, pinboard/namespace JSON DTOs) and the dev faucet
- `facade` — `ChainClient`, `facade::cli` (`Cli` alias), mixed command wrappers
- `config` — CWD JSON (`client_config`, `config_loader`)
- `logging` — `init_default_logging` (re-exports sanitizers from `eld-common`)
- `wallet_store_config` — `wallets.json` paths; identity types are `eld_common::wallet::Wallet`

Hex and ID conventions: [TYPE_DESIGN.md](TYPE_DESIGN.md). `Cli` is a type alias for `ChainClient`.

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
use eld_client::facade::cli::Cli;

fn _same_type(c: ChainClient) -> Cli {
    c
}
```

Config loaders look for JSON under the process CWD (for example `config/config.json`). Those files are not shipped in this crate. Wallet files hold unencrypted Ed25519 keys; see the workspace [SECURITY.md](https://github.com/eldnetwork/eld-chain/blob/main/SECURITY.md).

Rust imports use the underscore crate name `eld_client`.

## License

MIT. Copyright Eld network. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
