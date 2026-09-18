# ID and hex conventions

Domain types store raw bytes. String form is for JSON, logs, and CLI input. Canonical output is lowercase hex. This is the current library behavior, not a frozen mainnet spec.

| Type | Size | Parse | Canonical display |
|---|---|---|---|
| `Address` | 20 bytes | optional `0x` / `0X`, exactly 40 hex digits | `hex()` has no prefix; `hex_with_prefix()` is `0x` + lowercase |
| `ContentId`, `ChunkId`, `ManifestId` | 32 bytes | **requires** `0x` + 64 hex digits | `0x` + lowercase |
| `ContractId` | 32 bytes | optional `0x` | `0x` + lowercase |
| `PublicKey` | 32 bytes (Ed25519) | optional `0x` | lowercase hex, no `0x` |
| `ChallengeId`, `CapacitySeed`, `CapacityMerkleRoot` | 32 bytes | optional `0x` | lowercase hex, **no** `0x` (matches current P2P / proof tx edges) |

Typed IDs are for in-crate use. Some wire structs (`SyncMsg`, several tx fields) still use `String` or `[u8; 32]` at the edge; convert at those boundaries rather than changing the wire format in place.

Account addresses are `SHA-256(ed25519_verifying_key)[..20]`.
