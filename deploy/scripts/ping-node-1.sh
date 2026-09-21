#!/usr/bin/env bash
# Probe Eld node 1: Tendermint RPC (host 26657) and node_app REST (host 9001).
set -euo pipefail

TM_RPC="${TM_RPC:-http://127.0.0.1:26657}"
APP_API="${APP_API:-http://127.0.0.1:9001}"

pretty() {
  if command -v python3 >/dev/null 2>&1; then
    python3 -m json.tool
  else
    cat
  fi
}

fetch() {
  local label="$1"
  local url="$2"
  local tmp
  tmp="$(mktemp)"
  printf '\n==> %s\nGET %s\n' "$label" "$url"

  local http_code
  http_code="$(
    curl -sS -o "$tmp" -w '%{http_code}' --max-time 10 "$url"
  )"

  printf 'HTTP %s\n' "$http_code"
  if [[ ! -s "$tmp" ]]; then
    rm -f "$tmp"
    echo "empty response body" >&2
    return 1
  fi
  pretty <"$tmp" || cat "$tmp"
  rm -f "$tmp"

  if [[ "$http_code" != 2* ]]; then
    echo "${label}: expected 2xx, got ${http_code}" >&2
    return 1
  fi
}

fetch "Tendermint RPC (node 1)" "${TM_RPC%/}/status"
fetch "node_app REST (node 1)" "${APP_API%/}/node_identity"

printf '\nOK: both APIs returned data.\n'
