#!/usr/bin/env bash
# Wipe named volumes and start the local 4-node compose from genesis.
# After a volume wipe, each Tendermint data dir is empty, so
# priv_validator_state.json is missing until unsafe_reset_all recreates it.
# Eld and Tendermint images are pulled from GHCR. Does not build locally.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=deploy/scripts/_env.sh
source "${SCRIPT_DIR}/../../../_env.sh"
COMPOSE_FILE="$DEPLOY_DIR/docker/local/cluster/compose.ghcr.yaml"

compose() {
  docker compose -f "$COMPOSE_FILE" --env-file "$DEPLOY_ENV_FILE" "$@"
}

echo "Starting 4-node compose without history using ghcr.io/eldnetwork/eld-chain:sha-c4c6b1c and ghcr.io/eldnetwork/eld-tendermint:${TENDERMINT_VERSION_TAG_GHCR}"
compose down --volumes --remove-orphans

for i in 1 2 3 4; do
  echo "tendermint unsafe_reset_all on tendermint-${i}"
  compose run --rm --no-deps "tendermint-${i}" \
    tendermint unsafe_reset_all --home /tendermint/.tendermint
done

compose up -d

echo "Started 4-node compose with fresh state (history wiped)."
