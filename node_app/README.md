# eld_node_app

ABCI application for the Eld blockchain: Tendermint integration, RocksDB state, libp2p content sync, and Axum REST APIs.

Runtime secrets and data are **not** shipped in git: `wallets/`, `p2p_keypair.json`, and `data/` are local only.

## Dependencies

| Crate | Role |
|---|---|
| [`eld-common`](../common/README.md) | Protocol types, validation, signing |
| [`eld-client`](../client/README.md) | Shared HTTP/config types; some internal reuse |

Package name: `eld_node_app` (`publish = false`). Rust import: `eld_node_app`.

## Configuration

Sample configs live under [`config/`](config/):

| File | Purpose |
|---|---|
| `config.json` | RPC/app URLs, P2P, capacity, indexer (client fields overlap with `eld-client::ClientConfig`) |
| `consensus_config.json` | Chain ID, fee settings, validator params |
| `storage_config.json` | RocksDB paths and retention |

The node loads client and runtime sections from one `config/config.json` via `AppConfig` (see `src/config/app_config.rs`). Provide `wallets/wallets.json` and `config/p2p_keypair.json` locally before running; for the Docker cluster, see [deploy/README.md](../deploy/README.md#secrets-not-committed).

Do not commit `wallets.json`, `p2p_keypair.json`, or Tendermint validator keys.

## Build and test

From the repo root (same gate as CI):

```sh
./deploy/scripts/ci.sh
```

Build only this crate:

```sh
cargo build -p eld_node_app
cargo test -p eld_node_app
```

Run the binary against local Tendermint (after configs and secrets are in place):

```sh
cargo run -p eld_node_app
```

Query accounts and submit transfers with [`eld-client`](../client/README.md) examples or your own binary built on `ChainClient`.

## Local multi-node cluster

For four nodes on one machine, see [`deploy/README.md`](../deploy/README.md): `deploy/compose.yaml`, image build scripts, and `start-with-history.sh` / `start-without-history.sh`.

## Tendermint sync cases

When restarting Eld and Tendermint with persisted data:

1. **Both clean** — chain starts at height 0.
2. **Eld clean, TM has blocks** — Tendermint replays; ABCI handlers must be deterministic (e.g. `BTreeMap` over `HashMap` where ordering matters).
3. **Both have data, TM height ≥ Eld height** — normal resume.
4. **Both have data, TM height < Eld height** — unrecoverable; wipe Eld app data and resync.

Tendermint exposes current height and app hash via RPC `status` and ABCI `Info`.

## License

MIT. Copyright Eld network. See workspace [LICENSE](../LICENSE) and [NOTICE](../NOTICE).
