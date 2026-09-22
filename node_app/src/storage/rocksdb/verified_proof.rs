use eld_common::error::EldError;
use rocksdb::Transaction;

use super::keys::DUMMY_ROCKSDB_PAYLOAD;
use super::RocksDBStorage;

impl RocksDBStorage {
    /// Key for verified-proof submission claim rows (`vp_submit:` prefix avoids collisions).
    pub(crate) fn verified_proof_submission_claim_key(
        sender_address_hex: &str,
        challenge_id: &str,
    ) -> Vec<u8> {
        format!(
            "vp_submit:{}:{}",
            sender_address_hex.to_ascii_lowercase(),
            challenge_id
        )
        .into_bytes()
    }

    /// Persist `(sender, challenge_id)` if absent; return `true` on first claim, `false` if already present.
    /// Used by [`HybridStorage`](crate::storage::hybrid_storage::HybridStorage) via
    /// [`VerifiedProofSubmissionClaimStorage`](crate::storage::traits::VerifiedProofSubmissionClaimStorage).
    pub fn insert_verified_proof_submission_claim(
        &self,
        sender_address_hex: &str,
        challenge_id: &str,
    ) -> Result<bool, EldError> {
        let cf = self.verified_proof_submissions_cf()?;
        let key = Self::verified_proof_submission_claim_key(sender_address_hex, challenge_id);
        if self
            .db
            .get_cf(cf, &key)
            .map_err(|e| EldError::StorageError {
                operation: "verified_proof_submission_get".to_string(),
                details: e.to_string(),
            })?
            .is_some()
        {
            return Ok(false);
        }
        self.db
            .put_cf(cf, &key, DUMMY_ROCKSDB_PAYLOAD)
            .map_err(|e| EldError::StorageError {
                operation: "verified_proof_submission_put".to_string(),
                details: e.to_string(),
            })?;
        Ok(true)
    }

    /// Returns `true` if `(sender, challenge_id)` was already claimed (no write).
    pub fn has_verified_proof_submission_claim(
        &self,
        sender_address_hex: &str,
        challenge_id: &str,
    ) -> Result<bool, EldError> {
        let cf = self.verified_proof_submissions_cf()?;
        let key = Self::verified_proof_submission_claim_key(sender_address_hex, challenge_id);
        Ok(self
            .db
            .get_cf(cf, &key)
            .map_err(|e| EldError::StorageError {
                operation: "verified_proof_submission_has".to_string(),
                details: e.to_string(),
            })?
            .is_some())
    }

    /// Key for on-chain rewarded challenge dedup (`vp_rewarded:` prefix).
    pub(crate) fn verified_proof_challenge_rewarded_key(challenge_id: &str) -> Vec<u8> {
        format!("vp_rewarded:{challenge_id}").into_bytes()
    }

    pub fn is_verified_proof_challenge_rewarded(
        &self,
        challenge_id: &str,
    ) -> Result<bool, EldError> {
        let cf = self.verified_proof_submissions_cf()?;
        let key = Self::verified_proof_challenge_rewarded_key(challenge_id);
        self.db
            .get_cf(cf, &key)
            .map_err(|e| EldError::StorageError {
                operation: "verified_proof_challenge_rewarded_get".to_string(),
                details: e.to_string(),
            })
            .map(|v| v.is_some())
    }

    pub fn put_verified_proof_challenge_rewarded_with_tx(
        &self,
        challenge_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        let cf = self.verified_proof_submissions_cf()?;
        let key = Self::verified_proof_challenge_rewarded_key(challenge_id);
        tx.put_cf(cf, &key, DUMMY_ROCKSDB_PAYLOAD)
            .map_err(|e| EldError::StorageError {
                operation: "verified_proof_challenge_rewarded_put".to_string(),
                details: e.to_string(),
            })
    }
}

impl crate::storage::traits::VerifiedProofRewardDedupStorage for RocksDBStorage {
    fn is_verified_proof_challenge_rewarded(&self, challenge_id: &str) -> Result<bool, EldError> {
        RocksDBStorage::is_verified_proof_challenge_rewarded(self, challenge_id)
    }

    fn put_verified_proof_challenge_rewarded_with_tx(
        &self,
        challenge_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        RocksDBStorage::put_verified_proof_challenge_rewarded_with_tx(self, challenge_id, tx)
    }
}
