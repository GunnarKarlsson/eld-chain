Understood — thank you for the clarification.

You want **a single hash** that serves both purposes at once:

- acts as the **content address** (for storage/deduplication where possible)
- provides **uniqueness per post** (different user / different nonce → different ID)

So the final decision is:

**message_id = content_hash = SHA-256(payload_bytes || user_pubkey || nonce)**

There is **only one hash field** in the protocol, and it is used as:

- the primary storage key for the payload bytes
- the primary lookup / index key for the message metadata / post record
- the value committed on-chain in the transaction

This means:

- same payload + same user + same nonce → same ID (replay / duplicate prevention)
- same payload + same user + different nonce → different ID (user can repost identical content multiple times)
- same payload + different user → different ID (different posters always distinct)
- no separate `content_hash` field → no automatic deduplication across different users (even if they post identical bytes, each gets its own copy in storage)

This is a clean, intentional trade-off that prioritizes **post-level uniqueness** over cross-user deduplication — which aligns perfectly with a public pinboard where each post is a distinct event, even if the text is the same.

### Revised Technical Design Document (single-hash version)

**Eld Pinboard Submission Protocol – Option 1 (Single-Hash Final)**  
**Authorized Content Upload with Validator Co-Signature**

**Version**: 1.3  
**Date**: conceptual revision March 2025  
**Status**: MVP-ready (pure Tendermint + custom ABCI app, no Cosmos SDK)

### Background & Principles

Eld stores only compact commitments on-chain. The full message payload lives in application state (CADO-style) and is pruned after expiry.

Key design choices for this version:
- Single deterministic hash = message ID = content address
- Hash input: payload_bytes || user_pubkey || nonce
- Every post is unique (different nonce or different user → different ID)
- Same user can post identical content multiple times (different nonce)
- Identical content from different users is stored separately (no cross-user deduplication)
- Only validators can submit valid `PostMessageTx` (enforced by co-signature)

### Core Data Structures

```protobuf
// Inner part — signed by the user
message MessageCommitment {
  bytes   user_pubkey           = 1;   // raw public key
  bytes   message_id            = 2;   // = SHA-256(payload_bytes || user_pubkey || nonce)
  uint64  expires_height        = 4;
  string  visibility            = 5;   // "Public" | "Encrypted"
  string  topic                 = 6;   // optional
  bytes   recipient_pubkey      = 7;   // optional for Encrypted
  uint64  nonce                 = 8;   // unique per user_pubkey
  uint64  fee_amount            = 9;   // exact amount user agrees to pay (in ueld)
  bytes   user_signature        = 10;  // signs fields 1,3–9 (canonical serialization)
}

// Full transaction sent to Tendermint
message PostMessageTx {
  // Inner user-signed commitment (copied verbatim)
  MessageCommitment commitment = 1;

  // Validator attestation
  bytes   validator_pubkey      = 2;
  int64   received_timestamp    = 3;   // unix seconds
  bytes   validator_signature   = 4;   // signs (hash(commitment) || received_timestamp || validator_pubkey)
}
```

### Protocol Flow

1. **Client preparation**  
   - Prepare payload, metadata, nonce, fee_amount
   - Compute:  
     `message_id = SHA-256(payload_bytes || user_pubkey || nonce)`
   - Sign `MessageCommitment` (excluding `message_id` from signature input, or include it after computation — choose one consistently)

2. **REST API call** – `POST /pinboard/submit` (to validator node)  
   ```json
   {
     "commitment": {
       "user_pubkey":      "0x...",
       "message_id":       "0x...",
       "expires_height":   12345678,
       "visibility":       "Public",
       "topic":            "announcements",
       "recipient_pubkey": null,
       "nonce":            42,
       "fee_amount":       1000
     },
     "user_signature":   "0x...",
     "payload":          "base64-encoded-full-payload"
   }
   ```

   Validator node:
   - Verify user_signature
   - Re-compute `message_id` from payload + user_pubkey + nonce → must match submitted value
   - Validate payload size/format/TTL
   - Cache payload (key = message_id, short TTL)
   - Build & sign `PostMessageTx` (add validator_pubkey + timestamp + signature)
   - Broadcast via `broadcast_tx_sync`
   - Return immediately:
     ```json
     {
       "tx_hash":      "0xabc123...",
       "message_id":   "0x...",
       "status":       "submitted",
       "validator":    "eldvaloper1..."
     }
     ```

3. **Confirmation**  
   Poll Tendermint RPC `/tx?hash=...`

4. **DeliverTx**  
   - Verify both signatures
   - Check validator in set, timestamp recent, nonce not replayed
   - Fetch cached payload by `message_id`
   - Deduct fee from user
   - Store payload + metadata at:  
     `/@eld/message/<hex-or-base58(message_id)>`
   - Update indexes:  
     `/@eld/by_sender/<user>/<message_id>`  
     `/@eld/by_expiry/<expires_height>/<message_id>`  
     ...
   - Clean cache

5. **Expiry**  
   Sweep deletes expired message records and indexes. No separate content layer → payload is removed together with the message record.

This version is now fully aligned with your requirement: **one hash**, **uniqueness per post**, **same-user reposting allowed**, **validator co-signature enforcement**.

Let me know if you want the canonical serialization rules for signing, base58 encoding spec for IDs, or pseudocode for the REST handler / DeliverTx checks.