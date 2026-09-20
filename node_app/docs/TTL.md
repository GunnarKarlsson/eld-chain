TTL will be in blocks but just like Bitcoin there needs to be a concept of real time outside of chain.
so that block production happens at a regular rate.

The miner will include a timestamp in a block.
this is how bitcoin works:

How does bitcoin have a concept of time like approx 10 minutes between blocks. There is no time onchain

Searching the web

25 results

Bitcoin doesn't have a built-in, tamper-proof "wall-clock" time source on the chain itself — no atomic clocks, no GPS, no centralized time server. Yet it still achieves an average block interval of ~10 minutes very reliably over long periods.Here's how it works, step by step:1. Individual blocks use miner-provided timestamps (not trusted)Every block header contains a 4-byte Unix timestamp field — basically the miner saying "I think the current real-world time is X".But miners can lie (put a slightly wrong time), so Bitcoin imposes rules to limit cheating:The timestamp must be greater than the median timestamp of the previous 11 blocks (this is called Median Time Past or MTP — it creates a monotonically increasing time floor so time-based features like locktimes work reliably).
The timestamp cannot be too far in the future (no more than ~2 hours ahead of the node's own adjusted network time).

These rules prevent miners from arbitrarily jumping time forward/backward in a way that breaks consensus, but they don't enforce exact 10-minute spacing per block.2. The real magic: Difficulty Adjustment Algorithm (DAA) — the thing that creates the ~10 min averageBitcoin doesn't try to force exactly 10 minutes between every single block (impossible in a decentralized proof-of-work system — block times are random / Poisson-distributed).Instead, it targets an average of 10 minutes by automatically retargeting how hard mining is.Every 2016 blocks (~2 weeks if everything is perfect), the network recalculates the difficulty.
It looks at the timestamps of those previous 2016 blocks.
It measures how long (in timestamp time) those 2016 blocks actually took.
Target time for 2016 blocks = 2016 × 600 seconds = 1,209,600 seconds (~2 weeks).
New difficulty = old difficulty × (target time / actual time taken)

Simple examples:If the last 2016 blocks came too fast (say only 1.5 weeks passed in timestamps) → hash rate was high → difficulty increases → next blocks become harder → expected time slows back toward 10 min.
If blocks were too slow (say 3 weeks passed) → hash rate dropped → difficulty decreases → blocks become easier → expected time speeds up again.

Because every full node independently runs exactly the same math on the same blockchain data, they all agree what the correct difficulty for the next period must be. A miner who uses wrong difficulty gets their block rejected by the entire network.3. Key propertiesNo on-chain absolute time — only relative differences derived from block timestamps.
Timestamps can be fudged a bit, but large-scale manipulation gets punished because it distorts the difficulty retarget → the manipulator hurts themselves long-term (or gets forked off).
Individual blocks vary wildly — you see 1-second gaps, 40-minute gaps, occasionally hours — but the long-term average stays extremely close to 10 minutes.
The system is self-correcting even if huge hash rate swings happen (e.g. China mining ban, new ASICs, halvings indirectly affecting economics).

Quick comparison to show why this is cleverProperty
Bitcoin (PoW) approach
Many modern PoS / BFT chains
Block time target
Average ~10 min
Strict slot every X seconds
Enforcement
Dynamic difficulty retarget using timestamps
Scheduled rounds + timeouts + leader rotation
Individual block variance
High (Poisson / memoryless)
Very low
Reliance on external time
Indirect (via miner timestamps + node clocks)
Often stronger (but still needs some synch)
Decentralization impact
Very high (anyone can mine anytime)
Usually lower (known validators/slots)

How does Tendermint / Cosmos / BFT keep time between blocks constant?

Searching the web

24 results

Tendermint (now evolved into CometBFT, the consensus engine used by most Cosmos SDK chains) achieves much more stable and predictable block times than Bitcoin-style PoW — often in the range of ~5–7 seconds on chains like Cosmos Hub, with far lower variance — but it does not enforce a mathematically strict constant interval every single block like a perfectly synchronized clock.Instead, it uses a timed, round-based Byzantine Fault Tolerant (BFT) protocol with explicit timeouts and a configurable "commit delay" to create strong statistical regularity under normal network conditions.Core mechanism: rounds + timeouts + commit phaseSlot-like structure via rotating proposers
Validators take turns proposing blocks in a deterministic order (proportional to stake-weighted round-robin).
At each new height, there is one designated proposer.
Consensus proceeds in rounds (usually 1 round per height under good conditions)
Each round has fixed phases with timeouts:Propose phase → proposer broadcasts block → timeout_propose (default ~3 s, often tuned lower like 500 ms–1 s)
Prevote phase → validators vote on the proposal (or nil) → timeout_prevote
Precommit phase → validators precommit (or nil) → timeout_precommit

Each timeout has a base value + a small delta that increases per subsequent round (e.g. timeout_propose_delta = 500 ms–1 s) → this prevents livelock if the proposer is slow/offline.
Commit happens when +2/3 precommits are seen
Once a block reaches +2/3 precommits, validators commit it → the block is finalized (instant finality).
The key parameter for controlling average block time: timeout_commit
After commit, there is a configurable timeout_commit delay (often set to ~1–5 seconds in practice) before the next height officially starts and the next proposer begins proposing.This acts like a minimum inter-block delay or "breathing room".
Example: if consensus itself takes 2–3 s and timeout_commit = "5s", blocks tend to land around **6–8 s** apart.
Many chains tune timeout_commit so the typical healthy-case latency matches their target (e.g. Cosmos Hub ~6–7 s average).

Empty blocks & create_empty_blocks  If no transactions → chain can produce empty blocks (or skip them).
With create_empty_blocks = false (common setting), the chain waits until transactions arrive or a fallback interval passes → helps avoid spamming empty blocks but can cause occasional longer gaps.
With create_empty_blocks = true + small timeout_commit → near-constant ticking even when idle.

Why block times are quite stable (but not perfectly constant)Factor
Effect on variance
Typical real-world impact
Fast normal-case consensus
1 round → very quick (~1–4 s)
Low variance when network is synchronous
timeout_commit delay
Fixed "floor" after commit
Pulls average toward target (e.g. 6 s)
Missed/slow proposers
Timeout → round increment → next proposer
Adds ~few seconds delay (rare on healthy nets)
Network latency/jitter
Delays votes/proposal delivery
±0.5–2 s wobble in most cases
Multiple rounds (rare)
Exponential timeout growth
Can cause 10–30+ s outliers under attack/partition
Clock drift / median time rules
BFT Time uses median of last commit
Prevents time jumps, but minor influence

In practice (2025–2026 data):Healthy Cosmos chains: ~5.5–7.5 s median, very few blocks >10 s unless congestion or validator issues.
Modern CometBFT chains often see ±500 ms to ±1.5 s variance around the mean — far tighter than PoW.
Some app-chains deliberately target ~1–2 s by aggressive tuning (lower timeouts), but this requires low-latency validators.

Comparison to BitcoinAspect
Bitcoin (PoW)
Tendermint/CometBFT (BFT PoS)
Enforcement
Difficulty retarget (average only)
Explicit timeouts + commit delay
Variance per block
High (seconds  tens of minutes)
Low (± seconds)
Finality
Probabilistic
Immediate on commit
Empty/idle behavior
Always produces blocks
Configurable (empty or wait)
Reliance on timeouts
None (statistical)
Central (weak synchrony assumption)

Bottom line
Tendermint/CometBFT keeps block times "nearly constant" through carefully tuned timeouts (especially timeout_commit) + fast deterministic rounds under partial synchrony. The chain doesn't have a hard real-time scheduler like some permissioned systems, but the design + validator incentives make deviations rare and short-lived in healthy operation. For truly sub-second jitter, newer BFT variants (HotStuff derivatives like in Sui/Aptos) pipeline more aggressively, but Tendermint-style remains very predictable for most Cosmos ecosystem use cases.If you're building on Cosmos SDK / CometBFT, the best way to get the steadiest block times is tuning the consensus params in config.toml (especially timeout_commit + the propose/prevote/precommit timeouts) and running low-latency, well-connected validators.

---

What MB for a single message. 
If max is 1MB... what is max size for block
max ttl depends on size, larger messages have shorter life spans...

Max message size is 100KB ? Max block size is.... 1MB => 

1440 blocks per day: if one MB per block, 1.4GB per day. MAX chain: 

If 1 block per 2 seconds, and max block size is 0.5MB, and max TTL is 30 days: 30 * 0.5 (HOUR) * 24 (DAY) * 30 = 10.8GB max size...

max time to live:  30 days... 42GB....

Pay to pro-long content on chain... ?

---

If TTL is passed, request should not return content even if it isn't yet removed...

what will be content size VS chain size....

content size: MAX 5 GB per year say... 

how many 5KB messages: 1,000,000 messages at one time...
Not for everyone but for many....

----

Long term plan:
+ Modify or replace the tendermint layer with logic that compresses blocks for expired messages.
+ Consolidated chain work can be verified by validators and posted as tx. After a tx that consolidates say block 100 to 200, the node block indexer
+ add (what?) to block 100 metadata (based on the transaction) to allow it to link to a newer block (which's metadata is also modified)...
+ Fun to design this protocol...

---

"Eraser blockchain" typically refers to techniques allowing the deletion or redaction of specific data from a distributed ledger while maintaining chain integrity, which is traditionally immutable. Key approaches include local node erasure (FPLE), redaction via chameleon hashes, or cryptographic erasure ("crypto-shredding"). These methods help address privacy laws, such as GDPR "right to be forgotten," without breaking the network. 
arXiv.org
arXiv.org
 +4
Key Concepts in Blockchain Data Erasure
Local Node Erasure (FPLE): FPLE (Flexible and Private Local Erasure) allows individual nodes to erase infringing or sensitive data from their local storage while still validating the blockchain, allowing the node to stay in sync with the network.
Redactable Blockchains: Use chameleon hash functions to allow authorized parties to modify or delete specific data blocks without needing to re-mine the entire chain.
Cryptographic Erasure (Crypto-shredding): Data is encrypted before being stored. To "erase" it, the encryption key is destroyed, making the stored data unreadable and irrecoverable.
GDPR Compliance: Specialized tools are being developed to allow for the deletion of personal data in compliance with the "right to erasure" (GDPR).
Challenges: Erasing data globally without a hard fork is difficult. The core promise of blockchain is immutability, so deleting data creates a paradox, often requiring, as outlined in this Quora post, a change to the underlying trust model. 
Blancco
Blancco
 +5
Use Cases
Removing Harmful Content: Deleting illegal or malicious data (e.g., illegal images, malware) inserted into transaction fields.
Privacy Regulation: Ensuring compliance with privacy laws requiring user data removal. 
Agencia Española de Protección de Datos | AEPD
Agencia Española de Protección de Datos | AEPD
 +4
Methods for Deleting/Modifying Data
Hard Fork: A radical change where the community agrees to split the chain, removing data from the new version.
Zero-Knowledge Proofs (ZKPs): Compressing blockchain history, allowing nodes to discard old, sensitive data while retaining the ability to verify the ledger.
Erasure Coding: Splitting data into fragments, creating redundancy for storage efficiency, which allows for selective removal of data blocks while allowing for reconstruction if needed, according to this IEEE report. 
arXiv.org
arXiv.org
 +2

 ---

