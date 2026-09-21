# Local 4-node Docker Compose

Four `eld-app` + four Tendermint pairs on one machine. Images are built locally. There is no EC2 or single-node flow here.

ABCI listen ports differ per app (`26658`, `26668`, `26678`, `26688`). Do not collapse those onto one port.

## Prerequisites

- Docker Compose v2
- Go (to compile Tendermint)
- Sibling [`eld`](https://github.com/eldnetwork/eld) checkout, default `../eld`
- `deploy/.env` (gitignored). Copy the template once:

```sh
cp deploy/.env.example deploy/.env
```

Set `ELD_ROOT`, `TENDERMINT_DIR`, `TENDERMINT_VERSION_TAG`, and `NODE_APP_VERSION_TAG` in `.env`. Tags include the OS, for example `macos-0.0.16` and `macos-0.0.41` → images `eld-tendermint:macos-0.0.16` and `eld-app:macos-0.0.41`. Build and compose scripts read that file (override with `DEPLOY_ENV_FILE`).

## Secrets (not committed)

```sh
./deploy/scripts/sync-secrets-from-eld.sh
```

That copies `wallets.json`, each `p2p_keypair.json`, and each Tendermint `node_key.json` / `priv_validator_key.json` from `ELD_ROOT`.

## Images

```sh
./deploy/scripts/build-docker-image-tendermint-macos
./deploy/scripts/build-docker-image-eld-node
```

The node script is the old two-stage Eld flow: `Dockerfile.eld-base` compiles `eld_node_app` (`eld-base:<NODE_APP_VERSION_TAG>`), then `Dockerfile.app` is `FROM eld_base` and tags `eld-app:<NODE_APP_VERSION_TAG>`. Compose only runs the runtime image; the base image is a build cache, not a compose service.

Tendermint is compiled `GOOS=linux` for the Mac’s CPU, then wrapped like old `Dockerfile.tendermint-v2` as `eld-tendermint:<TENDERMINT_VERSION_TAG>`.

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
