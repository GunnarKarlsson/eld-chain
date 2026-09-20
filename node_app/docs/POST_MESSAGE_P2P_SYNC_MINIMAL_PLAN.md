# PostMessage P2P Blob Sync Minimal Plan

## Goal

Keep content data off-chain while making `PostMessage` handling deterministic and eventually consistent across validators.

Target behavior:

1. Upload node stores payload bytes in temp blob storage.
2. Upload node submits `PostMessage` tx.
3. All validators process the tx successfully based on tx validity, not blob locality.
4. Validators that do not have the blob request it from peers via P2P.
5. On receipt, validators verify and store as confirmed blob.

## Scope (phase 1)

- Keep blob storage in existing pinboard DB (`pinboard_temp_blob`, `pinboard_blob`).
- Do not couple this flow to capacity slot storage yet.
- Reuse existing P2P message types (`Announce`, `ContentRequest`, `ContentResponse`).
- Keep periodic sync retries as backup.

## Current issue to fix

Current `DeliverTx` for `PostMessage` fails when neither temp nor confirmed blob exists locally.  
This causes validators that did not receive the REST upload to reject tx execution instead of syncing asynchronously.

## Proposed design

### 1) DeliverTx must not fail on missing blob

In `process_post_message_tx`:

- Continue validating tx and staging metadata/index/refcount updates.
- If blob is missing (`temp` and `confirmed` both absent), do **not** return tx failure.
- Record the blob as missing and trigger a P2P request for `content_key`.

Result: all validators can process the same block deterministically.

### 2) Missing blob tracking + immediate request

When missing blob is detected during tx processing:

- Add/update missing tracker entry keyed by `content_key`.
- Immediately broadcast `ContentRequest { content_id: content_key }`.
- Keep periodic retry loop to request unresolved items.

### 3) P2P response stores into pinboard blob DB

On `ContentResponse`:

- Decode base64 payload.
- Verify hash: `sha256(decoded_bytes) == content_key`.
- If valid and blob not already present, write to `pinboard_blob(content_key)`.
- Remove missing tracker entry.

Important: For this phase, do not store response data in capacity slots for pinboard sync.

### 4) Serving requests

On `ContentRequest`:

- First try `pinboard_blob(content_key)`.
- Optional fallback: `pinboard_temp_blob(content_key)` for fast propagation before promotion.
- Publish `ContentResponse` with base64 bytes.

### 5) Commit behavior

Keep existing temp -> confirmed promotion logic, but treat missing blob as non-fatal:

- Log missing/illegal state clearly.
- Do not panic and do not block unrelated commit writes.

Metadata/index writes still proceed.

## Query/read behavior during convergence

If metadata exists but blob is still syncing:

- Return metadata with explicit "blob pending sync" status (or equivalent non-fatal response).
- Clients can retry until blob arrives.

## Minimal implementation steps

1. Relax missing-blob validation gate in `post_message_processor`.
2. Add "missing blob -> request via P2P" path from tx processing.
3. Ensure `ContentResponse` path verifies hash and writes to `pinboard_blob`.
4. Keep/update missing tracker entries by `content_key`.
5. Add logs/metrics for:
   - missing at tx processing
   - request sent
   - response verified
   - blob persisted
   - sync completion

## Invariants

- Tx consensus outcome must not depend on local blob presence.
- Blob is accepted only if hash matches `content_key`.
- Content bytes remain off-chain.
- Sync remains eventually consistent via immediate request + periodic retry.

## Out of scope (phase 1)

- Capacity reservation / slot-map integration.
- Slashing or economic enforcement changes.
- Large refactors of P2P message schema.

## Acceptance criteria (end goal)

After `PostMessage` tx confirmation, content should be readable from all validators via REST within a few seconds.

Suggested SLO targets:

- P95 cross-node blob availability: <= 10 seconds after tx confirmation.
- P99 cross-node blob availability: <= 20 seconds after tx confirmation.
- No validator returns permanent "blob missing" for a confirmed post unless sync timeout is exceeded.

## REST test plan (multi-node)

1. Submit pinboard message to node 1 REST endpoint.
2. Wait for tx confirmation from Tendermint RPC.
3. Start polling node 2-4 REST/ABCI read endpoint for the same `message_id`.
4. Stop per node when read succeeds and record latency (`confirm_ts` -> `read_success_ts`).
5. Fail test if any node does not succeed within timeout (for example 20 seconds).

Recommended rollout:

- Phase A: keep warn-only while tuning.
- Phase B: switch to hard-fail in CI/local smoke once latency is stable.

## Required observability

For each `message_id` / `content_key`, log timestamps for:

- tx confirmed
- first `ContentRequest` sent
- first `ContentResponse` received
- blob persisted to `pinboard_blob`
- first successful read on each peer node (in test script)

This provides direct measurement of end-to-end sync latency.

IMPORTANT 

- block confirmation should never be prevented in delivertx or commit consensus methods because of the new logic. any errors are logged for tx handling but block confirmation must always happen. so make sure the consensus code doesnt panic or pends due to your new code.