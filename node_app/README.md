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
| `RUST_LOG` | Tracing filter. Targets are crate names (`eld_node`, not `eld-node` or `eld`). Default when unset: `eld_node=info,eld_common=info,eld_client=info,abci=warn`. Debug example: `RUST_LOG=eld_node=debug,eld_common=info,eld_client=info,abci=warn`. If you use a global `info` instead, quiet noisy crates with `hyper=warn,tower=warn,h2=warn,tokio=warn,rustls=warn`. |

## HTTP / ABCI docs

Canonical client-facing surfaces:

- **REST and Tendermint RPC** — [`eld-client` README](../client/README.md) (`AppApi`, `AbciHttpApi`, `ChainClient`)
- **Wire types** — [`eld-common` README](../common/README.md)

`eld-node --help` is the operator CLI.

## Run with Tendermint

Supported runtime is Docker Compose. Images, expected secrets, and scripts are outlined in [`deploy/README.md`](../deploy/README.md).

- **Single node:** [`single-start-with-history.sh`](../deploy/scripts/docker/local/single/single-start-with-history.sh) / [`single-start-without-history.sh`](../deploy/scripts/docker/local/single/single-start-without-history.sh)
- **Four nodes:** [`cluster-start-with-history.sh`](../deploy/scripts/docker/local/cluster/cluster-start-with-history.sh) / [`cluster-start-without-history.sh`](../deploy/scripts/docker/local/cluster/cluster-start-without-history.sh)

Query accounts and submit transfers with [`eld-client`](../client/README.md) or a binary built on `ChainClient`.

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
