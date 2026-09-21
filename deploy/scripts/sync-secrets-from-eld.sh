#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=deploy/scripts/_env.sh
source "${SCRIPT_DIR}/_env.sh"

APP_DOCKER="${ELD_ROOT}/chain/node_app/config-docker"
TM_DOCKER="${ELD_ROOT}/third_party/tendermint-configs/docker/multi"
WALLETS_SRC="${ELD_ROOT}/chain/node_app/wallets/wallets.json"

if [[ ! -d "$ELD_ROOT" ]]; then
  echo "Sibling eld repo not found at $ELD_ROOT (set ELD_ROOT)." >&2
  exit 1
fi
if [[ ! -f "$WALLETS_SRC" ]]; then
  echo "Missing $WALLETS_SRC" >&2
  exit 1
fi

mkdir -p "$DEPLOY_DIR/wallets"
cp "$WALLETS_SRC" "$DEPLOY_DIR/wallets/wallets.json"

for n in 1 2 3 4; do
  app_dst="$DEPLOY_DIR/nodes/$n/app"
  tm_dst="$DEPLOY_DIR/nodes/$n/tendermint"
  mkdir -p "$app_dst" "$tm_dst"

  cp "$APP_DOCKER/config-docker-$n/p2p_keypair.json" "$app_dst/p2p_keypair.json"
  cp "$TM_DOCKER/tendermint-docker-config-$n/node_key.json" "$tm_dst/node_key.json"
  cp "$TM_DOCKER/tendermint-docker-config-$n/priv_validator_key.json" "$tm_dst/priv_validator_key.json"
  if [[ -f "$TM_DOCKER/tendermint-docker-config-$n/addrbook.json" ]]; then
    cp "$TM_DOCKER/tendermint-docker-config-$n/addrbook.json" "$tm_dst/addrbook.json"
  fi
done

echo "Copied local-dev secrets from $ELD_ROOT into $DEPLOY_DIR"
