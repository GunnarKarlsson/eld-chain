# Contributing

This repo is the Eld workspace: `eld-common` (`common/`), `eld-client` (`client/`), and a parallel copy of the node (`eld_node_app` in `node_app/`). The CLI, faucet, and the node used by Docker/local deploy still live in the sibling `eld` repo and **path-depend** on the library crates. They are not consumed from crates.io yet.

## Pull requests

PRs must pass `./scripts/ci.sh` (fmt, Clippy with warnings denied, build, test including rustdoc, gitleaks).

Public API changes that `eld` uses must keep **chain** (`eld/chain`) and **clients** (`eld/clients`) compiling against the new path deps. Coordinate call sites there in the same change.

Do not publish crates or flip `publish = true` unless that is the explicit goal of the PR.

## Layout

- Protocol types, validation, `Wallet`, `SlotAllocator` → `eld-common`
- Tendermint RPC, app REST, faucet HTTP, CWD config, `wallets.json` I/O, `ChainClient` → `eld-client`
- ABCI node binary (`eld_node_app`) → `node_app/` (copy of `eld/chain/node_app`; do not treat this as the deploy source yet)
- Package names are hyphenated for libraries (`eld-common`); the node package is still `eld_node_app`. Rust imports use underscores (`eld_common`)
- License and crate docs live **in each library crate directory** (`LICENSE`, `README.md`, `NOTICE`, `TYPE_DESIGN.md`) so a future crates.io tarball includes them
