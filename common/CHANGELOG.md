# Changelog

All notable changes to `eld-common` are documented here.

## [0.1.0] - 2026-03-21

First experimental release as a library crate inside `eld-chain` (not published to crates.io).

### Added

- Protocol types: `Address`, `Coin`, transactions, CADO, capacity proofs, pinboard/namespace types.
- `Wallet` Ed25519 signing identity and transaction validation.
- `SlotAllocator` and on-disk capacity slot maps.
- [TYPE_DESIGN.md](TYPE_DESIGN.md) for hex and typed ID conventions.

Wire format and validation match the current node and client crates in this workspace; this is not a frozen mainnet spec.
