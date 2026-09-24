#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}/../.."

step() {
  printf '\n==> %s\n' "$1"
}

# Workspace crates deny rustc warnings via [workspace.lints.rust] (`warnings = "deny"`).
# Clippy also gets `-D warnings` so Clippy lints cannot stay warn-level.
# Do not set RUSTFLAGS=-D warnings here: that fails on third-party crates too.

step "cargo fmt --all -- --check"
cargo fmt --all -- --check

step "cargo clippy --workspace --all-targets --all-features -- -D warnings"
cargo clippy --workspace --all-targets --all-features -- -D warnings

step "cargo build --workspace --all-targets --all-features"
cargo build --workspace --all-targets --all-features

step "cargo test --workspace --all-targets --all-features"
cargo test --workspace --all-targets --all-features

step "cargo test --workspace --doc"
cargo test --workspace --doc

step "cargo audit"
if ! command -v cargo-audit >/dev/null 2>&1; then
  cat >&2 <<'EOF'
cargo-audit is not installed.
cargo install cargo-audit --locked --version 0.22.2
macOS: brew install cargo-audit
EOF
  exit 1
fi
# 0.21 cannot parse CVSS 4.0 advisories in the current RustSec database.
audit_version="$(cargo audit --version | awk '{print $NF}')"
IFS=. read -r audit_major audit_minor _ <<<"${audit_version}"
if [ "${audit_major}" -lt 0 ] || { [ "${audit_major}" -eq 0 ] && [ "${audit_minor}" -lt 22 ]; }; then
  echo "cargo-audit ${audit_version} is too old; install 0.22.2 or newer" >&2
  exit 1
fi
cargo audit --deny yanked --deny unsound

step "cargo deny"
if ! command -v cargo-deny >/dev/null 2>&1; then
  cat >&2 <<'EOF'
cargo-deny is not installed.
cargo install cargo-deny --locked --version 0.20.2
macOS: brew install cargo-deny
EOF
  exit 1
fi
cargo deny check advisories bans licenses sources

step "gitleaks"
if ! command -v gitleaks >/dev/null 2>&1; then
  cat >&2 <<'EOF'
gitleaks is not installed.
macOS: brew install gitleaks
Linux: download a release from https://github.com/gitleaks/gitleaks/releases
EOF
  exit 1
fi

# Git history (committed secrets) plus the working tree (uncommitted files).
# `dir` ignores build artifacts via .gitleaks.toml.
gitleaks git --verbose --redact --no-banner --platform github
gitleaks dir --verbose --redact --no-banner
