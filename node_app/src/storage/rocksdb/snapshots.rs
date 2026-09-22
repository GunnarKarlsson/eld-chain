use crate::storage::traits::{CADOStorage, SnapshotChunk, SnapshotMetadata, SnapshotStorage};
use eld_common::cado::{CADOMetadata, CadoPath, CadoPathKey, CadoType};
use eld_common::constants::cado::PATH_PREFIX_SNAPSHOT_METADATA;
use eld_common::error::EldError;
use hex;
use sha2::Digest;

use super::RocksDBStorage;

impl SnapshotStorage for RocksDBStorage {
    fn put_snapshot_metadata(&self, metadata: &SnapshotMetadata) -> Result<(), EldError> {
        // Hash the height to create a proper hex-encoded name
        let height_hash = sha2::Sha256::digest(metadata.height.to_string());
        let snapshot_metadata_id = format!("0x{}", hex::encode(height_hash));
        let path = CadoPath::new(
            CadoType::SnapshotMetadata,
            CadoPathKey::Name(&snapshot_metadata_id),
        )?;
        let data = bincode::serialize(metadata).map_err(|e| EldError::StorageError {
            operation: "serialize_snapshot_metadata".to_string(),
            details: format!("Failed to serialize SnapshotMetadata: {e}"),
        })?;
        self.put_cado_data(
            path,
            data,
            CADOMetadata::new(CadoType::SnapshotMetadata, "system"),
        )?;
        Ok(())
    }

    fn get_snapshot_metadata(&self, height: i64) -> Result<Option<SnapshotMetadata>, EldError> {
        // Hash the height to create a proper hex-encoded name
        let height_hash = sha2::Sha256::digest(height.to_string());
        let snapshot_metadata_id = format!("0x{}", hex::encode(height_hash));
        let path = CadoPath::new(
            CadoType::SnapshotMetadata,
            CadoPathKey::Name(&snapshot_metadata_id),
        )?;

        match self.get_deserialized_cado_by_path::<SnapshotMetadata>(path) {
            Ok(m) => Ok(Some(m)),
            Err(EldError::NotFoundError { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn get_latest_snapshot_metadata(&self) -> Result<Option<SnapshotMetadata>, EldError> {
        // Get all snapshot metadata CADOs and find the one with highest height
        let paths = self.get_cado_paths_by_prefix(PATH_PREFIX_SNAPSHOT_METADATA)?;
        let mut latest_metadata = None;
        let mut latest_height = i64::MIN;

        for path in paths {
            match self.get_deserialized_cado_by_path::<SnapshotMetadata>(path.clone()) {
                Ok(metadata) => {
                    if metadata.height > latest_height {
                        latest_height = metadata.height;
                        latest_metadata = Some(metadata);
                    }
                }
                Err(EldError::NotFoundError { .. }) => {
                    tracing::warn!(path = %path, "No CADO found for snapshot metadata path during get_latest_snapshot_metadata");
                }
                Err(e) => {
                    tracing::warn!(?e, path = %path, "Failed to deserialize SnapshotMetadata during get_latest_snapshot_metadata");
                }
            }
        }

        Ok(latest_metadata)
    }

    fn list_snapshots(&self, limit: u32) -> Result<Vec<SnapshotMetadata>, EldError> {
        let paths = self.get_cado_paths_by_prefix(PATH_PREFIX_SNAPSHOT_METADATA)?;
        let mut snapshots = Vec::new();

        for path in paths {
            match self.get_deserialized_cado_by_path::<SnapshotMetadata>(path.clone()) {
                Ok(metadata) => {
                    snapshots.push(metadata);
                }
                Err(EldError::NotFoundError { .. }) => {
                    tracing::warn!(path = %path, "No CADO found for snapshot metadata path during list_snapshots");
                }
                Err(e) => {
                    tracing::warn!(?e, path = %path, "Failed to deserialize SnapshotMetadata during list_snapshots");
                }
            }
            if snapshots.len() >= limit as usize {
                break;
            }
        }

        // Sort by height in descending order
        snapshots.sort_by(|a, b| b.height.cmp(&a.height));
        Ok(snapshots)
    }

    fn put_snapshot_chunk(&self, height: i64, chunk: &SnapshotChunk) -> Result<(), EldError> {
        // Hash the height and chunk index to create a proper hex-encoded name
        let chunk_id = format!("{}_{}", height, chunk.index);
        let chunk_hash = sha2::Sha256::digest(chunk_id);
        let snapshot_chunk_id = format!("0x{}", hex::encode(chunk_hash));
        let path = CadoPath::new(
            CadoType::SnapshotChunk,
            CadoPathKey::Name(&snapshot_chunk_id),
        )?;
        let data = chunk.serialize_bin()?;
        self.put_cado_data(
            path,
            data,
            CADOMetadata::new(CadoType::SnapshotChunk, "system"),
        )?;
        Ok(())
    }

    fn get_snapshot_chunk(
        &self,
        height: i64,
        chunk_index: u32,
    ) -> Result<Option<SnapshotChunk>, EldError> {
        // Hash the height and chunk index to create a proper hex-encoded name
        let chunk_id = format!("{height}_{chunk_index}");
        let chunk_hash = sha2::Sha256::digest(chunk_id);
        let snapshot_chunk_id = format!("0x{}", hex::encode(chunk_hash));
        let path = CadoPath::new(
            CadoType::SnapshotChunk,
            CadoPathKey::Name(&snapshot_chunk_id),
        )?;

        match self.get_deserialized_cado_by_path::<SnapshotChunk>(path) {
            Ok(c) => Ok(Some(c)),
            Err(EldError::NotFoundError { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn delete_snapshot_chunk(&self, height: i64, chunk_index: u32) -> Result<(), EldError> {
        // Hash the height and chunk index to create a proper hex-encoded name
        let chunk_id = format!("{height}_{chunk_index}");
        let chunk_hash = sha2::Sha256::digest(chunk_id);
        let snapshot_chunk_id = format!("0x{}", hex::encode(chunk_hash));
        let path = CadoPath::new(
            CadoType::SnapshotChunk,
            CadoPathKey::Name(&snapshot_chunk_id),
        )?;
        self.system_delete_cado(path, "system")?;
        Ok(())
    }

    fn delete_snapshot_metadata(&self, height: i64) -> Result<(), EldError> {
        // Hash the height to create a proper hex-encoded name
        let height_hash = sha2::Sha256::digest(height.to_string());
        let snapshot_metadata_id = format!("0x{}", hex::encode(height_hash));
        let path = CadoPath::new(
            CadoType::SnapshotMetadata,
            CadoPathKey::Name(&snapshot_metadata_id),
        )?;
        self.system_delete_cado(path, "system")?;
        Ok(())
    }
}

#[cfg(test)]
impl crate::storage::traits::SnapshotStorageTestExt for RocksDBStorage {
    fn verify_snapshot(&self, height: i64) -> Result<bool, EldError> {
        let height_hash = sha2::Sha256::digest(height.to_string());
        let snapshot_metadata_id = format!("0x{}", hex::encode(height_hash));
        let metadata_path = CadoPath::new(
            CadoType::SnapshotMetadata,
            CadoPathKey::Name(&snapshot_metadata_id),
        )?;

        if self.get_cado_by_path(metadata_path)?.is_none() {
            return Ok(false);
        }

        let paths =
            self.get_cado_paths_by_prefix(eld_common::constants::cado::PATH_PREFIX_SNAPSHOT_CHUNK)?;
        for path in paths {
            match self.get_deserialized_cado_by_path::<SnapshotChunk>(path.clone()) {
                Ok(chunk) => {
                    let chunk_id = format!("{}_{}", height, chunk.index);
                    let chunk_hash = sha2::Sha256::digest(chunk_id);
                    let snapshot_chunk_id = format!("0x{}", hex::encode(chunk_hash));
                    let expected_path = CadoPath::new(
                        CadoType::SnapshotChunk,
                        CadoPathKey::Name(&snapshot_chunk_id),
                    )?;
                    if path.as_str() == expected_path.as_str() {
                        return Ok(true);
                    }
                }
                Err(EldError::NotFoundError { .. }) => {
                    tracing::warn!(path = %path, "No CADO found for snapshot chunk path during verify_snapshot");
                }
                Err(e) => {
                    tracing::warn!(?e, path = %path, "Failed to deserialize SnapshotChunk during verify_snapshot");
                }
            }
        }

        Ok(false)
    }

    fn get_snapshot_chunk_count(&self, height: i64) -> Result<u32, EldError> {
        let paths =
            self.get_cado_paths_by_prefix(eld_common::constants::cado::PATH_PREFIX_SNAPSHOT_CHUNK)?;
        let mut count = 0;

        for path in paths {
            match self.get_deserialized_cado_by_path::<SnapshotChunk>(path.clone()) {
                Ok(chunk) => {
                    let chunk_id = format!("{}_{}", height, chunk.index);
                    let chunk_hash = sha2::Sha256::digest(chunk_id);
                    let snapshot_chunk_id = format!("0x{}", hex::encode(chunk_hash));
                    let expected_path = CadoPath::new(
                        CadoType::SnapshotChunk,
                        CadoPathKey::Name(&snapshot_chunk_id),
                    )?;
                    if path.as_str() == expected_path.as_str() {
                        count += 1;
                    }
                }
                Err(EldError::NotFoundError { .. }) => {
                    tracing::warn!(path = %path, "No CADO found for snapshot chunk path during get_snapshot_chunk_count");
                }
                Err(e) => {
                    tracing::warn!(?e, path = %path, "Failed to deserialize SnapshotChunk during get_snapshot_chunk_count");
                }
            }
        }

        Ok(count)
    }

    fn snapshot_exists(&self, height: i64) -> Result<bool, EldError> {
        let height_hash = sha2::Sha256::digest(height.to_string());
        let snapshot_metadata_id = format!("0x{}", hex::encode(height_hash));
        let path = CadoPath::new(
            CadoType::SnapshotMetadata,
            CadoPathKey::Name(&snapshot_metadata_id),
        )?;
        Ok(self.get_cado_by_path(path)?.is_some())
    }

    fn prune_snapshots(&self, keep_last_n: u32) -> Result<(), EldError> {
        let snapshots = self.list_snapshots(u32::MAX)?;

        if snapshots.len() <= keep_last_n as usize {
            return Ok(());
        }

        let snapshots_to_delete = &snapshots[keep_last_n as usize..];

        for snapshot in snapshots_to_delete {
            let height_hash = sha2::Sha256::digest(snapshot.height.to_string());
            let snapshot_metadata_id = format!("0x{}", hex::encode(height_hash));
            let metadata_path = CadoPath::new(
                CadoType::SnapshotMetadata,
                CadoPathKey::Name(&snapshot_metadata_id),
            )?;

            if let Err(e) = self.system_delete_cado(metadata_path, "system") {
                tracing::error!(
                    ?e,
                    height = snapshot.height,
                    "Failed to delete snapshot metadata during prune_snapshots"
                );
            }

            let paths = self.get_cado_paths_by_prefix(
                eld_common::constants::cado::PATH_PREFIX_SNAPSHOT_CHUNK,
            )?;

            for path in paths {
                match self.get_deserialized_cado_by_path::<SnapshotChunk>(path.clone()) {
                    Ok(chunk) => {
                        let chunk_id = format!("{}_{}", snapshot.height, chunk.index);
                        let chunk_hash = sha2::Sha256::digest(chunk_id);
                        let snapshot_chunk_id = format!("0x{}", hex::encode(chunk_hash));
                        let expected_path = CadoPath::new(
                            CadoType::SnapshotChunk,
                            CadoPathKey::Name(&snapshot_chunk_id),
                        )?;
                        if path.as_str() == expected_path.as_str() {
                            if let Err(e) = self.system_delete_cado(path, "system") {
                                tracing::error!(
                                    ?e,
                                    chunk_index = chunk.index,
                                    height = snapshot.height,
                                    "Failed to delete snapshot chunk during prune_snapshots"
                                );
                            }
                        }
                    }
                    Err(EldError::NotFoundError { .. }) => {
                        tracing::warn!(path = %path, "No CADO found for snapshot chunk path during prune_snapshots");
                    }
                    Err(e) => {
                        tracing::warn!(?e, path = %path, "Failed to deserialize SnapshotChunk during prune_snapshots");
                    }
                }
            }
        }

        Ok(())
    }
}
