# eld-common

Protocol types, transaction wire format, and validation for the [Eld](https://github.com/eldnetwork/eld-chain) blockchain.

This crate is **not** published to crates.io yet (`publish = false`). Consume it as a path or git dependency. HTTP clients, CWD config, and wallet-file I/O live in [`eld-client`](https://github.com/eldnetwork/eld-chain/tree/main/client).

## Crate map

| Area | Modules |
|---|---|
| Identity | `address::Address`, `public_key`, `wallet::Wallet` |
| Value | `coin`, `nonce`, `fee` |
| Transactions | `tx`, `validation`, `error` |
| State / CADO | `account`, `cado`, `staking_account` |
| Capacity | `capacity` (proofs, seeds, merkle roots), `storage` (`SlotAllocator`) |
| App types | `pinboard`, `namespace`, `sync_msg` |
| IDs | `ContentId`, `ManifestId`, `ChallengeId`, and related typed IDs |
| Logging | `logging` — field sanitizers only; process subscriber setup lives in `eld` binaries |

Hex and ID conventions: [TYPE_DESIGN.md](TYPE_DESIGN.md). Third-party Coin attribution: [NOTICE](NOTICE).

## Usage

Path-depend from a workspace sibling (this repo):

```toml
eld_common = { path = "../common", package = "eld-common" }
```

The `eld` node, CLI, and faucet use:

```toml
eld_common = { path = "../../../eld-chain/common", package = "eld-common" }
```

```rust
use eld_common::Address;

fn main() -> Result<(), eld_common::error::EldError> {
    let address = Address::parse_hex_str("0x1234567890abcdef1234567890abcdef12345678")?;
    assert_eq!(address.hex().len(), 40);
    Ok(())
}
```

Rust imports use the underscore crate name `eld_common`.

## License

MIT. Copyright Eld network. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
