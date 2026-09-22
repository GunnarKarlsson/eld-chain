use super::connection::ConsensusConnection;
use crate::storage::traits::ConsensusConnectionStorage;
use ::abci::{async_api::Consensus, async_trait, types::*};

#[async_trait]
impl<S> Consensus for ConsensusConnection<S>
where
    S: ConsensusConnectionStorage,
{
    async fn init_chain(&self, init_chain_request: RequestInitChain) -> ResponseInitChain {
        self.init_chain_inner(init_chain_request).await
    }

    async fn begin_block(&self, begin_block_request: RequestBeginBlock) -> ResponseBeginBlock {
        self.begin_block_inner(begin_block_request).await
    }

    async fn deliver_tx(&self, deliver_tx_request: RequestDeliverTx) -> ResponseDeliverTx {
        self.deliver_tx_inner(deliver_tx_request).await
    }

    async fn end_block(&self, end_block_request: RequestEndBlock) -> ResponseEndBlock {
        self.end_block_inner(end_block_request).await
    }

    async fn commit(&self, commit_request: RequestCommit) -> ResponseCommit {
        self.commit_inner(commit_request).await
    }
}
