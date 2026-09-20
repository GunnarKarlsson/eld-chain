## Eld Pinboard Protocol – Conceptual Overview

Eld is a lightweight blockchain protocol that provides an on-chain pinboard for **short‑lived messages**. Messages can be **public** (plaintext) or **private** (encrypted), are stored fully on-chain while live, and are removed from the live state after expiry. Node disk usage is kept bounded by a combination of **TTL-based expiry in application state** and **periodic pruning of historical blocks** aligned with snapshots.

This document is a design and planning document only. It describes how the protocol should work and how we will evolve the existing Eld/Eld codebase to implement it.

### Core message model

- **Message identity**
  - Each message has a deterministic `message_id`, derived from its content and metadata (e.g. hash of `(sender, created_height, body/ciphertext, topic, nonce)`).
  - The `message_id` is used as the primary key for storage and retrieval.

- **On-chain message record (conceptual fields)**
  - `sender`: Eld address of the poster (0x‑prefixed hex, derived from public key).
  - `message_id`: globally unique identifier.
  - `created_height`: block height at which the message becomes visible on the pinboard.
  - `expires_height`: block height after which the message is considered expired and no longer part of the live pinboard.
  - `visibility`: enum `{ Public, Encrypted }`.
  - `topic` (optional): application- or domain-level tag for filtering (e.g. `@chat/room/123`).
  - `body` (Public only): UTF‑8 bytes of the message body.
  - `recipient_public_key` (Encrypted only): hex‑encoded public key of the intended recipient.
  - `ciphertext` (Encrypted only): opaque bytes of the encrypted payload (arbitrary format, defined by apps).

Only the above metadata is interpreted at protocol level. Application payload semantics are intentionally minimal so Eld remains a general “ephemeral pinboard” primitive.

### Protocol-level support for encrypted messages

“Protocol-level” here means:

- The **core transaction type and validation rules** explicitly distinguish between **public** and **encrypted** messages.
- The **node enforces structural invariants** for encrypted messages (presence of recipient key and ciphertext, size limits, mutual exclusivity with plaintext) before accepting them into blocks.
- Encrypted messages are **first-class citizens** of the protocol: they use the same consensus, fee, TTL, and pruning mechanisms as public messages.

Concretely:

- For `visibility = Public`:
  - `body` must be present and non-empty.
  - `recipient_public_key` and `ciphertext` must be absent.
  - The node validates `body` size and basic UTF‑8 validity.

- For `visibility = Encrypted`:
  - `recipient_public_key` and `ciphertext` must both be present and non-empty.
  - `body` must be absent.
  - The node validates:
    - `recipient_public_key` format (hex, expected length).
    - `ciphertext` size and maximum length.
  - The node does **not** attempt to decrypt; encryption scheme and key management are left to wallets/apps.

Because all fields are on-chain, Eld provides **metadata privacy but not full anonymity**:

- Sender address, `created_height`, and `expires_height` are visible for all messages.
- For encrypted messages, only the ciphertext is visible; the contents remain opaque to the network.

### TTL and message lifecycle

Message lifetime is defined in **blocks**, not wall-clock time:

- Clients specify either:
  - `ttl_blocks` (desired lifetime in blocks), or
  - `expires_height` directly.
- The node converts `ttl_blocks` to `expires_height = created_height + ttl_blocks` during processing.

Constraints:

- `ttl_blocks` must be **at least** a small minimum (e.g. `MIN_TTL_BLOCKS`, to avoid accidental instant expiry).
- `ttl_blocks` must be **at most** `MAX_TTL_BLOCKS`, chosen so that:
  - It corresponds roughly to a maximum of **1 year** given the configured block time.
  - It is less than or equal to the configured snapshot/pruning horizon, so that nodes can still serve messages for their entire intended lifetime.

Lifecycle (conceptual):

1. **Post**: user or app constructs a `PostMessage` transaction (public or encrypted), signs it, and submits it to the network.
2. **Validation**:
   - Transaction passes generic Eld checks (size, structure, signature, dynamic fee).
   - Message-specific validation checks TTL bounds, visibility invariants, and size limits.
3. **Commit**:
   - On inclusion in a block, Eld records the message in its application state and indexes.
   - `created_height` is set to the block’s height.
4. **Live period**:
   - Until `current_height < expires_height`, the message is considered **live** and is returned by pinboard queries (subject to filters).
5. **Expiry in state**:
   - When `current_height >= expires_height`, the message becomes **expired**.
   - A periodic **expiry sweep** (run at regular intervals) removes expired messages and their associated indexes from application state.
6. **Historical visibility**:
   - Even after expiry from state, the original transaction remains part of historical blocks until those blocks are pruned.

This design makes “ephemeral” a **state-level** property (pinboard view), while still preserving full historical verifiability within the configured pruning window.

### Public vs private message behavior

At a high level:

- **Public messages**
  - Body is plaintext on-chain.
  - Any node or client can read the message during its live period (and from history, while blocks are retained).
  - Ideal for announcements, app status messages, one-time codes that are short-lived, etc.

- **Encrypted messages**
  - Protocol only sees sender, recipient public key, and ciphertext.
  - Only the holder of the corresponding private key (or any party the recipient delegates to) can decrypt.
  - Ideal for short-lived chats, tokens/links, or session handoffs between services.

Both are fully integrated:

- They share the same:
  - `PostMessage` transaction type.
  - TTL and fee rules.
  - indexing and pruning behavior.
- Clients can treat them uniformly at the API/CLI layer, differing only in how they construct and interpret the payload.

### Pruning and bounded chain size

Eld aims to keep **node disk usage bounded**, even as the number of messages grows without bound over time. It does this via two mechanisms:

1. **TTL-based expiry in application state**
   - Expired messages and their indexes are removed from the live state at regular intervals.
   - Snapshots taken after expiry contain **only live messages**, not the entire historical message set.

2. **Periodic block pruning aligned with snapshots**
   - Nodes periodically take application state snapshots (including all live messages at that time).
   - After a new snapshot is safely stored and verified, the node can **prune** historical blocks and old snapshots beyond a configured retention window.
   - Pruning is done at a **regular block interval**, not every block, to avoid constant churn and to simplify operations.

Conceptually:

- Let:
  - `SNAPSHOT_INTERVAL` = number of blocks between full state snapshots.
  - `PRUNE_INTERVAL` = number of blocks between pruning runs.
  - `BLOCK_RETENTION` = number of recent blocks each node keeps (e.g. last 10 000 blocks).
  - `MAX_TTL_BLOCKS` = maximum message TTL.

Constraints:

- `BLOCK_RETENTION` should be **greater than or equal to** `MAX_TTL_BLOCKS`, so that live messages can always be reconstructed from recent blocks if needed.
- `PRUNE_INTERVAL` is typically a multiple of `SNAPSHOT_INTERVAL`, so pruning only occurs after fresh snapshots are available.

Approximate disk footprint:

- **State + snapshots**:
  - Dominated by the number of **currently live messages** and recent snapshots.
  - Because expired messages are purged from state, this is roughly proportional to *“average posting rate × average TTL”* within configured limits.
- **Blocks**:
  - Bounded by `BLOCK_RETENTION × average block size`, which includes transactions for both public and encrypted messages.

With suitable configuration, Eld can support **unbounded total message volume over time** while keeping per-node disk usage within operational targets.

### Maximum message size and transaction limits

Eld already enforces **global transaction size limits** (e.g. ~10 MB JSON limit and smaller per-transaction byte caps for standard transactions). For the pinboard, we introduce tighter **per-message** limits:

- **Message body / ciphertext size**
  - A conservative starting point is:
    - `MAX_MESSAGE_BYTES_STANDARD` (e.g. 16 KB) for both public and encrypted messages.
  - This is enforced at:
    - Transaction validation time (reject over-sized payloads).
    - HTTP API layer (reject over-sized payloads before they reach consensus).

- **Overall transaction size**
  - The existing transaction size rules remain in place.
  - Pinboard messages must fit within both the per-message and global transaction size limits.

These limits ensure that:

- Individual messages stay small enough to be cheap to propagate and store.
- The worst-case per-block growth is predictable, simplifying capacity planning and pruning policy.

### Minimal Eld MVP: components and UX

The **minimal Eld MVP** should provide a complete vertical slice: developers can generate keys, run a node, post/read messages, and explore the chain and content without additional tooling.

- **CLI with wallet support**
  - Responsibilities:
    - Generate and manage wallets (keypairs), with safe local storage.
    - Construct, sign, and submit Eld transactions (including `PostMessage` for the pinboard).
    - Provide simple read commands (get account, list transactions, query pinboard messages).
  - UX goals:
    - One-command wallet creation and faucet request for local/dev networks.
    - Clear, copy-pasteable examples for posting public and encrypted messages with TTL.

- **Eld node**
  - Responsibilities:
    - Run consensus (via Tendermint/CometBFT).
    - Maintain application state (accounts, pinboard messages, snapshots, pruning).
    - Expose HTTP APIs for pinboard, content, and CADO queries.
  - UX goals:
    - Single `docker compose` / `cargo run` path to start a local devnet node.
    - Sensible defaults for snapshot/pruning and TTL so developers can ignore ops details initially.

- **Web explorer**
  - Responsibilities:
    - Visualize blocks, transactions, accounts, and pinboard messages.
    - Provide human-readable views of public messages and basic stats (message volume, TTL distributions).
  - UX goals:
    - “View the last N messages” and “inspect a message by ID” without CLI.
    - Clear indication when a message is expired vs live.

- **Web content-browser**
  - Responsibilities:
    - Browse content objects registered via Eld’s content APIs (for apps that combine pinboard + content).
    - Cross-link content entries to pinboard messages when relevant (e.g. file share metadata, chat room announcements).
  - UX goals:
    - Simple search and filter by content ID, owner, or tag.
    - Integrated story with the explorer so a developer can move from “block/tx view” to “content view” in one click.

Together, these four components define the smallest coherent Eld product:
- The **CLI** creates identities and issues transactions.
- The **node** enforces protocol rules (including encrypted messages, TTL, and pruning).
- The **explorer** makes the chain and pinboard human-visible.
- The **content-browser** surfaces higher-level content use cases on top of Eld’s primitives.

## Plan: Protocol and Architecture Changes (No Code Yet)

This section describes the **intended changes** to the Eld protocol and node architecture to realize the pinboard vision, including encrypted messages at protocol level. It is a design-only plan; no changes have been applied to the codebase yet.

### 1. New core transaction type: `PostMessage`

- Introduce a new core transaction type (conceptually named `PostMessage`):
  - Adds a new payload variant in the core transaction enum.
  - Adds a new transaction type constant string (e.g. `\"PostMessage\"`) to the allowed payload types.
  - Integrates with existing:
    - Signature verification (`Tx.verify`).
    - Dynamic fee calculation.
    - JSON structure validation.

- Validation rules (high level):
  - Required fields:
    - `sender`, `visibility`, message content (either `body` or `(recipient_public_key, ciphertext)`), and TTL (`ttl_blocks` or `expires_height`).
  - Enforce one-of:
    - If `visibility = Public`: require `body`; forbid `ciphertext` and `recipient_public_key`.
    - If `visibility = Encrypted`: require `ciphertext` and `recipient_public_key`; forbid `body`.
  - Enforce:
    - Address format for `sender`.
    - Public key format for `recipient_public_key`.
    - Size limits for `body`/`ciphertext`.
    - TTL bounds (0 < `ttl_blocks` ≤ `MAX_TTL_BLOCKS`).

This makes encrypted messages a **first-class protocol concept**, not an application-level convention.

### 2. State storage and indexing for messages

To support efficient retrieval and expiry, Eld will store message data and indexes in its existing key–value state (CADO/Trie + RocksDB), conceptually using paths like:

- Primary record:
  - `/@eld/message/{message_id}`
- Indexes: (maybe later, we don't support indexing yet)...
  - `/@eld/messages_by_sender/{sender}/{created_height}/{message_id}`
  - `/@eld/messages_by_expiry/{expires_height}/{message_id}`
  - Optional topic index:
    - `/@eld/messages_by_topic/{topic}/{created_height}/{message_id}`

For encrypted messages, the stored record includes only ciphertext plus necessary metadata; plaintext is never present on-chain.

These indexes enable:

- Fast listing of messages by sender, topic, or expiry.
- Efficient expiry sweeps keyed by `expires_height`.

### 3. Expiry sweeps at regular intervals

The node will implement an **expiry sweep** mechanism that runs at a configurable block interval:

- Trigger:
  - On block commit, if `current_height % EXPIRY_SWEEP_INTERVAL == 0`, run a sweep.
- Behavior:
  - Scan `/@eld/messages_by_expiry/{height≤current}`.
  - For each entry:
    - Delete the primary message record.
    - Delete the corresponding sender/topic index entries.

This ensures that expired messages are removed from live state in batches, keeping snapshots compact and avoiding per-block overhead from constant micro-cleanups.

### 4. Periodic snapshots and block pruning

Eld already supports application state snapshots and snapshot pruning in storage. For the pinboard:

- Confirm and (if needed) refine:
  - Snapshot cadence (e.g. `SNAPSHOT_INTERVAL`).
  - Snapshot retention policy (how many snapshots to keep).
- Align with a **pruning policy** on the consensus layer (Tendermint/CometBFT configuration):
  - `PRUNE_INTERVAL`: run pruning every N blocks.
  - `BLOCK_RETENTION`: retain the last K blocks.
- Ensure:
  - New snapshots are written before pruning beyond their coverage.
  - `BLOCK_RETENTION ≥ MAX_TTL_BLOCKS` so that live messages are always within the retained history window.

The net effect is that:

- Live pinboard state is always representable from a recent snapshot plus a bounded tail of blocks.
- Historical message bodies eventually disappear from the chain’s stored blocks, consistent with Eld’s “temporary messages, permanent infrastructure” ethos.

### 5. HTTP APIs and CLI surface for pinboard

Building on the existing content APIs and CLI:

- Add HTTP endpoints:
  - `POST /pinboard/message` – submit a new public or encrypted message.
  - `GET /pinboard/message/{message_id}` – fetch a single message (if still live).
  - `GET /pinboard/messages` – list live messages filtered by sender, topic, time window, etc.
  - Apply the same `ApiError` response shape and rate-limiting (`RateLimitState`) as the current content and CADO endpoints.

- Extend Eld CLI:
  - `post-message` – creates a public message tx.
  - `post-encrypted-message` – creates an encrypted message tx given a recipient address or public key.
  - `get-message`, `list-messages` – convenience wrappers around the HTTP/ABCI APIs.

These APIs do not add new protocol semantics but make the pinboard primitive easy to consume.

## Phased Implementation Plan

This section outlines an incremental, testable rollout plan. Each phase assumes code changes will be made in future work; this document is only specifying the intended phases and their scope.

### Phase 1 – Formal spec and documentation (this document + refinements)

- Finalize:
  - `PostMessage` transaction schema and validation rules.
  - Protocol-level semantics for encrypted messages (one-of rules, key/ciphertext handling).
  - TTL model (`ttl_blocks` vs `expires_height`), `MAX_TTL_BLOCKS`, and interaction with pruning.
  - Message size limits (`MAX_MESSAGE_BYTES_STANDARD`) and relationship to global transaction limits.
  - High-level pruning policy (`SNAPSHOT_INTERVAL`, `PRUNE_INTERVAL`, `BLOCK_RETENTION`).
- Deliverables:
  - This protocol overview and plan document in `docs/`.
  - Any supplementary diagrams or example payloads as needed.

### Phase 2 – Core protocol wiring (transaction + validation, no API yet)

- Implement the new `PostMessage` transaction type in the core transaction model.
- Add message-specific validation rules (public vs encrypted, TTL bounds, size limits).
- Integrate with the node’s deliver_tx pipeline:
  - Ensure signature verification, fee calculation, and JSON structure validation all cover `PostMessage`.
- Add placeholder handler stubs in the consensus processor (without public APIs yet), able to:
  - Persist message records and indexes in state.
  - Log and emit basic events for debugging.

### Phase 3 – State expiry and snapshots alignment

- Implement the expiry sweep mechanism:
  - Data structures for `/@eld/messages_by_expiry/...`.
  - Periodic deletion at `EXPIRY_SWEEP_INTERVAL`.
- Review and, if necessary, adjust snapshot writing cadence to ensure:
  - Snapshots include only live messages.
  - Operators can safely prune up to a given height without losing required message state.

### Phase 4 – Pruning configuration and operator runbook

- Configure Tendermint/CometBFT pruning parameters per environment:
  - Choose concrete values for `SNAPSHOT_INTERVAL`, `PRUNE_INTERVAL`, and `BLOCK_RETENTION`.
  - Validate that `BLOCK_RETENTION ≥ MAX_TTL_BLOCKS`.
- Document:
  - How to operate an Eld node with pruning enabled.
  - Expected disk usage patterns under typical and heavy message loads.
  - How snapshots, pruning, and message TTL interact.

### Phase 5 – Pinboard HTTP/CLI interfaces

- Add HTTP endpoints for posting and querying pinboard messages, reusing existing content API patterns:
  - Standardized error responses and rate limiting.
  - Input validation that mirrors the on-chain `PostMessage` rules.
- Extend Eld CLI to expose:
  - `post-message`, `post-encrypted-message`, `get-message`, `list-messages`.
- Provide examples in docs:
  - How to post public vs encrypted messages.
  - How to pick TTLs.
  - How to interpret responses and error codes.

### Phase 6 – Hardening, observability, and UX polish

- Add metrics and logs:
  - Message posting rates, TTL distributions, expiry sweep performance, pruning events.
- Add safeguards:
  - Better error messages on invalid TTLs or size limits.
  - Optional “dry run” or local validation commands for clients.
- Iterate on defaults:
  - Tune `MAX_MESSAGE_BYTES_STANDARD`, `MAX_TTL_BLOCKS`, and pruning settings based on real-world usage and benchmarks.

Once these phases are complete, Eld will have a clearly specified and implemented **protocol-level pinboard** with **first-class encrypted messages**, **TTL-based ephemerality**, and **bounded node storage** consistent with the product vision articulated in `_INTRO.md`.

