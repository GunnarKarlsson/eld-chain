## Eld Post Message — Implementation Notes (Off-chain content, on-chain commitment)
**Goal**: allow a user to post a message via REST to a validator node. The validator verifies the user signature over the message + metadata, stores the message off-chain, and submits an on-chain `PostMessage` transaction (signed by the validator) that commits to the message via `message_id`. Message bytes are never part of the on-chain Tx.
This design intentionally does **not** require message blob availability during `CheckTx` / `DeliverTx`. Data availability is enforced later via best-effort P2P propagation and challenge/slashing rules.
---
## Terminology
- **User**: the original author of the message
- **Origin validator**: validator node that received the REST submission and broadcast the on-chain Tx
- **Message blob**: raw message bytes (or structured payload bytes) kept off-chain and deletable later
- **Commitment**: fields that are signed by the user and committed by the validator on-chain
- **message_id**: single hash that uniquely identifies a post and acts as the storage key
---
## Deterministic IDs & signing
### message_id (single-hash)
`message_id = SHA-256(message_bytes || user_pubkey_bytes || nonce_u64_le)`
Notes:
- Same message + same user + same nonce => same `message_id` (replay prevention)
- Same message + same user + different nonce => different `message_id` (reposts allowed)
- Same message + different user => different `message_id`
### User signature (covers everything, including message bytes)
User signs a canonical preimage that includes the message bytes and all commitment fields.
Recommended preimage:
- `domain_separator = "ELD_POST_MESSAGE_V1"`
- `signing_bytes = SHA-256(domain_separator || canonical_commitment_bytes || message_bytes)`
Where `canonical_commitment_bytes` encodes:
- `user_pubkey`
- `nonce`
- `expires_height`
- `visibility`
- `topic` (or empty)
- `recipient_pubkey` (or empty)
- `fee_amount`
- `message_id` (optional; if included, must match recomputed value)
Encoding rule: **must be fixed and unambiguous** (e.g. bincode or protobuf). Do not use ad-hoc JSON for this user signature unless you freeze canonical JSON rules.
---
## REST API (User -> Origin validator)
### Endpoint
`POST /v1/pinboard/messages:submit`
### Request (JSON)
```json
{
  "user_pubkey": "hex",
  "nonce": 42,
  "expires_height": 123456,
  "visibility": "Public",
  "topic": "optional",
  "recipient_pubkey": null,
  "fee_amount": "1000",
  "message_b64": "base64",
  "message_id": "hex",
  "user_signature": "hex",
  "idempotency_key": "optional-string"
}
Origin validator behavior
Decode message_b64 into message_bytes.
Recompute message_id from (message_bytes, user_pubkey, nonce). Must match request.
Verify user_signature over (commitment fields + message bytes) using user_pubkey.
Policy checks:
size limits, allowed visibility values
expiry bounds (min/max TTL)
optional: per-user / per-IP rate limits
optional: reject if (user_pubkey, nonce) already accepted recently (idempotency)
Store message blob off-chain keyed by message_id:
message_store[message_id] = { message_bytes, commitment_fields, received_timestamp, origin_validator_id }
Broadcast on-chain PostMessage transaction signed by the validator node key (see below).
Start best-effort P2P propagation: advertise message_id and serve blob to peers on request.
Response (JSON)
{
  "status": "submitted",
  "message_id": "hex",
  "tx_hash": "hex",
  "origin_validator": "0x...",
  "received_timestamp": 1710000000
}
Errors (suggested)
400 invalid_request (bad base64, missing fields)
401 invalid_user_signature
409 duplicate (same (user_pubkey, nonce) already accepted)
413 payload_too_large
422 message_id_mismatch
503 broadcast_failed
On-chain Tx (Origin validator -> chain)
New Payload type
Add a new Payload.type, e.g. "PostMessage".

Payload struct (commitment-only)
PostMessageTx (no message bytes):

sender: validator address (must match outer Tx signer address)
original_signer: user address (derived from original_signer_pubkey)
original_signer_pubkey: hex pubkey
message_id: hex (32 bytes)
nonce: u64
expires_height: u64
visibility: enum/string
topic: optional string
recipient_pubkey: optional hex pubkey
fee_amount: u128
received_timestamp: u64 (origin validator attestation time)
attested_user_signature: hex (optional but recommended for audit/evidence)
Outer Tx signature: standard chain Tx signature by validator key, as used elsewhere.

Consensus validation rules (CheckTx/DeliverTx)
Must be deterministic and must not require message blob access.

When processing PostMessageTx:

Verify outer Tx signature (existing logic).
Verify sender is in current validator set.
Validate fields:
message_id length/format
expires_height within allowed bounds
fee_amount non-negative and meets minimum policy (if any)
original_signer correctly derived from original_signer_pubkey (recommended)
Replay protection:
enforce (original_signer, nonce) monotonic or “not seen before”
enforce message_id uniqueness (no duplicate committed posts)
Apply state:
record message metadata keyed by message_id
index by sender/topic/expiry for queryability
debit fee from original_signer (if you charge at post time)
record origin_validator = sender for later availability responsibility
Do not verify attested_user_signature in consensus unless you also define canonical bytes in-chain and are comfortable with that dependency. If you do verify it in consensus, it still cannot include message bytes (since message bytes are off-chain), so it would only prove “user accepted commitment,” not “user signed message bytes.”

Off-chain propagation & retention
Origin validator obligations
Persist message_bytes for at least until expires_height (plus grace period).
Best-effort propagate to other validators via P2P:
announce message_id + commitment
serve message_bytes on request
optionally push to a bounded number of peers
Other validators
Listen for announcements.
Fetch/store blobs opportunistically based on capacity policy.
Participate in later availability/proof challenges.
Pruning
After expires_height, nodes may delete message_bytes.
On-chain metadata/indexes may also be swept/compacted if desired.
Data availability enforcement (challenge/slashing)
This design relies on protocol incentives rather than consensus-time blob checks.

Two slashable faults (recommended)
Unavailability fault: a validator responsible for a message_id fails to provide the message blob within a challenge window.
Invalid-attestation fault: the provided blob does not match the committed message_id (hash mismatch), or contradicts stated commitment fields.
Responsibility
Primary responsibility: origin_validator recorded on-chain in PostMessageTx.sender.
Replication responsibility: other validators may be challenged based on storage commitments / capacity proofs.
Evidence format (suggested, high-level)
message_id
the challenged validator identity
request transcript / timeout proof (or on-chain challenge-response protocol)
if provided: message_bytes and recomputed hash mismatch proof
(Exact mechanics depend on your existing capacity provider challenge system.)

Storage / indexes (suggested)
On-chain state (commitment metadata):

messages/<message_id> -> { original_signer, origin_validator, expires_height, visibility, topic, fee_amount, received_timestamp }
by_sender/<original_signer>/<message_id> -> true
by_expiry/<expires_height>/<message_id> -> true
optional: by_topic/<topic>/<message_id> -> true
Off-chain store (content):

blobs/<message_id> -> message_bytes
Open decisions
Canonical encoding for the user signature preimage (bincode/protobuf strongly preferred).
Whether to store attested_user_signature on-chain (recommended for audit/evidence).
Fee timing: charge at post time vs charge at retrieval/use.
Exact relationship between message posting and content-availability challenge system.

If you want, I can also align the proposed payload naming/field types to your existing `chain/common/src/tx.rs` conventions (e.g., `String` vs `u64`, address formatting with `0x` prefix) and to the existing transaction docs in `docs/transactions.md`.