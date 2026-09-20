# PostMessage Temp Blob Design (Concise)

## Goal

Keep blob bytes **off-chain** and out of `PostMessageTx`, but only persist blobs to the
**confirmed blob store** once the tx is committed.

## Stores

- **Temp blob store** (new): short-lived staging area for uploaded bytes.
- **Confirmed blob store** (existing pinboard blobs by `content_key`): durable bytes referenced by committed metadata.

## IDs and lookup keys

- **`message_id`**: unique post commitment id (metadata identity).
- **`content_key`**: `sha256(message_bytes)` (blob identity).

Rule: post retrieval path uses `message_id`; blob bytes are resolved through metadata `content_key`.

## Flow

### 1) Upload (REST submit)

1. Decode and validate `message_b64`.
2. Verify `sha256(message_bytes) == user.content_key`.
3. If `confirmed_blob[content_key]` already exists:
   - skip temp write (dedup by content hash).
4. Else:
   - write to `temp_blob[content_key]` (or `temp_blob[message_id]`, see keying note below),
   - store short TTL metadata for GC (e.g. `expire_at_unix` or `expire_at_height`).
5. Build/sign/broadcast `PostMessageTx` (still no blob in tx).

### 2) deliver_tx processor

Existing behavior stays: stage pinboard metadata/index/refcount deltas.

Additional validation gate:

- If neither `temp_blob[content_key]` nor `confirmed_blob[content_key]` exists,
  treat as invalid post data:
  - return tx failure code (invalid state for a fresh post),
  - log explicit error with `message_id` + `content_key`.

### 3) commit()

For each staged post metadata:

1. If `confirmed_blob[content_key]` exists:
   - do nothing for blob bytes (already durable),
   - continue metadata/index/refcount writes.
2. Else if `temp_blob[content_key]` exists:
   - copy/move bytes into `confirmed_blob[content_key]`,
   - delete temp entry.
3. Else (illegal state):
   - log error (do not panic),
   - continue commit (node remains healthy).

Then apply metadata/index/refcount logic as today.

## Illegal-state policy

Illegal state = committed/staged post references a `content_key` that exists in neither temp nor confirmed store.

- **In tx processing**: fail tx if this is detected.
- **In commit**: log-and-continue (never panic in `commit()`), because commit must not block unrelated writes.

## Temp blob TTL / GC

Temp entries carry a short TTL and are removed by a future GC pass.

Recommended minimal index:

- `temp_blob_expiry:{expire_at}:{content_key}` -> empty

GC scans by expiry and deletes:

- temp blob bytes
- temp expiry index rows

GC is best-effort and independent from consensus critical path.

## Keying note (temp store)

Prefer keying temp bytes by `content_key` for natural dedup. If idempotency/audit by submission is needed,
add optional side index:

- `temp_blob_by_message:{message_id}` -> `content_key`

This keeps dedup efficient while preserving message-level traceability.

## Minimal DB additions

- `pinboard_temp_blobs` CF: `content_key -> blob bytes`
- `pinboard_temp_blob_expiry` CF: `expire_at + content_key -> empty`
- (optional) `pinboard_temp_blob_by_message` CF: `message_id -> content_key`

## Determinism and safety

- Tx remains small (hash references only).
- Blob persistence to confirmed store happens only after/with commit handling.
- `commit()` never panics on pinboard blob issues; errors are logged and isolated from other state writes.
