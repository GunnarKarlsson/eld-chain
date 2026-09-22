# Contributing

This repo is the Eld workspace: `eld-common` (`common/`), `eld-client` (`client/`), and the ABCI node (`eld-node` in `node_app/`). Crates are not on crates.io yet (`publish = false`).

## Pull requests

PRs must pass `./deploy/scripts/ci.sh` (fmt, Clippy with warnings denied, build, test including rustdoc, gitleaks).

Treat changes to public types and functions as API changes: update crate READMEs, rustdoc, and examples when behavior or wire format shifts.

Do not publish crates or flip `publish = true` unless that is the explicit goal of the PR.

Use the [pull request template](.github/PULL_REQUEST_TEMPLATE.md). Report security issues through [SECURITY.md](SECURITY.md), not public issues.

## Layout

| Path | Package | Contents |
|---|---|---|
| `common/` | `eld-common` | Protocol types, validation, `Wallet`, `SlotAllocator` |
| `client/` | `eld-client` | Tendermint RPC, app REST, faucet HTTP, CWD config, `wallets.json` I/O, `ChainClient` |
| `node_app/` | `eld-node` | ABCI node binary and server logic |
| `deploy/` | — | CI script, local four-node Compose, Dockerfiles |

Package names are hyphenated (`eld-common`, `eld-client`, `eld-node`). Rust imports use underscores (`eld_common`, `eld_client`, `eld_node`).

Each library crate directory includes `LICENSE`, `README.md`, and `NOTICE`. `eld-common` and `eld-client` also maintain `CHANGELOG.md`; hex/ID rules live in `common/TYPE_DESIGN.md`.

## Documentation

When changing public API or wire behavior, update the relevant crate README and, if IDs or hex rules change, [common/TYPE_DESIGN.md](common/TYPE_DESIGN.md). Node protocol notes go under [node_app/docs/](node_app/docs/). Local cluster setup is documented in [deploy/README.md](deploy/README.md).

## Code of conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).
