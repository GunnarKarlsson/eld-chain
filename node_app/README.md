# eld-node

`eld-node` is the process that runs **next to Tendermint** on an Eld node. It is a binary (`publish = false`), not a library.

This process owns:

- **ABCI** — deterministic state machine Tendermint drives (consensus / mempool / info / snapshot)
- **REST** — Axum explorer and upload API (not consensus)
- **libp2p** — off-chain blobs, capacity challenges, and content sync
- **RocksDB** — committed app state

Tendermint still owns **consensus**, **block gossip**, and **RPC** (default `26657`: `status`, `broadcast_tx_*`, `abci_query`). `eld-node` signs some provider txs as a client of that local RPC (`ChainClient` + `tendermint_rpc_url` from `node_host`/`node_port`).

Runtime secrets and data are **not** shipped in git: `wallets/`, `p2p_keypair.json`, and `data/` are local only.

Unix-oriented (file permissions, `statvfs`); intended to run on Linux or macOS next to Tendermint.

## Dependencies

| Crate | Role |
|---|---|
| [`eld-common`](../common/README.md) | Protocol types, validation, signing |
| [`eld-client`](../client/README.md) | HTTP/config types; the node also uses `ChainClient` against local Tendermint RPC |

Package name: `eld-node`. Binary: `eld-node`.

## Ports (single-node sample)

| Surface | Config | Sample |
|---|---|---|
| ABCI (Tendermint → app) | `consensus_config.json` `app_host` / `app_port` | `0.0.0.0:26658` |
| REST | `config.json` `app_port` | `9001` |
| libp2p TCP / UDP | `config.json` `p2p_tcp_port` / `p2p_udp_port` | `4001` / `4002` |
| Tendermint RPC | `config.json` `node_host` / `node_port` | `127.0.0.1:26657` |

Compose remaps per-node ABCI and host REST/RPC; see [`deploy/README.md`](../deploy/README.md#ports-host).

## Required local files

| Path | Purpose |
|---|---|
| `config/config.json` | RPC/REST URLs, P2P ports, capacity, indexer (`AppConfig`: client + node runtime) |
| `config/consensus_config.json` | Chain ID, ABCI bind, fees, genesis accounts (override with `ELD_CONSENSUS_CONFIG_PATH` or `--config-path`) |
| `wallets/wallets.json` | Local signing keys (gitignored) |
| `config/p2p_keypair.json` | libp2p identity (gitignored) |

RocksDB lives under `ELD_DB_PATH` or `--db-path` (default `./data/rocksdb`).

Do not commit `wallets.json`, `p2p_keypair.json`, or Tendermint validator keys.

## Environment

| Variable | Purpose |
|---|---|
| `ELD_DB_PATH` | RocksDB directory (default `./data/rocksdb`) |
| `ELD_CONSENSUS_CONFIG_PATH` | Consensus JSON path (default `./config/consensus_config.json`) |
| `ELD_ADMIN_TOKEN` | If set, required on `/admin/v1/status` |
| `ELD_CAPACITY_VALIDATOR_WALLET_NAME` | Wallet name in `wallets.json` for capacity registration |
| `RUST_LOG` | Tracing filter (default in-process: `eld=info`) |

## HTTP / ABCI docs

Canonical client-facing surfaces:

- **REST and Tendermint RPC** — [`eld-client` README](../client/README.md) (`AppApi`, `AbciHttpApi`, `ChainClient`)
- **Wire types** — [`eld-common` README](../common/README.md)

`eld-node --help` is the operator CLI.

## Run with Tendermint

**Single node (local):** Tendermint must be running first, with its ABCI proxy pointed at `consensus_config.json` `app_host`:`app_port` (sample `26658`) and RPC on `node_port` (sample `26657`). From the repo root, with configs and secrets in place under `node_app/` (or CWD matching those relative paths):

```sh
cargo run -p eld-node
```

Query accounts and submit transfers with [`eld-client`](../client/README.md) or a binary built on `ChainClient`.

**Four nodes (Compose):** images, secrets, and scripts live in [`deploy/README.md`](../deploy/README.md) (`compose.yaml`, `start-with-history.sh` / `start-without-history.sh`). That is the supported multi-node path, not four `cargo run` processes.

## Build and test

From the repo root (same gate as CI):

```sh
./deploy/scripts/ci.sh
```

This crate only:

```sh
cargo build -p eld-node
cargo test -p eld-node
```

## Tendermint sync cases

When restarting Eld and Tendermint with persisted data:

1. **Both clean** — chain starts at height 0.
2. **Eld clean, TM has blocks** — Tendermint replays; ABCI handlers must be deterministic (e.g. `BTreeMap` over `HashMap` where ordering matters).
3. **Both have data, TM height ≥ Eld height** — normal resume.
4. **Both have data, TM height < Eld height** — unrecoverable; wipe Eld app data and resync.

Tendermint exposes current height and app hash via RPC `status` and ABCI `Info`.

## License

MIT. Copyright Eld network. See workspace [LICENSE](../LICENSE) and [NOTICE](../NOTICE).
