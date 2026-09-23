#!/usr/bin/env bash
# Stop the local 4-node stack: all eld-app and Tendermint containers, plus the
# compose network. Named volumes (chain / Tendermint data) are left in place so
# cluster-start-with-history.sh can resume. Does not rebuild or pull images.
# Image tags come from deploy/.env (or DEPLOY_ENV_FILE).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=deploy/scripts/_env.sh
source "${SCRIPT_DIR}/../../../_env.sh"
COMPOSE_FILE="$DEPLOY_DIR/docker/local/cluster/compose.yaml"

echo "Stopping 4-node compose using ${ELD_APP_IMAGE} and ${ELD_TM_IMAGE}"
docker compose -f "$COMPOSE_FILE" --env-file "$DEPLOY_ENV_FILE" down --remove-orphans

echo "Stopped 4-node compose (volumes kept)."
