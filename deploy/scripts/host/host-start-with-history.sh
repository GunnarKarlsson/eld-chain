#!/usr/bin/env bash
# Start a local Tendermint and eld-node from deploy/host/single, keeping history.
# Tendermint starts first, without unsafe_reset_all. eld-node is passed --init-data
# so it reloads RocksDB under deploy/host/single/data.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
HOST_DIR="${REPO_ROOT}/deploy/host/single"
TM_HOME="${TMHOME:-${HOME}/.tendermint}"
TM_PROXY="tcp://127.0.0.1:26658"
TM_LOG="${HOST_DIR}/tendermint.log"

if ! command -v tendermint >/dev/null 2>&1; then
  echo "tendermint is not on PATH." >&2
  exit 1
fi

stop_port() {
  local port="$1"
  local pids
  pids="$(lsof -ti:"${port}" 2>/dev/null || true)"
  if [[ -n "${pids}" ]]; then
    echo "Stopping process on port ${port}"
    # shellcheck disable=SC2086
    kill -TERM ${pids} 2>/dev/null || true
    sleep 1
    # shellcheck disable=SC2086
    kill -KILL ${pids} 2>/dev/null || true
  fi
}

cd "${HOST_DIR}"

stop_port 26657
stop_port 26656
stop_port 26658
stop_port 9001

if [[ ! -f "${TM_HOME}/config/genesis.json" ]]; then
  echo "tendermint init (${TM_HOME})"
  tendermint init --home "${TM_HOME}"
fi

echo "Starting tendermint node (log: ${TM_LOG})"
tendermint node --home "${TM_HOME}" --proxy_app="${TM_PROXY}" --log_level debug >"${TM_LOG}" 2>&1 &
TM_PID=$!

cleanup() {
  if kill -0 "${TM_PID}" 2>/dev/null; then
    echo "Stopping tendermint (${TM_PID})"
    kill -TERM "${TM_PID}" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

echo "Starting eld-node"
cargo run -p eld-node --manifest-path "${REPO_ROOT}/Cargo.toml" -- --init-data
