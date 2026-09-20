Metadata uniqueness
Identify posts (what goes on-chain / in indexes) by originator + commitment, not by bytes alone.
Blob identity is a content key (e.g. hash of message_bytes / CID): same octets → same key for everyone.
Logical post is a row keyed by things like original_signer + content_key (and any fields you require to be unique—e.g. instance/nonce only if you allow the same user to post the same blob twice with identical policy fields).
Per-post fields (TTL / expires_height, visibility, topic, fee, …) live on the metadata/post row, so two users posting "test" are two posts, same content_key, different metadata.
Blob sharing
Store payload bytes once per content_key in local blob/object storage.
Each PostMessage (or equivalent) references that content_key; no need to duplicate bytes when another originator posts the same content.
Refcount
Keep a content_key → refcount (or content_key → set of active post ids) in persisted application state (same DB family as the rest of consensus state, e.g. RocksDB)—not embedded in the blob file.
On PostMessage commit (deliver_tx): insert/update post metadata and increment refcount (or add a reference) for that content_key.
When a post expires (usually processed in end_block or a height/time sweep using TTL in metadata): remove that post row and decrement refcount (or remove that post id from the set).
All validators that apply the same block sequence update this table the same way → deterministic, no separate P2P “refcount sync.”
When to delete the blob
After handling expiry (or any rule that removes the last reference), if refcount is 0 (or the reference set is empty), the node may delete the local blob for that content_key.
Deletion is a local GC optimization; authority for “is anything still committed?” comes from chain-driven metadata/refcount, not from scanning arbitrary disk headers.
That’s the approach: unique posts in state, shared blobs by content key, refcount in app DB from chain execution, delete blob locally when committed references drop to zero.

## Wire format (implemented; storage/refcount later)

- **content_key**: `hex(SHA-256(message_bytes))`. Carried in `PostMessageUserRequest` / `PostMessageTx`; identical message bytes from any originator share this value.
- **tags**: At most 4 strings; each tag at most 64 UTF-8 bytes. If a tag parses as a chain address, it is normalized to `0x` + lowercase hex (20-byte address). Carried in `PostMessageUserRequest` / `PostMessageTx` for app-level filtering (e.g. dapp inbox).
- **User signing preimage**: `serde_json(PostMessageUserSigningPayload) || message_bytes` where the JSON object includes `content_key`, `original_signer`, `original_signer_pubkey`, `expires_height`, `visibility`, `topic`, `tags`, `fee_amount`.
- **message_id**: `hex(SHA-256(user_signing_preimage))` — unique per commitment (any field in the JSON or different `message_bytes` changes the id).