Node should be as small as possible.
Move from Tendermint to custom Rust node protocol implementation.

Long term plan:

Implement at tendermint / node level:
When older blocks contain only transactions with expired TTL, all blocks up to that point can be hashed into a new hash which
will be included in the next block.
This is done with a consolidation transaction.
How should this work. Max TTL tx will block consolidations, unless the consolidating tx can LIFT then forward to a new block
but retain the tx index. The new tx or block would include the original block index and the new block index.