# Eld Pinboard Garbage Collector (Blob GC) — Design

## Goal

Bound node disk usage by deleting **offchain pinboard message bytes** ("blobs") after their TTL has
expired, while keeping **transaction-derived metadata and indexes forever**.

This is a **best-effort**, **non-blocking** garbage collector that runs on a **separate thread** and
must never block block production.

## Non-goals / Explicit constraints

- **Never delete anything derived from a committed transaction**:
  - Pinboard metadata rows are permanent.
  - Secondary indexes (wallet/tag/expiry/commit indexes) are permanent.
  - Anything that was included in a tx is permanent.
- **GC is not a data availability mechanism for queries**. Queries are driven by TTL.
- GC may lag behind chain height. It does not need to finish before the next block.
- GC must be safe under node restarts and should be easy to monitor.

## Authoritative query behavior (TTL-gated)

Pinboard queries MUST gate message-body availability using metadata TTL only:

- If `current_height >= meta.expires_height`:
  - The query MUST NOT attempt to load the blob bytes (even if the blob still exists).
  - Return an "expired" response (e.g. `message_b64: null` or `410 Gone` depending on API shape).
- If `current_height < meta.expires_height`:
  - The query MAY attempt to load the blob bytes.
  - If blob is missing here, it is a storage inconsistency (operational error), not "normal expiry".

Rationale:
- This decouples API semantics from GC timing.
- It eliminates query/GC races: deleting blobs cannot break expired-message reads because expired
  reads never fetch blobs.

Implementation note:
- The node must make `current_height` available to the ABCI info query path (either by passing it
  in from the ABCI handler, or by reading a stored "last committed height" key).

## Storage model recap

### Permanent data (never deleted)

- `PinboardMessageMetadata` by `message_id` (includes `content_key` and `expires_height`).
- Secondary indexes (wallet/tag/expiry/commit ordering).

### Ephemeral data (eligible for deletion)

- Pinboard blob bytes by `content_key` (confirmed blob storage).
- Temp blob storage used by upload subsystem (`PINBOARD_TEMP_BLOB_TTL_SECS`).

## Blob sharing and deletion safety

Blobs are content-addressed (`content_key = hash(message_bytes)`), so multiple posts MAY reference
the same blob.

**Deletion rule:**

Delete the blob bytes for `content_key` only when **all posts that reference it are expired**.

Because metadata/index entries are permanent, the GC cannot rely on "decrementing references by
deleting metadata". Instead, it must compute safety using one of the approaches below.

## Two equivalent approaches (choose one at implementation time)

### Approach A (recommended): `content_key -> max_expires_height` (simple, monotonic)

Maintain a persisted mapping:

- `pinboard:blob_max_expires:{content_key} -> u64(max_expires_height)`

On every `PostMessage` commit, update:

- `blob_max_expires[content_key] = max(blob_max_expires[content_key], meta.expires_height)`

Then blob deletion is safe when:

- `current_height >= blob_max_expires[content_key]`

**Important nuance (content reuse):**

If the same `content_key` is posted again later with a later expiry, `blob_max_expires` increases.
Therefore, the GC MUST re-read `blob_max_expires` immediately before deletion (or use a CAS-style
check) to avoid deleting a blob that was extended after it was queued.

Pros:
- Extremely simple and monitorable.
- No need to track per-post reference sets or decrement counters on expiry.

Cons:
- Requires adding and maintaining a new keyspace.

### Approach B: refcount + expiry accounting (more moving parts)

Maintain:

- `refcount[content_key]` incremented on commit.
- A mechanism to decrement the refcount when a post *expires* (but metadata is not deleted).

This requires a deterministic, height-driven "expiry accounting" step to apply decrements exactly
once per post. Even if offchain deletion is on a background thread, the expiry-accounting step must
be restart-safe and avoid double-decrement.

Pros:
- Supports more complex policies later.

Cons:
- More complexity than needed if TTL-gated queries are the only availability rule.

## Background GC thread ("game loop")

### Thread model

Run a dedicated long-lived thread (or Tokio task) that:

- Wakes up every `GC_TICK_SECS` (e.g. 2–10 seconds).
- Reads latest committed chain height `H`.
- Performs a bounded amount of work (`MAX_DELETES_PER_TICK`, `MAX_KEYS_SCANNED_PER_TICK`, or a time
  budget).
- Persists progress (cursor) frequently.

The GC thread must never block consensus/block production.

### Input enumeration (candidates)

The GC needs an efficient way to enumerate content keys that might now be deletable.

If using **Approach A**, simplest is to iterate the `blob_max_expires` mapping itself:

- Iterate keys in lexicographic order with a persisted cursor:
  - `pinboard:gc:cursor:blob_max_expires -> last_key_bytes`

For each `content_key`, read `max_expires_height` and delete if `H >= max_expires_height`.

If iterating `blob_max_expires` is too expensive, add an auxiliary index:

- `pinboard:idx:blob_expiry:{max_expires_height_be}:{content_key} -> empty`

This allows scanning up to height `H` similarly to the existing message expiry index.

### Cursor / restart safety

Persist a cursor so the GC resumes after restart without re-scanning everything:

- `pinboard:gc:cursor:{name} -> opaque_last_key` (bytes)
- `pinboard:gc:last_tick_ts -> u64`
- `pinboard:gc:last_seen_height -> u64`

If cursor missing/corrupt:
- Fall back to scanning from the beginning.

### Idempotency rules

Deletion must be safe to retry:

- Deleting a non-existent blob is treated as success.
- If deletion fails (IO error), leave the cursor *before* the failed key so it is retried next tick.

### Bounding and backpressure

Per tick:

- Stop after `MAX_DELETES_PER_TICK` successful deletions.
- Stop after `MAX_KEYS_SCANNED_PER_TICK` examined keys.
- Optionally stop after `MAX_TICK_MILLIS` elapsed time.

This ensures predictable CPU/IO usage and easy monitoring.

## Temp blob sweeper (upload subsystem)

Temp blobs are governed by wall-clock TTL (e.g. `PINBOARD_TEMP_BLOB_TTL_SECS`), not chain height.

Implement a separate best-effort sweeper (can share the same GC thread) that:

- Iterates temp blob keys by prefix.
- Deletes entries with `now >= expires_at_timestamp`.
- Uses its own cursor and per-tick caps.

This is node-local and must not affect consensus.

## Monitoring and metrics

Expose the following in logs and metrics:

- `gc_last_seen_height`
- `gc_cursor` (or a hash/shortened cursor for readability)
- `gc_lag_blocks` (if using expiry-index scan) or `gc_progress_ratio` (if scanning blob_max_expires)
- `gc_keys_scanned_total`
- `gc_blobs_deleted_total`
- `gc_blob_delete_errors_total`
- `gc_temp_blobs_deleted_total`
- `gc_tick_duration_ms` (histogram)

Alerting recommendations:

- Error rate sustained > 0
- Tick duration consistently near budget
- Lag grows without bound (indicates worker stuck)

## Failure modes and expected behavior

- GC falls behind: disk usage grows temporarily; queries remain correct (TTL-gated).
- Node restarts: GC resumes from cursor; if cursor lost, it re-scans from start (safe but slower).
- Blob missing for unexpired message: operational error; indicates storage loss or misconfigured GC.
- Blob exists for expired message: normal; GC is best-effort and may lag.

## Implementation checklist (future work)

1. **Query change**: gate blob reads on `expires_height` and `current_height`.
2. **Height source**: provide committed height to info queries (store `last_committed_height`).
3. **RocksDB support**:
   - Add a CF or prefix for `blob_max_expires` (Approach A) and cursor keys.
   - Add a delete method for confirmed pinboard blobs (currently only temp-blob delete exists).
4. **Commit-time update**:
   - Update `blob_max_expires[content_key] = max(old, expires_height)` when staging/committing a
     `PostMessage`.
5. **GC thread**:
   - tick loop + cursor + caps + metrics
   - delete confirmed blobs when eligible
   - optionally include temp-blob sweeper

