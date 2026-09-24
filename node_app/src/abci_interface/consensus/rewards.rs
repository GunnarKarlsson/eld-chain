use crate::app_state::{AccountWithCadoHash, AppState};
use crate::storage::traits::CADOStorage;
use eld_common::account::Account;
use eld_common::address::Address;
use eld_common::cado::{CADOMetadata, CadoBody, CadoPath, CadoPathKey, CadoType};
use eld_common::coin::Coin;
use eld_common::error::EldError;
use eld_common::nonce::Nonce;
use std::sync::Arc;

#[derive(Debug)]
pub struct ValidatorRewardManager<T>
where
    T: CADOStorage,
{
    storage: Arc<T>,
}

impl<T> ValidatorRewardManager<T>
where
    T: CADOStorage,
{
    pub(crate) fn new(storage: Arc<T>) -> Self {
        Self { storage }
    }

    /// Credits `reward` to the capacity provider account.
    ///
    /// Called only from `VerifiedProof` delivery. The transaction is in the block, so every
    /// node applies the same balance change.
    pub(crate) fn credit_verified_proof_reward(
        &self,
        current_state: &mut AppState,
        provider: Address,
        reward: Coin,
    ) -> Result<(), EldError> {
        let provider_path = CadoPath::new(CadoType::Account, CadoPathKey::Address(provider))?;
        let provider_account_with_hash = current_state
            .envelope
            .get_account_from_cado(&*self.storage, &provider_path)
            .unwrap_or_else(|| AccountWithCadoHash {
                account: Account::new(provider, Coin::zero(), Nonce::new(Nonce::ZERO)),
                hash: [0; 32],
            });
        let provider_account = provider_account_with_hash.account;

        let updated_balance = (provider_account.balance() + reward)?;
        let updated_provider = Account::new(
            *provider_account.address(),
            updated_balance,
            provider_account.nonce(),
        );
        let provider_serialized =
            bincode::serialize(&updated_provider).map_err(|e| EldError::StorageError {
                operation: "serialize_account".to_string(),
                details: format!("Failed to serialize provider account: {e}"),
            })?;
        let provider_meta = CADOMetadata::new(CadoType::Account, provider.to_string());
        let provider_cado = CadoBody::mutable_updated(
            provider_account_with_hash.hash,
            provider_serialized,
            provider_meta,
        );
        current_state
            .envelope
            .update_cado_cache(provider_path, provider_cado);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::rocksdb::RocksDBStorage;
    const VERIFIED_PROOF_REWARD_BASE_AMOUNT: u128 = 1000;

    #[test]
    fn verified_proof_reward_credits_provider_account_cado() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let storage = Arc::new(RocksDBStorage::new(dir.path()).expect("rocksdb"));
        let manager = ValidatorRewardManager::new(storage.clone());
        let mut state = AppState::default();
        let provider = Address::parse_hex_str("0x2222222222222222222222222222222222222222")
            .expect("provider address");
        let reward = Coin::new(VERIFIED_PROOF_REWARD_BASE_AMOUNT).expect("reward coin");

        manager
            .credit_verified_proof_reward(&mut state, provider, reward)
            .expect("first credit");
        manager
            .credit_verified_proof_reward(&mut state, provider, reward)
            .expect("second credit");

        let path =
            CadoPath::new(CadoType::Account, CadoPathKey::Address(provider)).expect("account path");
        let credited = state
            .envelope
            .get_account_from_cado(&*storage, &path)
            .expect("credited account");
        assert_eq!(
            credited.account.balance().amount(),
            2 * VERIFIED_PROOF_REWARD_BASE_AMOUNT
        );
        assert_eq!(*credited.account.address(), provider);
    }
}
