#!/usr/bin/env bash
# Wipe named volumes and start the local single pair from genesis.
# After a volume wipe, the Tendermint data dir is empty, so
# priv_validator_state.json is missing until unsafe_reset_all recreates it.
# Image tags come from deploy/.env (or DEPLOY_ENV_FILE). Does not rebuild images.
# Project name is eld-single, so this does not touch the 4-node volumes.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=deploy/scripts/_env.sh
source "${SCRIPT_DIR}/../../../_env.sh"
COMPOSE_FILE="$DEPLOY_DIR/docker/local/single/compose.yaml"

compose() {
  docker compose -f "$COMPOSE_FILE" --env-file "$DEPLOY_ENV_FILE" "$@"
}

echo "Starting single compose without history using ${ELD_APP_IMAGE} and ${ELD_TM_IMAGE}"
compose down --volumes --remove-orphans

echo "tendermint unsafe_reset_all on tendermint-1"
compose run --rm --no-deps tendermint-1 \
  tendermint unsafe_reset_all --home /tendermint/.tendermint

compose up -d

echo "Started single compose with fresh state (history wiped)."
