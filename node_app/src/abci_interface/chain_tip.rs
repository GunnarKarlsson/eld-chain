use crate::app_state::{AppHash, AppState};
use std::sync::RwLock;

#[derive(Debug, Clone, Default)]
pub struct ChainTipSnapshot {
    pub committed_height: i64,
    pub current_epoch: i64,
    pub app_hash: AppHash,
}

/// Lightweight, read-optimized projection of committed chain head.
///
/// Why this exists:
/// - `AppState` is shared as `Arc<Mutex<AppState>>`, and reading any field through it contends
///   with consensus writes (begin/end block, commit, state transitions).
/// - High-frequency read paths (ABCI info/query, background GC tick, debug/status polling) often
///   only need tiny committed-head values such as height/epoch/hash.
/// - Putting those read-hot fields behind this dedicated lock avoids taking the full AppState
///   mutex for simple reads.
///
/// How to use it:
/// - Treat this as a derived mirror of committed state, not a second source of truth.
/// - Consensus commit updates it after committed state has advanced.
/// - Readers copy required fields quickly and drop the read guard immediately.
///
/// Diff vs `AppState`:
/// - `AppState` holds full consensus state (trie, validator lists, caches, envelopes) and remains
///   authoritative.
/// - `ChainTip` only carries small committed-head metadata that is cheap to copy and query often.
/// - `ChainTip` never stores large structures and should not be used for mutation logic.
#[derive(Debug, Default)]
pub struct ChainTip {
    inner: RwLock<ChainTipSnapshot>,
}

impl ChainTip {
    pub fn from_app_state(state: &AppState) -> Self {
        Self {
            inner: RwLock::new(ChainTipSnapshot {
                committed_height: state.envelope.block_height,
                current_epoch: state.envelope.current_epoch,
                app_hash: *state.app_hash(),
            }),
        }
    }

    pub fn update_from_app_state(&self, state: &AppState) {
        if let Ok(mut guard) = self.inner.write() {
            guard.committed_height = state.envelope.block_height;
            guard.current_epoch = state.envelope.current_epoch;
            guard.app_hash = *state.app_hash();
        }
    }

    pub fn snapshot(&self) -> ChainTipSnapshot {
        match self.inner.read() {
            Ok(guard) => guard.clone(),
            Err(_) => ChainTipSnapshot::default(),
        }
    }

    pub fn committed_height_u64(&self) -> u64 {
        self.snapshot().committed_height.max(0) as u64
    }
}
