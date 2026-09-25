#!/usr/bin/env bash
# Wipe named volumes and start the local 4-node compose from genesis.
# After a volume wipe, each Tendermint data dir is empty, so
# priv_validator_state.json is missing until unsafe_reset_all recreates it.
# Image tags come from deploy/.env (or DEPLOY_ENV_FILE). Does not rebuild images.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=deploy/scripts/_env.sh
source "${SCRIPT_DIR}/../../../_env.sh"
COMPOSE_FILE="$DEPLOY_DIR/docker/local/cluster/compose.ghcr.yaml"

compose() {
  docker compose -f "$COMPOSE_FILE" --env-file "$DEPLOY_ENV_FILE" "$@"
}

echo "Starting 4-node compose without history using ${ELD_APP_IMAGE} and ghcr ${ELD_TM_IMAGE}"
compose down --volumes --remove-orphans

for i in 1 2 3 4; do
  echo "tendermint unsafe_reset_all on tendermint-${i}"
  compose run --rm --no-deps "tendermint-${i}" \
    tendermint unsafe_reset_all --home /tendermint/.tendermint
done

compose up -d

echo "Started 4-node compose with fresh state (history wiped)."
