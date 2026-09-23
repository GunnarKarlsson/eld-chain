#!/usr/bin/env bash
# Stop the local single pair: eld-app-1, tendermint-1, and the compose network.
# Named volumes are left in place so single-start-with-history.sh can resume.
# Does not rebuild or pull images, and does not touch the 4-node stack.
# Image tags come from deploy/.env (or DEPLOY_ENV_FILE).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=deploy/scripts/_env.sh
source "${SCRIPT_DIR}/../../../_env.sh"
COMPOSE_FILE="$DEPLOY_DIR/docker/local/single/compose.yaml"

echo "Stopping single compose using ${ELD_APP_IMAGE} and ${ELD_TM_IMAGE}"
docker compose -f "$COMPOSE_FILE" --env-file "$DEPLOY_ENV_FILE" down --remove-orphans

echo "Stopped single compose (volumes kept)."
