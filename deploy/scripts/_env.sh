# Shared by deploy scripts. Locates deploy/ from this file, so callers may live
# under scripts/docker/local/{cluster,single} as well as scripts/.
# shellcheck shell=bash

ENV_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEPLOY_DIR="$(cd "${ENV_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${DEPLOY_DIR}/.." && pwd)"
DEPLOY_ENV_FILE="${DEPLOY_ENV_FILE:-${DEPLOY_DIR}/.env}"

if [[ ! -f "$DEPLOY_ENV_FILE" ]]; then
  echo "Missing env file: $DEPLOY_ENV_FILE" >&2
  echo "Copy the template: cp ${DEPLOY_DIR}/.env.example ${DEPLOY_DIR}/.env" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "$DEPLOY_ENV_FILE"
set +a

require_env() {
  local name="$1"
  local value="${!name:-}"
  if [[ -z "$value" ]]; then
    echo "$name is not set in $DEPLOY_ENV_FILE" >&2
    exit 1
  fi
}

require_env TENDERMINT_DIR
require_env TENDERMINT_VERSION_TAG
require_env NODE_APP_VERSION_TAG

if [[ "$TENDERMINT_DIR" != /* ]]; then
  TENDERMINT_DIR="$(cd "${REPO_ROOT}/${TENDERMINT_DIR}" && pwd)"
fi

TENDERMINT_DOCKERFILE="${TENDERMINT_DOCKERFILE:-${DEPLOY_DIR}/docker/Dockerfile.tendermint}"
ELD_TM_IMAGE="eld-tendermint:${TENDERMINT_VERSION_TAG}"
ELD_APP_IMAGE="eld-app:${NODE_APP_VERSION_TAG}"
ELD_BASE_IMAGE="eld-base:${NODE_APP_VERSION_TAG}"
