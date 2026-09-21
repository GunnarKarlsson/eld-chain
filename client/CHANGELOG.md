# Changelog

All notable changes to `eld-client` are documented here.

## [0.1.0] - 2026-03-21

First experimental release as a library crate inside `eld-chain` (not published to crates.io).

### Added

- `AbciHttpApi` and `AppApi` for Tendermint RPC and node app REST.
- `ChainClient` facade: accounts, staking, transfers, pinboard, namespaces, CADO, wallet file I/O.
- `ClientConfig` / `ClientSetup` and CWD JSON loading; `config/config.json.example`.
- `SubmittedTx` with typed `tx_hash` from `broadcast_tx_commit`.
- Examples: `query_account`, `broadcast_transfer`.

### Changed

- Renamed `CliConfig` → `ClientConfig`; node runtime fields moved to `node_app::NodeRuntimeConfig`.
- Config loader is crate-private; node uses its own `config/loader.rs`.
- Library constructors require explicit `FeeConfig` and optional wallet path (`with_wallets`).
