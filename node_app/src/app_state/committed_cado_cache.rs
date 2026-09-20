use eld_common::cado::is_infrastructure_cado_path;
use eld_common::cado::CadoBody;
use patricia_tree::PatriciaMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::hash::{Hash, Hasher};

/// A wrapper around patricia_tree::PatriciaMap that matches BinaryPatriciaNode's interface
#[derive(Debug, Clone)]
pub struct CommittedCadoCache {
    map: PatriciaMap<CadoBody>,
    pub root_hash: [u8; 32],
}

impl CommittedCadoCache {
    pub fn new() -> Self {
        Self {
            map: PatriciaMap::new(),
            root_hash: [0u8; 32],
        }
    }

    pub fn insert(&mut self, key: &[u8], value: CadoBody) {
        self.map.insert(key, value);
    }

    pub fn remove(&mut self, key: &[u8]) {
        self.map.remove(key);
    }

    /// Returns a committed CADO at `path` bytes, if present (Layer B registry lookups).
    pub fn get(&self, key: &[u8]) -> Option<&CadoBody> {
        self.map.get(key)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Iterates committed path → CADO pairs in arbitrary order.
    pub fn iter(&self) -> impl Iterator<Item = (Vec<u8>, CadoBody)> + '_ {
        self.map.iter().map(|(k, v)| (k.clone(), v.clone()))
    }

    /// Clone omitting infrastructure CADO paths (snapshots, app_state_tip, ABCI chunks).
    pub fn without_infrastructure(&self) -> Self {
        let mut filtered = Self::new();
        for (key, value) in self.iter() {
            let Ok(path_str) = std::str::from_utf8(&key) else {
                continue;
            };
            if is_infrastructure_cado_path(path_str) {
                continue;
            }
            filtered.insert(&key, value);
        }
        filtered.calculate_hash();
        filtered
    }

    pub fn calculate_hash(&mut self) {
        let mut hasher = Sha256::new();

        // Hash all key-value pairs in a deterministic order
        let mut pairs: Vec<_> = self.map.iter().collect();
        pairs.sort_by_key(|(k, _)| k.clone());

        for (key, value) in pairs {
            hasher.update(&key);
            if let Ok(value_bytes) = bincode::serialize(value) {
                hasher.update(&value_bytes);
            }
        }

        self.root_hash = hasher.finalize().into();
    }

    pub fn get_root_hash(&self) -> [u8; 32] {
        self.root_hash
    }
}

impl Hash for CommittedCadoCache {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Hash all key-value pairs in a deterministic order
        let mut pairs: Vec<_> = self.map.iter().collect();
        pairs.sort_by_key(|(k, _)| k.clone());
        for (key, value) in pairs {
            key.hash(state);
            value.hash(state);
        }
        self.root_hash.hash(state);
    }
}

impl Serialize for CommittedCadoCache {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("BinaryPatriciaNode", 2)?;

        // Convert the map to a vector of key-value pairs for serialization
        let pairs: Vec<_> = self.map.iter().collect();
        state.serialize_field("pairs", &pairs)?;
        state.serialize_field("hash", &self.root_hash)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for CommittedCadoCache {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Helper {
            pairs: Vec<(Vec<u8>, CadoBody)>,
            hash: [u8; 32],
        }

        let helper = Helper::deserialize(deserializer)?;
        let mut node = CommittedCadoCache::new();
        for (key, value) in helper.pairs {
            node.insert(&key, value);
        }
        node.root_hash = helper.hash;
        Ok(node)
    }
}

impl Default for CommittedCadoCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eld_common::cado::CADOMetadata;
    use eld_common::cado::CadoType;
    use eld_common::constants::cado::{PATH_PREFIX_ACCOUNT, PATH_PREFIX_CONTENT_MANIFEST};

    fn create_test_cado(data: &[u8], owner: &str) -> CadoBody {
        CadoBody::mutable_new(data.to_vec(), CADOMetadata::new(CadoType::Account, owner))
    }

    #[test]
    fn test_production_paths() {
        let mut trie = CommittedCadoCache::new();

        // Insert account paths
        let account_paths = [
            format!(
                "{}{}",
                PATH_PREFIX_ACCOUNT, "0x23b1f0b6199479b5d04fb54e21df14d51530b7b1"
            ),
            format!(
                "{}{}",
                PATH_PREFIX_ACCOUNT, "0x34b12980c87bca4b81bd33b5d0793e70ad27dc93"
            ),
            format!(
                "{}{}",
                PATH_PREFIX_ACCOUNT, "0x5a42855687ed7988196e157ca40a3a133b09d9d5"
            ),
            format!(
                "{}{}",
                PATH_PREFIX_ACCOUNT, "0x5de5bb98d243d7a7d74ae1d592cef1becb43957c"
            ),
            format!(
                "{}{}",
                PATH_PREFIX_ACCOUNT, "0x5ffd39cc9d6ca5003d2414a784242f2fa9baaab3"
            ),
            format!(
                "{}{}",
                PATH_PREFIX_ACCOUNT, "0xdd6756748bfe442e61681e243896200b4347bd69"
            ),
            format!(
                "{}{}",
                PATH_PREFIX_ACCOUNT, "0xe17404c417fa10cc04fdf73604fcacca8d0a687c"
            ),
        ];

        // Insert content manifest paths
        let content_manifest_paths = [
            format!(
                "{}{}",
                PATH_PREFIX_CONTENT_MANIFEST,
                "0x23c076780fc0cec4fdd4f0015c9b9043866b824a61162940697080c0125f871a"
            ),
            format!(
                "{}{}",
                PATH_PREFIX_CONTENT_MANIFEST,
                "0x96f0c103ce43703463229798078b5638ebb06f464e817c50d4b1303337ebabb6"
            ),
        ];

        // Insert all paths into trie
        for path in account_paths.iter().chain(content_manifest_paths.iter()) {
            trie.insert(
                path.as_bytes(),
                create_test_cado(path.as_bytes(), "test_owner"),
            );
        }

        // Verify all paths can be retrieved
        for path in account_paths.iter().chain(content_manifest_paths.iter()) {
            assert!(
                trie.get(path.as_bytes()).is_some(),
                "Failed to retrieve path: {path}"
            );
        }

        // Specifically verify the content manifest that was reported missing
        let missing_manifest = format!(
            "{}{}",
            PATH_PREFIX_CONTENT_MANIFEST,
            "0x23c076780fc0cec4fdd4f0015c9b9043866b824a61162940697080c0125f871a"
        );
        assert!(
            trie.get(missing_manifest.as_bytes()).is_some(),
            "Failed to retrieve content manifest that was reported missing"
        );
    }

    #[test]
    fn without_infrastructure_strips_snapshot_paths() {
        use crate::app_state::app_state_snapshot::AppStateSnapshot;
        use eld_common::constants::cado::PATH_PREFIX_ACCOUNT;

        let mut cache = CommittedCadoCache::new();
        let account_path = format!(
            "{}{}",
            PATH_PREFIX_ACCOUNT, "0xe17404c417fa10cc04fdf73604fcacca8d0a687c"
        );
        cache.insert(
            account_path.as_bytes(),
            create_test_cado(account_path.as_bytes(), "owner"),
        );

        let infra_path = AppStateSnapshot::latest_path().expect("infra path");
        let infra_cado = CadoBody::immutable(
            vec![9, 9, 9],
            CADOMetadata::new(CadoType::AppStateSnapshot, "system"),
        );
        cache.insert(infra_path.as_str().as_bytes(), infra_cado);

        assert_eq!(cache.len(), 2);

        let filtered = cache.without_infrastructure();
        assert_eq!(filtered.len(), 1);
        assert!(filtered.get(account_path.as_bytes()).is_some());
        assert!(filtered.get(infra_path.as_str().as_bytes()).is_none());
    }
}
