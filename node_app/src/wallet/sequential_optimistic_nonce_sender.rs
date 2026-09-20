//! Per-wallet sequential assignment of transaction nonces with optimistic local advance.
//!
//! Initialize from the chain once (`get_next_nonce_for_account_cado` semantics), then increment
//! locally for each subsequent send without waiting for commit. Intended for a **single**
//! process sending on behalf of that address.

use eld_common::error::EldError;
use eld_common::nonce::Nonce;
use tokio::sync::Mutex;

/// Holds the next tx nonce to assign for one sender address.
///
/// Not `Clone` — one instance per hot wallet that broadcasts sequentially.
pub struct SequentialOptimisticNonceSender {
    /// `None` until the first successful chain read; thereafter always `Some(next)`.
    next_tx_nonce: Mutex<Option<Nonce>>,
}

impl Default for SequentialOptimisticNonceSender {
    fn default() -> Self {
        Self::new()
    }
}

impl SequentialOptimisticNonceSender {
    pub fn new() -> Self {
        Self {
            next_tx_nonce: Mutex::new(None),
        }
    }

    /// Returns the nonce for this transaction and advances the local cursor (`n` → `n.next()`).
    ///
    /// After `u32::MAX`, `next()` is `None` and the cursor is cleared. The next call retries
    /// `fetch_chain_next_nonce` (and fails if that also cannot produce a nonce).
    ///
    /// On the first call, runs `fetch_chain_next_nonce` (should match
    /// [`eld_client::api::abci::query::get_next_nonce_for_account_cado`]). Later calls ignore the
    /// fetcher unless the cursor was never set (e.g. first fetch returned `Ok(None)` — will retry).
    pub async fn take_next_nonce<F, Fut>(
        &self,
        mut fetch_chain_next_nonce: F,
    ) -> Result<Nonce, EldError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<Option<Nonce>, EldError>>,
    {
        let mut slot = self.next_tx_nonce.lock().await;
        if slot.is_none() {
            *slot = fetch_chain_next_nonce().await?;
        }
        let current = slot.ok_or_else(|| EldError::NetworkError {
            operation: "nonce retrieval".to_string(),
            details: "SequentialOptimisticNonceSender: failed to obtain initial nonce from chain"
                .to_string(),
        })?;
        let assigned = current;
        *slot = current.next();
        Ok(assigned)
    }

    /// After a failed broadcast of `assigned`, set the cursor so the next
    /// [`take_next_nonce`](Self::take_next_nonce) returns `assigned` again (reuse same nonce).
    ///
    /// Also used after a network error to install the next nonce read from the account CADO.
    pub async fn reuse_nonce(&self, assigned: Nonce) {
        let mut slot = self.next_tx_nonce.lock().await;
        *slot = Some(assigned);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn advances_without_second_fetch() {
        let s = SequentialOptimisticNonceSender::new();
        let mut fetch_count = 0u32;
        let n1 = s
            .take_next_nonce(|| {
                fetch_count += 1;
                async move { Ok(Some(Nonce::new(5))) }
            })
            .await
            .unwrap();
        assert_eq!(n1.value(), 5);
        assert_eq!(fetch_count, 1);

        let n2 = s
            .take_next_nonce(|| {
                fetch_count += 1;
                async move { Ok(Some(Nonce::new(99))) }
            })
            .await
            .unwrap();
        assert_eq!(n2.value(), 6);
        assert_eq!(fetch_count, 1);
    }

    #[tokio::test]
    async fn reuse_nonce_returns_same_assignment_then_advances() {
        let s = SequentialOptimisticNonceSender::new();
        let n1 = s
            .take_next_nonce(|| async { Ok(Some(Nonce::new(5))) })
            .await
            .unwrap();
        assert_eq!(n1.value(), 5);

        s.reuse_nonce(n1).await;

        let n2 = s
            .take_next_nonce(|| async { Ok(Some(Nonce::new(99))) })
            .await
            .unwrap();
        assert_eq!(n2.value(), 5);

        let n3 = s
            .take_next_nonce(|| async { Ok(Some(Nonce::new(99))) })
            .await
            .unwrap();
        assert_eq!(n3.value(), 6);
    }

    #[tokio::test]
    async fn take_next_nonce_clears_cursor_after_u32_max() {
        let s = SequentialOptimisticNonceSender::new();
        let n1 = s
            .take_next_nonce(|| async { Ok(Some(Nonce::new(u32::MAX))) })
            .await
            .unwrap();
        assert_eq!(n1.value(), u32::MAX);

        let err = s.take_next_nonce(|| async { Ok(None) }).await.unwrap_err();
        assert!(format!("{err}").contains("failed to obtain initial nonce from chain"));
    }

    #[tokio::test]
    async fn take_next_nonce_propagates_fetch_error() {
        let s = SequentialOptimisticNonceSender::new();
        let err = s
            .take_next_nonce(|| async {
                Err(EldError::NetworkError {
                    operation: "nonce retrieval".to_string(),
                    details: "rpc down".to_string(),
                })
            })
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("rpc down"));
    }
}
