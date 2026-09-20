Design proposal: validator node verification
Problem summary
Any address can stake and be added to the validators list
Active validators are selected by stake without verifying they correspond to running nodes
Tendermint expects all validators in the set to participate, causing stalls if a validator isn't running
Proposed solution
Phase 1: Node registry — track registered nodes
Concept: Maintain a registry of nodes that have called init_chain (i.e., nodes that have started and registered).
Implementation:
// In AppStateEnvelopepub struct AppStateEnvelope {    // ... existing fields ...        // NEW: Registry of nodes that have called init_chain    pub registered_nodes: BTreeMap<String, RegisteredNodeInfo>, // address -> node info}#[derive(Debug, Clone, Serialize, Deserialize)]pub struct RegisteredNodeInfo {    pub validator_address: String,      // Address derived from validator public key    pub validator_public_key: Vec<u8>,  // Ed25519 public key (32 bytes)    pub registered_at_block: i64,       // Block height when node registered    pub last_seen_block: i64,           // Last block where node participated (updated on votes/proposals)}
Where to populate:
In init_chain (lines 555-631 in consensus.rs): When a node calls init_chain, add it to registered_nodes with its validator address and public key.
Phase 2: Staking validation — verify staker is a registered node
Concept: When processing StakeTx, verify the staker's public key matches a registered node's validator key.
Implementation:
// In process_stake (processor.rs, around line 2415)// When processing StakeTx:1. Extract public_key from StakeTx2. Derive address from public_key: Address::from_public_key(public_key)3. Check if this address exists in registered_nodes:   - If NO: Reject stake transaction with error "Address does not correspond to a registered node"   - If YES: Verify the public_key matches the registered node's validator_public_key     - If mismatch: Reject with error "Public key does not match registered node"     - If match: Allow staking and add to validators list
Rationale: Only nodes that have started and called init_chain can stake. This ensures the address corresponds to a running node.
Phase 3: Live validator check — verify validators are participating
Concept: Before selecting active validators, filter to only those that are "live" (recently participated in consensus).
Two approaches:
Option A: Tendermint validator set (recommended)
Use Tendermint's current validator set as the source of truth
Only select validators whose addresses match validators in Tendermint's current set
Tendermint already tracks which validators are participating
Option B: Participation tracking
Track last_seen_block for each validator
Update last_seen_block when a validator votes/proposes
Filter out validators that haven't been seen in the last N blocks (e.g., 20 blocks = 1 epoch)
Implementation:
// In select_validators_for_epoch (consensus.rs, line 214)fn select_validators_for_epoch(&self, current_state: &mut AppState) {    // Get all validators    let mut validators = current_state.envelope.validators.clone();        // OPTION A: Filter by Tendermint's current validator set    // (Need to get this from Tendermint somehow - might require ABCI query or state)        // OPTION B: Filter by participation    let current_block = current_state.envelope.block_height;    let max_staleness = BLOCKS_PER_EPOCH; // e.g., 20 blocks    validators.retain(|v| {        if let Some(node_info) = current_state.envelope.registered_nodes.get(&v.address) {            (current_block - node_info.last_seen_block) <= max_staleness        } else {            false // Not a registered node        }    });        // Sort by stake (descending)    validators.sort_by(|a, b| b.stake.cmp(&a.stake));        // Take top VALIDATORS_PER_EPOCH    current_state.envelope.active_validators = validators        .into_iter()        .take(VALIDATORS_PER_EPOCH)        .collect();}
Updating last_seen_block:
In begin_block: Check if proposer is a registered node, update last_seen_block
In end_block: Check validator updates, update last_seen_block for all validators in the set
Or: Query Tendermint's validator set periodically to see who's actually participating
Design decisions
Node registry vs Tendermint state
Registry: Explicit tracking, easier to query
Tendermint state: Source of truth, but requires integration
Recommendation: Use both — registry for staking validation, Tendermint state for live checks
Staleness threshold
How many blocks without participation before considering a validator "dead"?
Recommendation: BLOCKS_PER_EPOCH (20 blocks) — if a validator misses an entire epoch, they're likely down
Handling node restarts
A node that restarts will call init_chain again
Update registered_at_block and last_seen_block on re-registration
Allow re-staking if the node was previously registered
Edge cases
What if a node registers but never stakes? → Not a problem, they won't be selected
What if a node stakes but then goes offline? → Participation tracking filters them out
What if a node restarts mid-epoch? → They'll re-register, but won't be selected until next epoch
Implementation order
Add registered_nodes to AppStateEnvelope
Populate registered_nodes in init_chain
Add validation in process_stake to check node registration
Add participation tracking (update last_seen_block)
Filter validators by participation in select_validators_for_epoch
Questions
How to get Tendermint's current validator set? Is there an ABCI query or state access?
Should we track participation in begin_block/end_block, or query Tendermint?
What's the staleness threshold? One epoch, or configurable?
Should we allow nodes to re-register if they restart, or require a new address?
This ensures only registered nodes can stake, and only live/participating validators are selected for consensus.
here's an issue, maybe. Let's say we start a cluster of 4 nodes. Each call init_chain and are added to the registered_nodes in the app_state.envelope. Now, 100 blocks later, node five is started, with node 4 as a peer. It will replay all tx up to current tx. When it runs init_chain, it can't change the state at that point because it was set 100 blocks ago. So how would it join the registered_nodes?

Checking when init_chain is called and how new nodes join:
Confirming: init_chain is only called at genesis, not when new nodes join. A node joining later replays transactions but doesn't call init_chain again.
Revised design: node registration via transaction
Problem
init_chain only runs at genesis
Late-joining nodes replay transactions but don't call init_chain
We need a way to register nodes that works during replay
Solution: node registration transaction
Concept: Use a transaction to register nodes, so it can be replayed.
Option 1: Explicit RegisterNode transaction (recommended)
New Transaction Type:
// In tx.rspub enum PayloadInner {    // ... existing variants ...    RegisterNode(RegisterNodeTx),}pub struct RegisterNodeTx {    pub node_address: String,        // Address derived from validator public key    pub validator_public_key: String, // Hex-encoded Ed25519 public key (32 bytes)    pub node_id: String,             // Optional: Tendermint node ID for verification}
How it works:
When a node starts, it sends a RegisterNodeTx transaction
This transaction is processed like any other transaction
It adds the node to registered_nodes in the state
When node 5 joins 100 blocks later, it replays this transaction and gets registered
Validation in process_register_node:
// Verify the transaction signature matches the validator_public_key// This ensures only the node with the matching private key can register itself// Derive address from public_key and verify it matches node_address
Pros:
Works for late-joining nodes (transaction gets replayed)
Explicit registration
Can include additional metadata (node ID, version, etc.)
Transaction signature proves ownership of the validator key
Cons:
Requires nodes to send a transaction on startup
Adds a transaction to the chain
Option 2: Auto-registration on first participation
Concept: Register a node automatically when it first participates in consensus (votes or proposes).
Implementation:
// In begin_block or end_block// When processing validator updates or seeing votes:1. Check if proposer/voter address exists in registered_nodes2. If NO:    - Extract validator public key from Tendermint's validator set   - Add to registered_nodes automatically   - Log: "Auto-registered node: {address}"
Pros:
No explicit transaction needed
Automatic registration when node participates
Cons:
Harder to verify the node actually owns the validator key
Registration happens implicitly, which might be confusing
What if a node votes but hasn't staked yet? (They shouldn't be a validator)
Option 3: Hybrid approach (recommended)
Concept: Combine both - allow explicit registration via transaction, but also auto-register from Tendermint's validator set.
Implementation:
// In select_validators_for_epoch:1. Get Tendermint's current validator set (via ABCI query or state)2. For each validator in Tendermint's set:   - Check if address exists in registered_nodes   - If NO: Auto-register it (extract public key from Tendermint)   - Update last_seen_block3. Filter validators by:   - Must exist in registered_nodes (either explicit or auto-registered)   - Must have staked (exist in validators list)   - Must be "live" (recent participation or in Tendermint's set)
For staking validation:
// In process_stake:1. Check if address exists in registered_nodes2. If NO:    - Try to find in Tendermint's current validator set   - If found in Tendermint: Auto-register it   - If NOT found: Reject stake ("Address does not correspond to a running node")3. If YES: Verify public_key matches registered node's validator_public_key
Recommended approach: Option 1 (RegisterNode transaction) + Option 3 (Tendermint fallback)
Why this works:
Nodes can explicitly register via transaction (works for late joiners via replay)
Fallback to Tendermint's validator set for nodes that are already validators
Staking validation ensures only registered nodes can stake
Validator selection filters by both registration and live participation
Flow:
Node Startup:1. Node starts and connects to network2. Node sends RegisterNodeTx transaction3. Transaction is processed, node added to registered_nodes4. Node can now stakeLate Joining Node (Node 5):1. Node 5 starts 100 blocks later2. Replays all transactions including RegisterNodeTx from other nodes3. Replays its own RegisterNodeTx (if it sent one)4. State now includes node 5 in registered_nodes5. Node 5 can stakeStaking:1. Process StakeTx2. Check if address in registered_nodes OR Tendermint validator set3. If yes: Allow staking4. If no: RejectValidator Selection:1. Get Tendermint's current validator set2. Auto-register any validators not in registered_nodes3. Filter: Must be registered AND have staked AND be in Tendermint set4. Select top N by stake
Questions
How to access Tendermint's current validator set from ABCI? Is there a query or state access?
Should RegisterNodeTx be free or require a fee?
Should we allow re-registration if a node restarts with the same validator key?
This design ensures:
Late-joining nodes can register via transaction replay
Only registered nodes can stake
Only live validators (in Tendermint's set) are selected
Consensus doesn't stall due to non-existent validators