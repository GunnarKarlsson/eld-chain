# Contributing

This repo is the shared library workspace for Eld: `eld-common` (`common/`) and `eld-client` (`client/`). The node, CLI, and faucet live in the sibling `eld` repo and **path-depend** on both crates. They are not consumed from crates.io yet.

## Pull requests

PRs must pass `./scripts/ci.sh` (fmt, Clippy with warnings denied, build, test including rustdoc, gitleaks).

Public API changes that `eld` uses must keep **chain** (`eld/chain`) and **clients** (`eld/clients`) compiling against the new path deps. Coordinate call sites there in the same change.

Do not publish crates or flip `publish = true` unless that is the explicit goal of the PR.

## Layout

- Protocol types, validation, `Wallet`, `SlotAllocator` → `eld-common`
- Tendermint RPC, app REST, faucet HTTP, CWD config, `wallets.json` I/O, `ChainClient` → `eld-client`
- Package names are hyphenated (`eld-common`); Rust imports use underscores (`eld_common`)
- License and crate docs live **in each crate directory** (`LICENSE`, `README.md`, `NOTICE`, `TYPE_DESIGN.md`) so a future crates.io tarball includes them
