## Eld Use Cases

Eld is an on-chain pinboard for **short‑lived messages** with **public** and **protocol-level encrypted** message types plus TTL (expiry). Below are example application patterns that fit Eld’s “post it now, forget it later” model.

### 1) Private encrypted messages (recipient-only readability)

**Problem**: A dApp needs to deliver short secrets (one-time links, session tokens, invites, approvals) such that they **cannot be read by intermediaries** (RPC providers, indexers, relayers, validators, or other chain observers).

**How Eld fits**:
- The app posts an **Encrypted** Eld message:
  - Includes `recipient_public_key` and `ciphertext`.
  - Network enforces that encrypted messages have the correct structure (no plaintext body).
  - Only the recipient’s private key can decrypt the ciphertext.
- TTL ensures the secret is ephemeral:
  - Example TTL: 20 blocks for a short-lived approval; or a few hours/days for an invite.

**Security properties (what you get / don’t get)**:
- **Confidentiality**: strong (ciphertext only; content is opaque on-chain).
- **Integrity**: strong (tx is signed by sender; message is committed by consensus).
- **Metadata privacy**: limited (sender, recipient public key, and timing are visible).

**Typical flow**:
1. Recipient publishes (or shares) their public key.
2. Sender encrypts the payload off-chain and posts an encrypted Eld message with a TTL.
3. Recipient scans for encrypted messages addressed to their public key (or its hash) and decrypts.

### 2) File sharing app (ephemeral file pointers and access grants)

**Problem**: A file sharing app wants to share files temporarily. The files themselves may be too large for on-chain storage, but **access control and time-limited sharing** should be decentralized and auditable.

**How Eld fits**:
- Store the *file bytes* off-chain (S3, IPFS, Arweave, a content network, etc.).
- Use Eld to store **time-limited metadata** about the file:
  - Content address (hash/CID), filename, MIME type, size.
  - Access policy hints (public link vs recipient-limited).
  - Optional: a decryption key or capability token **encrypted** to the recipient.
- TTL represents link validity:
  - Example TTL: 10k blocks (~days) for a temporary share link.

**Two common patterns**:
- **Public share**: public Eld message with `{cid, name, size}` and TTL.
- **Private share**: encrypted Eld message containing `{cid, key/capability, metadata}` so only the recipient can access/decrypt.

**Why this is valuable**:
- The “share grant” becomes a durable on-chain receipt during its lifetime.
- The grant expires automatically from the pinboard view and can be pruned with retention policies.

### 3) Game data cache (fast, ephemeral state for dApps)

**Problem**: A game needs a decentralized “scratchpad” for short-lived data:
- Matchmaking tickets
- Temporary player buffs
- Daily/weekly events
- Serverless coordination signals between clients

Storing all of this permanently on typical chains is expensive and causes permanent bloat.

**How Eld fits**:
- Use Eld public messages for non-secret coordination:
  - Example: “matchmaking ticket for region=EU, rank=Gold, expires in 300 blocks”.
- Use Eld encrypted messages for private coordination:
  - Example: shared seed, ephemeral room key, or a time-limited invitation encrypted to a player’s pubkey.
- TTL models the natural lifetime of “game session data”.

**Example**:
- A player posts a ticket (TTL 200 blocks).
- A peer replies with an encrypted room key (TTL 50 blocks).
- After the match starts, all of this naturally expires and is removed from live state.

### 4) Chat app (with roll-ups so every message isn’t a transaction)

**Problem**: Traditional “one message = one on-chain tx” chat is costly and noisy. A practical chat needs:\n- Many messages per minute\n- Near-real-time UX\n- On-chain anchoring for integrity, but not full on-chain throughput for every message\n\n+**How Eld fits**: use **roll-ups** (batching) so the chain sees periodic commitments, not every message.

#### Option A — Client-side batch roll-up (simple, pragmatic)

- Chat messages are exchanged off-chain (p2p, websocket relay, any transport).
- Periodically (e.g. every 20 seconds or every 50 messages), a designated “roller” (could rotate) posts an Eld message containing:\n  - A **Merkle root** (or hash chain tip) committing to the batch\n  - Minimal metadata (room id, batch sequence number, time window)\n  - Optionally a compressed bundle of encrypted messages if it fits size limits\n\n+**On-chain**: one Eld tx per batch.\n+**Off-chain**: the actual messages are distributed out-of-band; anyone can verify inclusion using Merkle proofs against the posted root.\n\n+**Pros**:\n+- Very low on-chain volume\n+- Easy to implement incrementally\n+\n+**Cons**:\n+- Requires a data availability strategy (participants keep the messages, or use an ephemeral relay)\n\n+#### Option B — Hash-chain anchoring (even lighter than Merkle trees)\n+\n+- Each message includes `prev_hash`, forming a signed hash chain.\n+- Every N messages, post only the latest chain tip (hash) to Eld.\n+- Participants who have the off-chain log can verify continuity and detect tampering.\n+\n+**Pros**:\n+- Minimal computation and smallest on-chain footprint\n+\n+**Cons**:\n+- Proving inclusion of an arbitrary message is weaker than Merkle trees unless you share the full chain segment\n+\n+#### Option C — Hybrid: occasional on-chain ciphertext bundles\n+\n+- For small rooms / low volume, you can include an encrypted “bundle” on-chain within a single Eld message:\n+  - `ciphertext = compress(encrypt([m1, m2, ...]))`\n+  - TTL covers how long the room expects the bundle to remain retrievable.\n+- For high volume rooms, fall back to Option A or B.\n+\n+#### Practical recommendation\n+\n+- Start with **Option A (Merkle root roll-ups)**:\n+  - It gives strong integrity and inclusion proofs.\n+  - The chain only stores a compact commitment per time slice.\n+  - It’s compatible with Eld’s TTL and pruning: commitments can be short-lived while still enabling “during-session verifiability”.\n+\n+In all roll-up options, private chat is achieved by encrypting message payloads end-to-end (E2EE) and using Eld primarily for **commitments, discovery, and ephemeral anchoring** rather than raw per-message storage.\n+
