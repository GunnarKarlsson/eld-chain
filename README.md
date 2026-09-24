# Eld chain

![Rust](https://img.shields.io/badge/rust-1.88.0-orange?logo=rust)
![Linux](https://img.shields.io/badge/platform-Linux-black?logo=linux)
![macOS](https://img.shields.io/badge/platform-macOS-black?logo=apple)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![CI](https://github.com/eldnetwork/eld-chain/actions/workflows/ci.yml/badge.svg)](https://github.com/eldnetwork/eld-chain/actions/workflows/ci.yml)
[![Stars](https://img.shields.io/github/stars/eldnetwork/eld-chain)](https://github.com/eldnetwork/eld-chain/stargazers)

Eld is an L1 for ephemeral, content-addressed storage: data is addressed, kept only for a TTL, then expires by protocol.

This repository is the chain implementation — protocol types (eld-common), off-chain client (eld-client), and the Tendermint ABCI node (eld-node). It is not the website, docs site, explorer, or CosmWasm contracts.

| Want | Go here |
| --- | --- |
| Protocol narrative, CLI, capacity providers | docs.eld.network |
| Marketing / product overview | www.eld.network |
| Live chain UI | explorer.eld.network |
| This codebase (node, client, types) | this repo |

Testnet is live; crate versions here are unpublished (publish = false) and protocol constants are local-dev values, not mainnet economics.

## This repository

Protocol types (`eld-common`), off-chain client helpers (`eld-client`), and the ABCI node application (`eld-node`) for the Eld blockchain.

Library crates ship `LICENSE`, `README.md`, `NOTICE`, and (where relevant) `CHANGELOG.md` and `TYPE_DESIGN.md` so a future crates.io/docs.rs package is self-contained. Crates here are not published yet (`publish = false`).

## Crates

| Directory | Package | Role |
|---|---|---|
| [`common/`](common/README.md) | `eld-common` | Protocol types, validation, `Wallet` identity, CADO, capacity, pinboard |
| [`client/`](client/README.md) | `eld-client` | Tendermint RPC, app REST, faucet HTTP, `ChainClient`, CWD config, wallet files |
| [`node_app/`](node_app/README.md) | `eld-node` | ABCI application (Tendermint, RocksDB, libp2p, Axum REST) |

Library crate imports use underscores (`eld_common`, `eld_client`) because Cargo package names may contain hyphens. `eld-node` is a binary crate (`eld-node`), not a library.


## Architecture

[`eld-common`](common/README.md) is the protocol library: addresses, coins, nonces, transactions and payloads, CADO paths, capacity-proof types (including on-disk `SlotAllocator`), pinboard and namespace types, signing `Wallet` identity, validation, constants, and errors.

[`eld-client`](client/README.md) is the off-chain process library:

- `api::abci` — Tendermint RPC / ABCI (`AbciHttpApi`, queries, `broadcast_tx_commit`)
- `api::rest` — node app REST (`AppApi` plus pinboard/namespace JSON DTOs) and the dev faucet
- `facade` — `ChainClient` and command wrappers that may use both stacks
- `config` — CWD JSON (`ClientConfig`, `ClientSetup`); `wallets.json` I/O at the crate root

Config loaders return `Result`; binaries can exit after they see an error.

[`eld-node`](node_app/README.md) is the ABCI application. Runtime data (`data/`, `tx_responses/`), wallets, and P2P key files are not shipped in git — create them locally or copy from [`deploy/host/single/config/`](deploy/host/single/config/) samples before running.

Hex and ID conventions: [common/TYPE_DESIGN.md](common/TYPE_DESIGN.md) (canonical; workspace root [TYPE_DESIGN.md](TYPE_DESIGN.md) points there).

## Encoding

v0 uses three codecs. This is the current client/node map, not a frozen spec.

| Codec | Edges |
|---|---|
| **serde_json** | Transaction body and Ed25519 signing (`serde_json` of the tx with an empty sig, then append `chain_id`). Mempool and block bytes are UTF-8 hex of that JSON. Same codec for Tendermint / app / faucet HTTP, CLI config, wallets, and on-disk slot maps. |
| **bincode** | CADO payload bytes (`Account`, staking accounts, `EpochRecord`, `NamespaceRecord`, CADO envelope). The node also uses bincode for GossipSub `SyncMsg` and persisted pinboard metadata. |
| **parity-scale-codec** | `Coin` only, leftover from the Cardano-adapted type. Not an Eld wire format; JSON and bincode go through serde. |

A later canonical transaction encoding would be a breaking change.

## Repository documentation

| File | Purpose |
|---|---|
| [CONTRIBUTING.md](CONTRIBUTING.md) | PR workflow, layout, CI |
| [SECURITY.md](SECURITY.md) | Vulnerability reporting, wallet hygiene |
| [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) | Community standards |
| [deploy/README.md](deploy/README.md) | Local four-node Docker Compose and CI scripts |
| [common/TYPE_DESIGN.md](common/TYPE_DESIGN.md) | Hex and typed ID conventions |

## Setup

Rust 1.88.0 (see `rust-toolchain.toml`). Install [gitleaks](https://github.com/gitleaks/gitleaks) for secret scanning (`brew install gitleaks` on macOS).

```sh
./deploy/scripts/ci.sh
```

That runs the same checks as GitHub Actions: `cargo fmt --check`, Clippy, build, test, and gitleaks. Rustc and Clippy warnings are treated as errors.

## Configuration

Binaries read endpoint and runtime settings from `config/config.json` (client fields are used by `eld-client`; the node also reads P2P, capacity, and indexer fields from the same file). See [`client/config/config.json.example`](client/config/config.json.example) for client fields.

Wallet files (`wallets/wallets.json`) hold unencrypted Ed25519 private keys; do not commit them. Sample non-secret configs live under `deploy/host/single/config/`; do not add `p2p_keypair.json` or `wallets.json`.

Protocol constants in `eld_common::constants::protocol` (minimum stake, validators per epoch, blocks per epoch, block reward) are local-dev values, not mainnet economics.

## Usage

Path-depend from another crate in your workspace:

```toml
eld_common = { path = "../common", package = "eld-common" }
eld_client = { path = "../client", package = "eld-client" }
```

Or depend on this repository with git:

```toml
eld_common = { git = "https://github.com/eldnetwork/eld-chain", package = "eld-common" }
eld_client = { git = "https://github.com/eldnetwork/eld-chain", package = "eld-client" }
```

```rust
use eld_common::Address;

let address = Address::parse_hex_str("0x1234567890abcdef1234567890abcdef12345678")?;
```

## License

MIT. Copyright Eld network.

`common/src/coin.rs` is adapted from IOHK rust-cardano (MIT) and Crypto.com (Apache-2.0). See [NOTICE](NOTICE) and the file header. This crate does not ship CosmWasm / CW20 bytecode.
