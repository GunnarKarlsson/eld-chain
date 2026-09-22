use super::connection::ConsensusConnection;
use crate::errors::handle_recoverable_eld_error;
use crate::node_identity::ensure_identity_from_status;
use crate::storage::traits::ConsensusConnectionStorage;
use abci::types::*;
use eld_common::error::EldError;
use tracing::info;

impl<S> ConsensusConnection<S>
where
    S: ConsensusConnectionStorage,
{
    pub(crate) async fn begin_block_inner(
        &self,
        begin_block_request: RequestBeginBlock,
    ) -> ResponseBeginBlock {
        if !self
            .local_identity
            .read()
            .map(|g| g.is_configured())
            .unwrap_or(false)
        {
            let rpc_url = self.capacity_manager.config().tendermint_rpc_url.clone();
            ensure_identity_from_status(&self.local_identity, &rpc_url).await;
        }

        // Process evidence of validators missing blocks
        let byzantine_validators = begin_block_request.byzantine_validators;
        for evidence in byzantine_validators {
            if let Some(_validator) = evidence.validator {
                // Find the validator in your system by public key
                //let validator_addr = self.find_validator_by_pubkey(&validator.pub_key);

                // Apply penalty - e.g., reduce stake
                //self.penalize_validator(validator_addr, evidence.height);
            }
        }

        // Process last commit info to track who's voting
        if let Some(last_commit_info) = begin_block_request.last_commit_info {
            for _vote_info in last_commit_info.votes {
                // Track validator participation
                // vote_info.signed_last_block tells you if they signed
            }
        }

        // copy last committed state to current state
        let committed_state = match self.committed_state.lock() {
            Ok(state) => state.clone(),
            Err(e) => {
                handle_recoverable_eld_error(EldError::InitializationError {
                    component: "committed state lock".to_string(),
                    details: e.to_string(),
                });
                return Default::default(); // Return default response on lock failure
            }
        };

        let abci_block_height = begin_block_request
            .header
            .as_ref()
            .map(|h| h.height)
            .unwrap_or(0);

        let this_node_provider_id = self.capacity_manager.config().provider_id;
        let validator_addresses: Vec<String> = committed_state
            .envelope
            .validators
            .iter()
            .map(|v| v.address.to_string())
            .collect();

        info!(
            abci_block_height,
            "validator addresses: {}",
            validator_addresses.join(", ")
        );
        info!(
            abci_block_height,
            this_node_provider_id = %this_node_provider_id,
            "this node capacity provider address"
        );

        let mut current_state = match self.current_state.lock() {
            Ok(state) => state,
            Err(e) => {
                handle_recoverable_eld_error(EldError::InitializationError {
                    component: "current state lock".to_string(),
                    details: e.to_string(),
                });
                return Default::default(); // Return default response on lock failure
            }
        };
        info!(
            "begin_block: committed_state.envelope.committed_cado_cache.len(): {}",
            committed_state.envelope.committed_cado_cache.len()
        );
        *current_state = Some(committed_state);

        Default::default()
    }
}
