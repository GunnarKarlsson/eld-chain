# eld-common

Protocol types, transaction wire format, and validation for the [Eld](https://github.com/eldnetwork/eld-chain) blockchain.

HTTP clients, config, and wallet file I/O live in [`eld-client`](../client/README.md). This crate is the shared on-chain type layer. Experimental; not on crates.io yet (`publish = false`).

## Modules

| Area | Main types | Role |
|---|---|---|
| Identity | `Address`, `PublicKey`, `Wallet` | Addresses, Ed25519 keys, signing identity |
| Value | `Coin`, `Nonce`, `FeeConfig` | Balances, fees, nonces |
| Transactions | `Tx`, `validation`, `EldError` | Wire format, parsing, validation |
| State / CADO | `Account`, `CadoBody`, `StakingAccount` | On-chain state and CADO paths |
| Capacity | `CapacityProof`, `CapacitySeed`, `SlotAllocator` | Storage proofs and on-disk slot maps |
| App types | `pinboard`, `namespace`, `SyncMsg` | Pinboard, namespaces, P2P sync messages |
| IDs | `ContentId`, `ManifestId`, `ChallengeId`, … | Typed content and proof identifiers |
| Logging | `logging` | Field sanitizers for logs (no subscriber setup) |

- **Wire format** — transaction signing and ABCI payloads use `serde_json`; CADO bytes use `bincode` (see [Encoding](#encoding)).

### Supporting modules

| Module | Purpose |
|---|---|
| `constants` | Protocol limits and local-dev chain parameters |

## Add to your project

From the same workspace as this repo:

```toml
eld_common = { path = "../common", package = "eld-common" }
```

From git:

```toml
eld_common = { git = "https://github.com/eldnetwork/eld-chain", package = "eld-common" }
```

Rust imports use the underscore crate name: `eld_common`.

## Quick start

```rust
use eld_common::Address;

fn main() -> Result<(), eld_common::error::EldError> {
    let address = Address::parse_hex_str("0x1234567890abcdef1234567890abcdef12345678")?;
    assert_eq!(address.hex().len(), 40);
    Ok(())
}
```

## Encoding

v0 uses three codecs on the wire. Full detail is in the workspace [README](../README.md#encoding).

| Codec | Used for |
|---|---|
| **serde_json** | Transaction signing and mempool/block bytes (UTF-8 hex of signed JSON). HTTP, config, wallets. |
| **bincode** | CADO payload bytes, GossipSub `SyncMsg`, persisted pinboard metadata. |
| **parity-scale-codec** | `Coin` only (Cardano-adapted legacy type; not an Eld wire format). |

## Wallets and security

`wallet::Wallet` is the Ed25519 signing identity for building transactions. **Private keys are not stored in this crate** — wallet JSON files and file permissions are handled by [`eld-client`](../client/README.md#wallets-and-security).

- Do **not** commit wallet files or log serialized key material.
- Report security issues via the workspace [SECURITY.md](../SECURITY.md).

Hex and ID conventions: [TYPE_DESIGN.md](TYPE_DESIGN.md). Third-party Coin attribution: [NOTICE](NOTICE).

## Documentation

| File | Purpose |
|---|---|
| [CHANGELOG.md](CHANGELOG.md) | Crate release notes |
| [TYPE_DESIGN.md](TYPE_DESIGN.md) | Hex and typed ID conventions (canonical copy) |
| [LICENSE](LICENSE) / [NOTICE](NOTICE) | MIT + third-party attribution |

## License

MIT. Copyright Eld network. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
