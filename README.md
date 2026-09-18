# Eld chain

Shared types, transaction wire format, validation, and client helpers for the Eld blockchain.

This workspace currently has one crate, `eld_common`. The node, CLI, and contracts will be added later as sibling crates. The crate is experimental and is not published to crates.io (`publish = false`).

## Architecture

`eld_common` is the protocol library: addresses, coins, nonces, transactions and payloads, CADO paths, capacity-proof types, pinboard and namespace types, validation of those types, constants, and errors. Hex and ID conventions are in [TYPE_DESIGN.md](TYPE_DESIGN.md).

The JSON transaction signing encoding (`serde_json` of the tx plus `chain_id`) is the current client/node wire format, not a frozen spec. A later canonical encoding would be a breaking change.

HTTP RPC helpers, CLI command flows, process-wide logging setup, config loaders that read `config/*.json` from the working directory, and on-disk capacity slot files are local operator/node code. They live in this crate today because the node and CLI are not here yet. They are not a frozen public API. A library function should not call `std::process::exit`; that will move to the CLI. CosmWasm / on-chain contract execution is not part of this crate.

Intended split once sibling crates exist:

- `eld_common` — protocol types and tx codec
- a client crate or Cargo feature — Tendermint / app HTTP helpers
- `eld-cli` — command flows, wallet files, config load from CWD
- the node — ABCI, slot files, consensus config

## Overview

`eld_common` is the library used by Eld nodes and clients. It includes:

- Account addresses, transactions, and signatures (Ed25519)
- CADO paths, capacity proofs, pinboard and namespace types
- HTTP helpers for Tendermint RPC, the app API, and a dev faucet
- Local JSON wallet loading (plaintext hex keys; see [SECURITY.md](SECURITY.md))

## Setup

Rust 1.88.0 (see `rust-toolchain.toml`). Install [gitleaks](https://github.com/gitleaks/gitleaks) for secret scanning (`brew install gitleaks` on macOS).

```sh
./scripts/ci.sh
```

That runs the same checks as GitHub Actions: `cargo fmt --check`, Clippy, build, test, and gitleaks. Rustc and Clippy warnings are treated as errors.

## Configuration

CLI/node helpers read JSON config from paths such as `config/config.json` and `wallets/wallets.json`. Those files are not shipped here. Wallet files hold unencrypted Ed25519 private keys; do not commit them.

Protocol constants in `eld_common::constants::protocol` (minimum stake, validators per epoch, blocks per epoch) are local-dev values, not mainnet economics.

## Usage

Depend on the crate by path while it lives in this workspace:

```toml
eld_common = { path = "../common" }
```

```rust
use eld_common::Address;

let address = Address::parse_hex_str("0x1234567890abcdef1234567890abcdef12345678")?;
```

## License

MIT. Copyright Eld network. Third-party notices are in [NOTICE](NOTICE).
