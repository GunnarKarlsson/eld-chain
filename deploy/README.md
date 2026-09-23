# deploy

Local development infrastructure for [`eld-chain`](../README.md): CI scripts, Docker Compose layouts, and per-node config mounts.

## Layout

| Path | Purpose |
|---|---|
| [`scripts/ci.sh`](scripts/ci.sh) | Workspace CI gate (fmt, Clippy, build, test, gitleaks) — same as GitHub Actions |
| [`docker/Dockerfile.app`](docker/Dockerfile.app), [`Dockerfile.eld-base`](docker/Dockerfile.eld-base), [`Dockerfile.tendermint`](docker/Dockerfile.tendermint) | Local image build |
| [`docker/local/cluster/`](docker/local/cluster/) | Four `eld-app` + four Tendermint pairs. Checked-in config is `nodes/N/{app,tendermint}` |
| [`docker/local/single/`](docker/local/single/) | One app + one Tendermint. Own one-validator Tendermint config; app files and keys from cluster node 1 |
| [`docker/remote/cluster/`](docker/remote/cluster/) | Same four pairs on one host, images pulled from ECR |
| [`host/single/config/`](host/single/config/) | Localhost sample for a binary next to a Tendermint you start yourself (`127.0.0.1`, `./data/capacity`) |
| [`.env.example`](.env.example) | Template for image tags and external paths used by build scripts |

Image builds, CI, and secret sync stay in [`scripts/`](scripts/). Start and stop scripts live next to the stack they run: [`scripts/docker/local/cluster/`](scripts/docker/local/cluster/), [`scripts/docker/local/single/`](scripts/docker/local/single/), and [`scripts/host/`](scripts/host/).

## CI

From the repo root:

```sh
./deploy/scripts/ci.sh
```

Rustc and Clippy warnings are errors. Install [gitleaks](https://github.com/gitleaks/gitleaks) for the secret scan step.

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

If you already maintain these files elsewhere, `./deploy/scripts/sync-secrets-from-eld.sh` copies them when `ELD_ROOT` in `.env` points at that tree (see script for expected layout).

## Images

```sh
./deploy/scripts/build-docker-image-tendermint-macos
./deploy/scripts/build-docker-image-eld-node
```

The node script is a two-stage flow: `Dockerfile.eld-base` compiles `eld-node` (`eld-base:<NODE_APP_VERSION_TAG>`), then `Dockerfile.app` is `FROM eld_base` and tags `eld-app:<NODE_APP_VERSION_TAG>`. Compose only runs the runtime image; the base image is a build cache, not a compose service.

Tendermint is compiled `GOOS=linux` in `TENDERMINT_DIR` for the Mac’s CPU, then that tree’s `build/tendermint` is wrapped as `eld-tendermint:<TENDERMINT_VERSION_TAG>`.

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

## Remote cluster (ECR)

`docker/remote/cluster/compose.yaml` is the same four pairs, with images pulled from ECR (`IMAGE_VERSION_APP`, `IMAGE_VERSION_TM`). It does not build. Copy `docker/local/cluster/nodes` and `wallets` onto the host first:

- `/home/ec2-user/eld-deploy/nodes/N/app`
- `/home/ec2-user/eld-deploy/nodes/N/tendermint`
- `/home/ec2-user/eld-deploy/wallets/wallets.json`

```sh
IMAGE_VERSION_APP=0.0.2 IMAGE_VERSION_TM=0.0.2 \
  docker compose -f deploy/docker/remote/cluster/compose.yaml pull
IMAGE_VERSION_APP=0.0.2 IMAGE_VERSION_TM=0.0.2 \
  docker compose -f deploy/docker/remote/cluster/compose.yaml up -d
```

## Host binary

`host/single/config/` is the localhost sample (`127.0.0.1`, `./data/capacity`). It is not mounted by Compose. These scripts start a local `tendermint` on PATH first (`--proxy_app=tcp://127.0.0.1:26658`), then `eld-node` from `host/single`. Tendermint logs go to `host/single/tendermint.log`. Ctrl-C stops both.

Wipe Tendermint data (`tendermint unsafe_reset_all`) and `host/single/data`, then start:

```sh
./deploy/scripts/host/host-start-without-history.sh
```

Keep Tendermint data and pass `--init-data` to `eld-node`:

```sh
./deploy/scripts/host/host-start-with-history.sh
```
