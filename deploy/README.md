# deploy

Local development infrastructure for [`eld-chain`](../README.md): CI scripts, Docker Compose layouts, and per-node config mounts.

## Layout

| Path | Purpose |
|---|---|
| [`scripts/ci.sh`](scripts/ci.sh) | Workspace CI gate (fmt, Clippy, build, test, cargo audit, cargo deny, gitleaks) — same as GitHub Actions |
| [`docker/Dockerfile.app`](docker/Dockerfile.app), [`Dockerfile.eld-base`](docker/Dockerfile.eld-base), [`Dockerfile.tendermint`](docker/Dockerfile.tendermint) | Local image build |
| [`docker/Dockerfile.release`](docker/Dockerfile.release) | GHCR release image (`linux/amd64`, `linux/arm64`) |
| [`docker/local/cluster/`](docker/local/cluster/) | Four `eld-app` + four Tendermint pairs. Checked-in config is `nodes/N/{app,tendermint}` |
| [`docker/local/single/`](docker/local/single/) | One app + one Tendermint. Own one-validator Tendermint config; app files and keys from cluster node 1 |
| [`docker/remote/cluster/`](docker/remote/cluster/) | Same four pairs on one host, images pulled from ECR |
| [`.env.example`](.env.example) | Template for image tags and external paths used by build scripts |

Image builds and CI stay in [`scripts/`](scripts/). Start and stop scripts live next to the stack they run: [`scripts/docker/local/cluster/`](scripts/docker/local/cluster/) and [`scripts/docker/local/single/`](scripts/docker/local/single/). Supported runtime is Docker Compose (single node or 4-node).

## CI

From the repo root:

```sh
./deploy/scripts/ci.sh
```

Rustc and Clippy warnings are errors. Install [cargo-audit](https://github.com/rustsec/rustsec) 0.22.2+, [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) 0.20.2+, and [gitleaks](https://github.com/gitleaks/gitleaks). Policy lives in `deny.toml` and `.cargo/audit.toml`.

---

# Local 4-node Docker Compose

Four `eld-app` + four Tendermint pairs on one machine (`docker/local/cluster/compose.yaml`). Images are built locally. That Compose file is the only checked-in copy of app and Tendermint config (`nodes/`).

ABCI listen ports differ per app (`26658`, `26668`, `26678`, `26688`). Do not collapse those onto one port.

## Prerequisites

- Docker Compose v2
- Go (to compile Tendermint)
- `deploy/.env` (gitignored). Copy the template once:

```sh
cp deploy/.env.example deploy/.env
```

Set `TENDERMINT_DIR` (path to a Tendermint source tree), `TENDERMINT_VERSION_TAG`, and `NODE_APP_VERSION_TAG` in `.env`. Tags include the OS, for example `macos-0.0.16` and `macos-0.0.41` → images `eld-tendermint:macos-0.0.16` and `eld-app:macos-0.0.41`. Build and compose scripts read that file (override with `DEPLOY_ENV_FILE`).

## Secrets (not committed)

Before first run, each node needs local key material under `deploy/docker/local/cluster/`:

- `wallets/wallets.json`
- `nodes/N/app/p2p_keypair.json` (N = 1..4)
- `nodes/N/tendermint/node_key.json` and `priv_validator_key.json`

Create them locally. They are gitignored.

## Images

```sh
./deploy/scripts/build-docker-image-tendermint-macos
./deploy/scripts/build-docker-image-eld-node
```

The node script is a two-stage flow: `Dockerfile.eld-base` compiles `eld-node` (`eld-base:<NODE_APP_VERSION_TAG>`), then `Dockerfile.app` is `FROM eld_base` and tags `eld-app:<NODE_APP_VERSION_TAG>`. Compose only runs the runtime image; the base image is a build cache, not a compose service.

Tendermint is compiled `GOOS=linux` in `TENDERMINT_DIR` for the Mac’s CPU, then that tree’s `build/tendermint` is wrapped as `eld-tendermint:<TENDERMINT_VERSION_TAG>`.

## Publish

A tag matching `v*.*.*` starts [`.github/workflows/image.yml`](../.github/workflows/image.yml). CI runs first. The image job then publishes `linux/amd64` and `linux/arm64` as one manifest:

`ghcr.io/eldnetwork/eld-chain:<tag>`

```sh
git tag -a v0.0.1 -m "eld-chain v0.0.1"
git push origin v0.0.1
```

`Dockerfile.release` is a release build (`cargo build --release --locked -p eld-node`). The local `eld-app` image stays a debug build from `Dockerfile.eld-base` and `Dockerfile.app`. The process working directory is `/app`, so the same config, wallet, and data mounts apply.

## Run

Start and stop scripts load `deploy/.env` (or `DEPLOY_ENV_FILE`) and pass it to Compose so `eld-app` / `eld-tendermint` tags match `NODE_APP_VERSION_TAG` and `TENDERMINT_VERSION_TAG`. They do not rebuild images.

Keep chain and Tendermint volumes (restart containers only):

```sh
./deploy/scripts/docker/local/cluster/cluster-start-with-history.sh
```

Wipe volumes and start from genesis (fresh state). After the wipe this runs `tendermint unsafe_reset_all` on each Tendermint service so `data/priv_validator_state.json` exists before `up`:

```sh
./deploy/scripts/docker/local/cluster/cluster-start-without-history.sh
```

Stop containers and the compose network (keep volumes):

```sh
./deploy/scripts/docker/local/cluster/cluster-stop.sh
```

## Ports (host)

| Pair | REST | ABCI | TM P2P | TM RPC |
|------|------|------|--------|--------|
| 1 | 9001 | 26658 | 26656 | 26657 |
| 2 | 9002 | 26668 | 26666 | 26667 |
| 3 | 9003 | 26678 | 26676 | 26677 |
| 4 | 9004 | 26688 | 26686 | 26687 |

Inside the Compose network, each Tendermint RPC still listens on `26657` (`TENDERMINT_RPC_URL=http://tendermint-N:26657`).

The Compose project name is `deploy`, so existing named volumes (`deploy_eld-data-*`, `deploy_tendermint-data-*`) still attach.

## Local single pair

`docker/local/single/compose.yaml` starts `eld-app-1` and `tendermint-1` only, with the same host ports as cluster node 1. App config and wallets come from cluster node 1. Tendermint uses `docker/local/single/tendermint/` (one validator, no peers). `node_key.json` and `priv_validator_key.json` in that directory are local copies of cluster node 1's keys, gitignored, so `unsafe_reset_all` can rewrite them. The four-node files under `docker/local/cluster/nodes/` are unchanged.

Same three actions as the cluster, against the single Compose project (`eld-single`). `single-start-without-history.sh` resets only `tendermint-1` and does not touch the 4-node volumes.

```sh
./deploy/scripts/docker/local/single/single-start-with-history.sh
./deploy/scripts/docker/local/single/single-start-without-history.sh
./deploy/scripts/docker/local/single/single-stop.sh
```
