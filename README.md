# Eld chain

Shared types, transaction wire format, validation, and client helpers for the Eld blockchain.

This repository currently publishes the `eld_common` crate only. The node (`node_app`), contracts, and CLI will be added later as sibling crates in this workspace.

`eld_common` is protocol types and the transaction codec, plus HTTP helpers used by local tooling. Config loaders that read `config/config.json` from the working directory are operator conveniences, not part of a frozen public API. The crate is experimental.

## Overview

`eld_common` is the library used by Eld nodes and clients. It includes:

- Account addresses, transactions, and signatures (Ed25519)
- CADO paths, capacity proofs, pinboard and namespace types
- HTTP helpers for Tendermint RPC, the app API, and a dev faucet
- Local JSON wallet loading (plaintext hex keys; see [SECURITY.md](SECURITY.md))

Hex and ID conventions are in [TYPE_DESIGN.md](TYPE_DESIGN.md). The crate is not published to crates.io (`publish = false`). CosmWasm / on-chain contract execution is not part of this crate.

## Setup

Rust 1.88.0 (see `rust-toolchain.toml`).

```sh
cargo build
cargo test
cargo clippy --all-targets --all-features
```

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
