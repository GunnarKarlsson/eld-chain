# Eld chain

Protocol types (`eld-common`), off-chain client helpers (`eld-client`), and a parallel copy of the ABCI node (`eld_node_app`) for the Eld blockchain.

The library crate directories ship `LICENSE`, `README.md`, `NOTICE`, and `TYPE_DESIGN.md` so a future crates.io/docs.rs package is self-contained. Crates here are not published (`publish = false`). See [CONTRIBUTING.md](CONTRIBUTING.md).

The node binary used by Docker and local deploy scripts still lives in the sibling `eld` repo (`eld/chain/node_app`) and path-depends on `eld-common` / `eld-client`. This workspace also contains a **copy** of that crate under [`node_app/`](node_app/) (`eld_node_app`) so the node can compile here; it is not a cutover.

## Architecture

[`eld-common`](common/README.md) (directory `common/`) is the protocol library: addresses, coins, nonces, transactions and payloads, CADO paths, capacity-proof types (including on-disk `SlotAllocator`), pinboard and namespace types, signing `Wallet` identity, validation of those types, constants, and errors. Hex and ID conventions are in [TYPE_DESIGN.md](TYPE_DESIGN.md).

[`eld-client`](client/README.md) (directory `client/`) is the off-chain process library:

- `api::abci` — Tendermint RPC / ABCI (`AbciHttpApi`, queries, `broadcast_tx_commit`)
- `api::rest` — node app REST (`AppApi` plus pinboard/namespace JSON DTOs) and the dev faucet
- `facade` — `ChainClient` and command wrappers that may use both stacks
- `config` — CWD JSON (`client_config`, `config_loader`); `wallets.json` I/O at the crate root

Config loaders return `Result`; the CLI can exit after it sees an error.

[`eld_node_app`](node_app/README.md) (directory `node_app/`) is the ABCI application (Tendermint, RocksDB, libp2p, Axum). Package name stays `eld_node_app`. Runtime data (`data/`, `tx_responses/`), wallets, and P2P key files are not shipped; copy those from `eld/chain/node_app` when running this copy locally.

Rust imports use underscores (`eld_common`, `eld_client`) because Cargo package names may contain hyphens.

CosmWasm / on-chain WASM contracts are not part of this repo.

Intended later binaries in this repo:

- `eld-cli` — thin clap front-end over `eld-client` (binary name `eld`)
- `eld-faucet` — app, `publish = false` (the node crate here is still named `eld_node_app`, not `eld-node`)

## Encoding

v0 uses three codecs. This is the current client/node map, not a frozen spec. Hex display rules for IDs are in [TYPE_DESIGN.md](TYPE_DESIGN.md).

| Codec | Edges |
|---|---|
| **serde_json** | Transaction body and Ed25519 signing (`serde_json` of the tx with an empty sig, then append `chain_id`). Mempool and block bytes are UTF-8 hex of that JSON. Same codec for Tendermint / app / faucet HTTP, CLI config, wallets, and on-disk slot maps. |
| **bincode** | CADO payload bytes (`Account`, staking accounts, `EpochRecord`, `NamespaceRecord`, CADO envelope). The node also uses bincode for GossipSub `SyncMsg` and persisted pinboard metadata. |
| **parity-scale-codec** | `Coin` only, leftover from the Cardano-adapted type. Not an Eld wire format; JSON and bincode go through serde. |

A later canonical transaction encoding would be a breaking change.

## Overview

- Account addresses, transactions, signatures, and `Wallet` identity (Ed25519) — `eld-common`
- CADO paths, capacity proofs, pinboard and namespace types — `eld-common`
- HTTP helpers for Tendermint RPC, the app API, and a dev faucet — `eld-client`
- Local JSON wallet **files** (plaintext hex keys; see [SECURITY.md](SECURITY.md)) — `eld-client`
- ABCI node process — `eld_node_app` (parallel copy; deploy still uses `eld/chain/node_app`)

## Setup

Rust 1.88.0 (see `rust-toolchain.toml`). Install [gitleaks](https://github.com/gitleaks/gitleaks) for secret scanning (`brew install gitleaks` on macOS).

```sh
./deploy/scripts/ci.sh
```

That runs the same checks as GitHub Actions: `cargo fmt --check`, Clippy, build, test, and gitleaks. Rustc and Clippy warnings are treated as errors.

## Configuration

CLI and node binaries read endpoint and runtime settings from `config/config.json` (client fields are used by `eld-client`; the node also reads P2P, capacity, and indexer fields from the same file). Wallet files (`wallets/wallets.json`) hold unencrypted Ed25519 private keys; do not commit them. Sample non-secret configs live under `node_app/config/`; do not add `p2p_keypair.json` or `wallets.json`.

Protocol constants in `eld_common::constants::protocol` (minimum stake, validators per epoch, blocks per epoch, block reward) are local-dev values, not mainnet economics.

## Usage

Path-depend from a workspace sibling:

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
use eld_common::Address;

let address = Address::parse_hex_str("0x1234567890abcdef1234567890abcdef12345678")?;
```

## License

MIT. Copyright Eld network.

`common/src/coin.rs` is adapted from IOHK rust-cardano (MIT) and Crypto.com (Apache-2.0). See [NOTICE](NOTICE) and the file header. This crate does not ship CosmWasm / CW20 bytecode.
