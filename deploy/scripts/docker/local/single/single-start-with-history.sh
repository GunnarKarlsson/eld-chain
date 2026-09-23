#!/usr/bin/env bash
# Restart the local single pair, keeping named volumes (chain / Tendermint history).
# Does not run unsafe_reset_all — that would wipe validator state.
# Image tags come from deploy/.env (or DEPLOY_ENV_FILE). Does not rebuild images.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=deploy/scripts/_env.sh
source "${SCRIPT_DIR}/../../../_env.sh"
COMPOSE_FILE="$DEPLOY_DIR/docker/local/single/compose.yaml"

compose() {
  docker compose -f "$COMPOSE_FILE" --env-file "$DEPLOY_ENV_FILE" "$@"
}

echo "Starting single compose with history using ${ELD_APP_IMAGE} and ${ELD_TM_IMAGE}"
compose down --remove-orphans
compose up -d

echo "Started single compose (volumes kept)."
