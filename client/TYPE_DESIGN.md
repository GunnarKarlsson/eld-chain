# ID and hex conventions

Domain types store raw bytes. String form is for JSON, logs, and CLI input. Canonical output is lowercase hex. This is the current library behavior, not a frozen mainnet spec.

**Parse (all of these types):** accept an optional `0x` / `0X` prefix and case-insensitive hex digits of the exact byte length. Do not require a prefix on HTTP, tx JSON, CLI, or serde input — clients in testing send both forms.

**Display / serde serialize:** keep the historical wire form so existing clients do not break. Prefix on output is a per-type compatibility choice, not a second parse rule.

| Type | Size | Parse | Canonical display / serde |
|---|---|---|---|
| `Address` | 20 bytes | optional `0x` / `0X`, exactly 40 hex digits | `hex()` has no prefix; `Display` / serde / `hex_with_prefix()` is `0x` + lowercase |
| `ContentId`, `ManifestId` | 32 bytes | optional `0x` / `0X`, exactly 64 hex digits | `0x` + lowercase |
| `PublicKey` | 32 bytes (Ed25519) | optional `0x` / `0X` | lowercase hex, no `0x` |
| `ChallengeId`, `CapacitySeed`, `CapacityMerkleRoot` | 32 bytes | optional `0x` / `0X`, exactly 64 hex digits | lowercase hex, **no** `0x` (matches current P2P / proof tx edges) |

Typed IDs are for in-crate use. Some wire structs (`SyncMsg`, several tx fields) still use `String` or `[u8; 32]` at the edge; convert at those boundaries rather than changing the wire format in place.

Account addresses are `SHA-256(ed25519_verifying_key)[..20]`.
