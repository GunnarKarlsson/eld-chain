use eld_common::address::Address;
use eld_common::cado::{epoch_from_record_path_name, CadoPath};
use eld_common::constants::cado::{LATEST, PATH_PREFIX_EPOCH_RECORD, TYPE_EPOCH_RECORD};
use rocksdb::ReadOptions;

use super::RocksDBStorage;

/// RocksDB key under `indexed_transactions` holding the count of primary indexed rows
/// (`0x…` keys without `:`, i.e. full tx bodies). Serialized as little-endian `u64`.
/// Never overlaps tx IDs (those match `0x` + hex only).
pub(super) const INDEXED_TRANSACTIONS_PRIMARY_TOTAL_COUNT_KEY: &[u8] =
    b"_meta:indexed_transactions_primary_total";

/// Little-endian `u64` count of successful indexed [`VerifiedProof`](eld_common::tx::PayloadInner::VerifiedProof)
/// network-wide (rollup). Does not overlap `0x` primaries or `vp:` keys.
pub(super) const VERIFIED_PROOF_GLOBAL_REWARDS_COUNT_KEY: &[u8] =
    b"_meta:verified_proof_global_rewards_count";

/// Placeholder value for RocksDB keys where only existence matters (dedup markers).
pub(super) const DUMMY_ROCKSDB_PAYLOAD: &[u8] = b"1";

/// Chronological ordering index for txs: **`block_pos:` + fixed hex height + ':' + hex index**.
/// Stored **in addition to** unpadded legacy `block_height:{h}:tx_index:{i}` rows.
///
/// Iterate between [`BLOCK_POS_CHRON_LOWER_BOUND`] (inclusive) and
/// [`BLOCK_POS_CHRON_UPPER_BOUND_EXCLUSIVE`] (exclusive) — byte-wise order equals chain order `(height, tx_index)`.
pub(super) const BLOCK_POS_CHRON_LOWER_BOUND: &[u8] = b"block_pos:";
/// Exclusive scan upper bound: same prefix as real keys except the last byte is **semicolon** `;`,
/// which sorts after **colon** `:` — so every actual key (`block_pos:` + digits + `:` + …) stays below this.
pub(super) const BLOCK_POS_CHRON_UPPER_BOUND_EXCLUSIVE: &[u8] = b"block_pos;";

/// Exclusive upper bound for `path_index` scans under [`PATH_PREFIX_EPOCH_RECORD`].
pub(super) const EPOCH_RECORD_PATH_INDEX_UPPER_EXCLUSIVE: &str = "/@eld/epoch_record;";

/// Optional filters matching `GET /transactions` query semantics.
#[derive(Clone, Copy)]
pub(crate) struct IndexedTxChronFilters<'a> {
    pub block_height: Option<u64>,
    pub sender: Option<Address>,
    pub payload_type: Option<&'a str>,
}

impl RocksDBStorage {
    pub(super) fn block_position_chron_index_key_bytes(
        block_height: u64,
        block_index: u32,
    ) -> Vec<u8> {
        format!("block_pos:{block_height:016x}:{block_index:08x}").into_bytes()
    }

    pub(super) fn chron_block_position_read_options() -> ReadOptions {
        let mut ro = ReadOptions::default();
        ro.set_iterate_lower_bound(BLOCK_POS_CHRON_LOWER_BOUND.to_vec());
        ro.set_iterate_upper_bound(BLOCK_POS_CHRON_UPPER_BOUND_EXCLUSIVE.to_vec());
        ro.set_total_order_seek(true);
        ro
    }

    pub(super) fn parse_block_position_chron_key(key_str: &str) -> Option<(u64, u32)> {
        let rest = key_str.strip_prefix("block_pos:")?;
        let (h_hex, ix_hex) = rest.split_once(':')?;
        let bh = u64::from_str_radix(h_hex, 16).ok()?;
        let bi = u32::from_str_radix(ix_hex, 16).ok()?;
        Some((bh, bi))
    }

    pub(super) fn epoch_record_path_index_read_options() -> ReadOptions {
        let mut ro = ReadOptions::default();
        ro.set_iterate_lower_bound(PATH_PREFIX_EPOCH_RECORD.as_bytes().to_vec());
        ro.set_iterate_upper_bound(EPOCH_RECORD_PATH_INDEX_UPPER_EXCLUSIVE.as_bytes().to_vec());
        ro.set_total_order_seek(true);
        ro
    }

    pub(super) fn epoch_number_from_record_path_str(path_str: &str) -> Option<i64> {
        let path = CadoPath::parse(path_str).ok()?;
        if path.type_() != TYPE_EPOCH_RECORD {
            return None;
        }
        let name = path.name();
        if name == LATEST {
            return None;
        }
        epoch_from_record_path_name(name)
    }
    /// Secondary index for successful [`VerifiedProof`](eld_common::tx::PayloadInner::VerifiedProof)
    /// rewards: `vp:{provider}:{height:016x}:{block_index:08x}`.
    pub(crate) fn verified_proof_reward_index_key(
        provider_normalized: &str,
        block_height: u64,
        block_index: u32,
    ) -> Vec<u8> {
        format!("vp:{provider_normalized}:{block_height:016x}:{block_index:08x}").into_bytes()
    }

    pub(crate) fn parse_verified_proof_reward_index_key(
        key_str: &str,
    ) -> Option<(String, u64, u32)> {
        let rest = key_str.strip_prefix("vp:")?;
        let parts: Vec<&str> = rest.split(':').collect();
        if parts.len() != 3 {
            return None;
        }
        let addr = parts[0].to_string();
        let h = u64::from_str_radix(parts[1], 16).ok()?;
        let bi = u32::from_str_radix(parts[2], 16).ok()?;
        Some((addr, h, bi))
    }
    pub(super) fn u64_hex_fixed(v: u64) -> String {
        format!("{v:016x}")
    }

    pub(super) fn pinboard_wallet_index_key(
        wallet: &str,
        committed_height: u64,
        message_id: &str,
    ) -> String {
        format!(
            "{}:{}:{}",
            wallet,
            Self::u64_hex_fixed(committed_height),
            message_id
        )
    }

    pub(super) fn pinboard_tag_index_key(
        tag: &str,
        committed_height: u64,
        message_id: &str,
    ) -> String {
        format!(
            "{}:{}:{}",
            tag,
            Self::u64_hex_fixed(committed_height),
            message_id
        )
    }

    pub(super) fn pinboard_expiry_index_key(expires_height: u64, message_id: &str) -> String {
        format!("{}:{}", Self::u64_hex_fixed(expires_height), message_id)
    }

    pub(super) fn pinboard_commit_index_key(committed_height: u64, message_id: &str) -> String {
        format!("{}:{}", Self::u64_hex_fixed(committed_height), message_id)
    }
}
