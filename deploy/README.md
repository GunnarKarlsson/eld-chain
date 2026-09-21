# deploy

Local development infrastructure for [`eld-chain`](../README.md): CI scripts, Docker Compose for a four-node cluster, and per-node config mounts.

This tree is for **local** multi-node testing on one machine.

## Layout

| Path | Purpose |
|---|---|
| [`scripts/ci.sh`](scripts/ci.sh) | Workspace CI gate (fmt, Clippy, build, test, gitleaks) — same as GitHub Actions |
| [`compose.yaml`](compose.yaml) | Four `eld-app` + four Tendermint pairs |
| [`nodes/`](nodes/) | Per-node app and Tendermint config mounted into containers |
| [`docker/`](docker/) | Dockerfiles (`Dockerfile.app`, `Dockerfile.eld-base`, Tendermint wrapper) |
| [`.env.example`](.env.example) | Template for image tags and external paths used by build scripts |

Helper scripts: `start-with-history.sh`, `start-without-history.sh`, `stop.sh`, `sync-secrets-from-eld.sh`, image build scripts under `scripts/`.

## CI

From the repo root:

```sh
./deploy/scripts/ci.sh
```

Rustc and Clippy warnings are errors. Install [gitleaks](https://github.com/gitleaks/gitleaks) for the secret scan step.

---

# Local 4-node Docker Compose

Four `eld-app` + four Tendermint pairs on one machine. Images are built locally.

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

Before first run, each node needs local key material under `deploy/nodes/`:

- `deploy/wallets/wallets.json`
- `deploy/nodes/N/app/p2p_keypair.json` (N = 1..4)
- `deploy/nodes/N/tendermint/node_key.json` and `priv_validator_key.json`

If you already maintain these files elsewhere, `./deploy/scripts/sync-secrets-from-eld.sh` copies them when `ELD_ROOT` in `.env` points at that tree (see script for expected layout).

## Images

```sh
./deploy/scripts/build-docker-image-tendermint-macos
./deploy/scripts/build-docker-image-eld-node
```

The node script is a two-stage flow: `Dockerfile.eld-base` compiles `eld_node_app` (`eld-base:<NODE_APP_VERSION_TAG>`), then `Dockerfile.app` is `FROM eld_base` and tags `eld-app:<NODE_APP_VERSION_TAG>`. Compose only runs the runtime image; the base image is a build cache, not a compose service.

Tendermint is compiled `GOOS=linux` for the Mac’s CPU, then wrapped as `eld-tendermint:<TENDERMINT_VERSION_TAG>`.

## Run

Start and stop scripts load `deploy/.env` (or `DEPLOY_ENV_FILE`) and pass it to Compose so `eld-app` / `eld-tendermint` tags match `NODE_APP_VERSION_TAG` and `TENDERMINT_VERSION_TAG`. They do not rebuild images.

Keep chain and Tendermint volumes (restart containers only):

```sh
./deploy/scripts/start-with-history.sh
```

Wipe volumes and start from genesis (fresh state). After the wipe this runs `tendermint unsafe_reset_all` on each Tendermint service so `data/priv_validator_state.json` exists before `up`:

```sh
./deploy/scripts/start-without-history.sh
```

Stop containers and the compose network (keep volumes):

```sh
./deploy/scripts/stop.sh
```

## Ports (host)

| Pair | REST | ABCI | TM P2P | TM RPC |
|------|------|------|--------|--------|
| 1 | 9001 | 26658 | 26656 | 26657 |
| 2 | 9002 | 26668 | 26666 | 26667 |
| 3 | 9003 | 26678 | 26676 | 26677 |
| 4 | 9004 | 26688 | 26686 | 26687 |

Inside the Compose network, each Tendermint RPC still listens on `26657` (`TENDERMINT_RPC_URL=http://tendermint-N:26657`).
