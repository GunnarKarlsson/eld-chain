//! Merkle Patricia Trie for committed CADO state roots (`AppStateTip.cado_root_hash`).

use crate::app_state::nibbles::path_to_nibbles;
use hash256_std_hasher::Hash256StdHasher;
use hash_db::Hasher as HashDBHasher;
use memory_db::{HashKey, MemoryDB};
use reference_trie::GenericNoExtensionLayout;
use sha2::{Digest, Sha256};
use trie_db::{DBValue, TrieDBMut, TrieDBMutBuilder, TrieHash, TrieMut};

/// SHA-256 hasher for Eld state trie (matches CADO hashing).
#[derive(Default, Clone, Copy, Debug, Eq, PartialEq)]
pub struct Sha256Hasher;

impl HashDBHasher for Sha256Hasher {
    type Out = [u8; 32];
    type StdHasher = Hash256StdHasher;
    const LENGTH: usize = 32;

    fn hash(x: &[u8]) -> Self::Out {
        Sha256::digest(x).into()
    }
}

type EldLayout = GenericNoExtensionLayout<Sha256Hasher>;

/// In-memory Merkle Patricia Trie (trie-db + reference-trie layout).
#[derive(Clone)]
pub struct StateTrie {
    db: MemoryDB<Sha256Hasher, HashKey<Sha256Hasher>, DBValue>,
    root: TrieHash<EldLayout>,
}

impl std::fmt::Debug for StateTrie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateTrie")
            .field("root_hash", &self.root_hash())
            .finish()
    }
}

impl Default for StateTrie {
    fn default() -> Self {
        Self::empty()
    }
}

impl StateTrie {
    /// Empty trie (no entries).
    pub fn empty() -> Self {
        Self {
            db: MemoryDB::default(),
            root: Default::default(),
        }
    }

    fn with_trie_mut<R>(&mut self, f: impl FnOnce(&mut TrieDBMut<'_, EldLayout>) -> R) -> R {
        let mut trie = TrieDBMutBuilder::<EldLayout>::new(&mut self.db, &mut self.root).build();
        f(&mut trie)
    }

    /// Inserts or updates `key` (CADO path bytes) → `value` (typically `content_hash()`).
    pub fn insert(&mut self, key: &[u8], value: &[u8]) {
        let nibbles = path_to_nibbles(key);
        self.with_trie_mut(|trie| {
            trie.insert(&nibbles, value)
                .expect("state trie insert failed");
        });
    }

    /// Removes `key` from the trie (no-op if absent).
    pub fn remove(&mut self, key: &[u8]) {
        let nibbles = path_to_nibbles(key);
        self.with_trie_mut(|trie| {
            let _ = trie.remove(&nibbles);
        });
    }

    /// Current Merkle root (32 bytes).
    pub fn root_hash(&self) -> [u8; 32] {
        let root = self.root.as_ref();
        if root.len() == 32 {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(root);
            arr
        } else {
            [0u8; 32]
        }
    }

    /// Root hash of an empty trie (for genesis / default).
    pub fn empty_root_hash() -> [u8; 32] {
        Self::empty().root_hash()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_changes_root() {
        let mut trie = StateTrie::empty();
        let empty = trie.root_hash();
        trie.insert(b"/@eld/account/0x01", &[1u8; 32]);
        assert_ne!(trie.root_hash(), empty);
    }

    #[test]
    fn remove_after_insert_changes_root() {
        let mut trie = StateTrie::empty();
        let empty = trie.root_hash();
        let key = b"/@eld/account/0x01";
        let hash = [2u8; 32];
        trie.insert(key, &hash);
        let with_entry = trie.root_hash();
        assert_ne!(with_entry, empty);
        trie.remove(key);
        assert_ne!(trie.root_hash(), with_entry);
    }
}
