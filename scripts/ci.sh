#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

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
