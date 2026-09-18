use crate::address::Address;
use crate::capacity_proof::SlotState;
use crate::error::EldError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::info;

/// [`EldError::NotFoundError::resource_type`] when `content_id` has no mapping in [`SlotAllocator`]'s content-slot map.
pub const CONTENT_ID_NOT_IN_SLOT_MAP: &str = "content_id_not_in_slot_map";

/// Individual slot in the capacity file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slot {
    pub offset: u64,
    pub size: usize,
    pub state: SlotState,
}

/// Slot map tracking all slots in the capacity file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotMap {
    pub slots: Vec<Slot>,
    pub capacity_bytes: u64,
    pub seed: [u8; 32],
    pub provider_id: Address,
}

impl SlotMap {
    pub fn to_json(&self) -> Result<String, EldError> {
        serde_json::to_string_pretty(self).map_err(|e| EldError::StorageError {
            operation: "serialize_slot_map".to_string(),
            details: format!("Failed to serialize slot map: {e}"),
        })
    }

    pub fn from_json(slot_map_json: &str) -> Result<Self, EldError> {
        serde_json::from_str(slot_map_json).map_err(|e| EldError::StorageError {
            operation: "deserialize_slot_map".to_string(),
            details: format!("Failed to deserialize slot map: {e}"),
        })
    }
}

/// Manages slot map creation, loading, and updates
pub struct SlotAllocator {
    slot_map: Option<SlotMap>,
    slots_file: PathBuf,
    capacity_file: PathBuf,
    content_slot_map: HashMap<String, Vec<(usize, usize)>>, // content_id -> (slot_index, actual_size) pairs
    content_slot_map_file: PathBuf,
}

impl SlotAllocator {
    /// Create a new SlotMapManager
    pub fn new(capacity_dir: &Path, provider_id: &Address) -> Self {
        let capacity_file = capacity_dir.join(format!("capacity_{provider_id}.dat"));
        let slots_file = capacity_file.with_extension("slots.json");
        let content_slot_map_file = capacity_file.with_extension("content-slot-map.json");

        Self {
            slot_map: None,
            slots_file,
            capacity_file,
            content_slot_map: HashMap::new(),
            content_slot_map_file,
        }
    }

    /// Check if slot map exists on disk
    pub fn slot_map_exists(&self) -> bool {
        self.slots_file.exists()
    }

    /// Load slot map from disk
    pub fn load_slot_map(&mut self) -> Result<(), EldError> {
        if !self.slot_map_exists() {
            return Err(EldError::StorageError {
                operation: "load_slot_map".to_string(),
                details: format!(
                    "Slot map file does not exist: {}",
                    self.slots_file.display()
                ),
            });
        }

        let slot_map_json =
            std::fs::read_to_string(&self.slots_file).map_err(|e| EldError::StorageError {
                operation: "load_slot_map".to_string(),
                details: format!("Failed to read slot map file: {e}"),
            })?;

        let slot_map = SlotMap::from_json(&slot_map_json)?;

        info!(
            provider_id = %slot_map.provider_id,
            capacity_bytes = slot_map.capacity_bytes,
            slot_count = slot_map.slots.len(),
            "Loaded slot map from disk"
        );

        self.slot_map = Some(slot_map);

        // Also load content-slot map if it exists
        self.load_content_slot_map().unwrap_or_else(|e| {
            info!(
                "Content-slot map file not found or invalid, starting with empty map: {}",
                e
            );
        });

        Ok(())
    }

    /// Load content-slot map from disk
    pub fn load_content_slot_map(&mut self) -> Result<(), EldError> {
        if !self.content_slot_map_file.exists() {
            return Err(EldError::StorageError {
                operation: "load_content_slot_map".to_string(),
                details: format!(
                    "Content-slot map file does not exist: {}",
                    self.content_slot_map_file.display()
                ),
            });
        }

        let map_json = std::fs::read_to_string(&self.content_slot_map_file).map_err(|e| {
            EldError::StorageError {
                operation: "load_content_slot_map".to_string(),
                details: format!("Failed to read content-slot map file: {e}"),
            }
        })?;

        let map: HashMap<String, Vec<(usize, usize)>> =
            serde_json::from_str(&map_json).map_err(|e| EldError::StorageError {
                operation: "load_content_slot_map".to_string(),
                details: format!("Failed to deserialize content-slot map: {e}"),
            })?;

        info!(
            content_slot_map_file = %self.content_slot_map_file.display(),
            content_count = map.len(),
            "Loaded content-slot map from disk"
        );

        self.content_slot_map = map;
        Ok(())
    }

    /// Save content-slot map to disk
    pub fn save_content_slot_map(&self) -> Result<(), EldError> {
        let map_json = serde_json::to_string_pretty(&self.content_slot_map).map_err(|e| {
            EldError::StorageError {
                operation: "save_content_slot_map".to_string(),
                details: format!("Failed to serialize content-slot map: {e}"),
            }
        })?;

        // Ensure parent directory exists
        if let Some(parent) = self.content_slot_map_file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| EldError::StorageError {
                operation: "save_content_slot_map".to_string(),
                details: format!("Failed to create directory: {e}"),
            })?;
        }

        std::fs::write(&self.content_slot_map_file, map_json).map_err(|e| {
            EldError::StorageError {
                operation: "save_content_slot_map".to_string(),
                details: format!("Failed to write content-slot map file: {e}"),
            }
        })?;

        info!(
            content_slot_map_file = %self.content_slot_map_file.display(),
            content_count = self.content_slot_map.len(),
            "Saved content-slot map to disk"
        );

        Ok(())
    }

    /// Save slot map to disk
    pub fn save_slot_map(&self) -> Result<(), EldError> {
        let slot_map = self.slot_map_ref()?;

        let slot_map_json = slot_map.to_json()?;

        // Ensure parent directory exists
        if let Some(parent) = self.slots_file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| EldError::StorageError {
                operation: "save_slot_map".to_string(),
                details: format!("Failed to create directory: {e}"),
            })?;
        }

        std::fs::write(&self.slots_file, slot_map_json).map_err(|e| EldError::StorageError {
            operation: "save_slot_map".to_string(),
            details: format!("Failed to write slot map file: {e}"),
        })?;

        info!(
            slots_file = %self.slots_file.display(),
            "Saved slot map to disk"
        );

        Ok(())
    }

    /// Get the current slot map (if loaded)
    pub fn get_slot_map(&self) -> Option<&SlotMap> {
        self.slot_map.as_ref()
    }

    /// Get mutable reference to slot map
    pub fn get_slot_map_mut(&mut self) -> Option<&mut SlotMap> {
        self.slot_map.as_mut()
    }

    pub fn slot_map_ref(&self) -> Result<&SlotMap, EldError> {
        self.slot_map
            .as_ref()
            .ok_or_else(|| EldError::StorageError {
                operation: "slot_map_ref".to_string(),
                details: "Slot map not loaded".to_string(),
            })
    }

    pub fn slot_map_mut(&mut self) -> Result<&mut SlotMap, EldError> {
        self.slot_map
            .as_mut()
            .ok_or_else(|| EldError::StorageError {
                operation: "slot_map_mut".to_string(),
                details: "Slot map not loaded".to_string(),
            })
    }

    /// Set slot map (typically after creation)
    pub fn set_slot_map(&mut self, slot_map: SlotMap) {
        self.slot_map = Some(slot_map);
    }

    /// Find open slots for content storage
    /// Returns indices of open slots
    pub fn find_open_slots(&self, num_slots_needed: usize) -> Result<Vec<usize>, EldError> {
        let slot_map = self.slot_map_ref()?;

        let mut open_slots = Vec::new();
        for (idx, slot) in slot_map.slots.iter().enumerate() {
            if matches!(slot.state, SlotState::Open) {
                open_slots.push(idx);
                if open_slots.len() >= num_slots_needed {
                    break;
                }
            }
        }

        if open_slots.len() < num_slots_needed {
            return Err(EldError::StorageError {
                operation: "find_open_slots".to_string(),
                details: format!(
                    "Not enough open slots. Need {}, found {}",
                    num_slots_needed,
                    open_slots.len()
                ),
            });
        }

        Ok(open_slots)
    }

    /// Update slot states to Content
    /// slot_sizes: Vec of (slot_index, actual_size) pairs
    pub fn mark_slots_as_content(
        &mut self,
        slot_sizes: &[(usize, usize)],
        deal_id: String,
        committed_hash: [u8; 32],
    ) -> Result<(), EldError> {
        let slot_map = self.slot_map_mut()?;

        for &(idx, _actual_size) in slot_sizes {
            if idx >= slot_map.slots.len() {
                return Err(EldError::StorageError {
                    operation: "mark_slots_as_content".to_string(),
                    details: format!("Slot index {idx} out of bounds"),
                });
            }

            slot_map.slots[idx].state = SlotState::Content {
                deal_id: deal_id.clone(),
                committed_hash,
            };
        }

        // Update content-slot map with (slot_index, actual_size) pairs
        self.content_slot_map
            .insert(deal_id.clone(), slot_sizes.to_vec());

        // Save content-slot map to disk
        self.save_content_slot_map()?;

        info!(
            deal_id = %deal_id,
            slots_updated = slot_sizes.len(),
            "Marked slots as Content and updated content-slot map"
        );

        Ok(())
    }

    /// Reference to the content-slot map (content_id -> list of (slot_index, actual_size)).
    pub fn get_content_slot_map(&self) -> &HashMap<String, Vec<(usize, usize)>> {
        &self.content_slot_map
    }

    /// Find slot indices and sizes for a given content_id
    /// Returns (slot_index, actual_size) pairs in the order they were stored
    pub fn find_slots_by_content_id(
        &self,
        content_id: &str,
    ) -> Result<Vec<(usize, usize)>, EldError> {
        self.content_slot_map
            .get(content_id)
            .cloned()
            .ok_or_else(|| EldError::NotFoundError {
                resource_type: CONTENT_ID_NOT_IN_SLOT_MAP.to_string(),
                identifier: content_id.to_string(),
            })
    }

    /// Remove content mapping and mark all associated slots back to Open.
    /// Returns the released `(slot_index, actual_size)` entries.
    pub fn remove_content_by_id(
        &mut self,
        content_id: &str,
    ) -> Result<Vec<(usize, usize)>, EldError> {
        let slot_sizes = self
            .content_slot_map
            .get(content_id)
            .cloned()
            .ok_or_else(|| EldError::NotFoundError {
                resource_type: CONTENT_ID_NOT_IN_SLOT_MAP.to_string(),
                identifier: content_id.to_string(),
            })?;

        {
            let slot_map = self.slot_map_mut()?;
            for (idx, _) in &slot_sizes {
                if *idx >= slot_map.slots.len() {
                    return Err(EldError::StorageError {
                        operation: "remove_content_by_id".to_string(),
                        details: format!("Slot index {idx} out of bounds"),
                    });
                }
                slot_map.slots[*idx].state = SlotState::Open;
            }
        }

        self.content_slot_map.remove(content_id);
        self.save_content_slot_map()?;

        Ok(slot_sizes)
    }

    /// Get capacity file path
    pub fn capacity_file_path(&self) -> &Path {
        &self.capacity_file
    }

    /// Get slots file path
    pub fn slots_file_path(&self) -> &Path {
        &self.slots_file
    }

    /// Write chunk data to a specific slot in the capacity file
    /// Returns the hash of the written data
    pub fn write_chunk_to_slot(
        &self,
        slot_index: usize,
        data: &[u8],
    ) -> Result<[u8; 32], EldError> {
        use sha2::{Digest, Sha256};
        use std::fs::OpenOptions;
        use std::io::{Seek, SeekFrom, Write};

        let slot_map = self.slot_map_ref()?;

        if slot_index >= slot_map.slots.len() {
            return Err(EldError::StorageError {
                operation: "write_chunk_to_slot".to_string(),
                details: format!("Slot index {slot_index} out of bounds"),
            });
        }

        let slot = &slot_map.slots[slot_index];

        // Ensure data fits in slot (pad with zeros if needed)
        let mut chunk_data = data.to_vec();
        if chunk_data.len() < slot.size {
            chunk_data.resize(slot.size, 0);
        } else if chunk_data.len() > slot.size {
            return Err(EldError::StorageError {
                operation: "write_chunk_to_slot".to_string(),
                details: format!(
                    "Data size {} exceeds slot size {}",
                    chunk_data.len(),
                    slot.size
                ),
            });
        }

        // Remove immutable flag before writing (file may have been set immutable during allocation)
        #[cfg(target_os = "macos")]
        {
            use std::process::Command;
            if let Some(capacity_file_str) = self.capacity_file.to_str() {
                let _ = Command::new("chflags")
                    .args(["nouchg", capacity_file_str])
                    .output();
            }
        }

        #[cfg(target_os = "linux")]
        {
            use std::process::Command;
            if let Some(capacity_file_str) = self.capacity_file.to_str() {
                let _ = Command::new("chattr")
                    .args(["-i", capacity_file_str])
                    .output();
            }
        }

        // Open capacity file and write data at slot offset
        let mut file = OpenOptions::new()
            .write(true)
            .open(&self.capacity_file)
            .map_err(|e| EldError::StorageError {
                operation: "write_chunk_to_slot".to_string(),
                details: format!("Failed to open capacity file: {e}"),
            })?;

        file.seek(SeekFrom::Start(slot.offset))
            .map_err(|e| EldError::StorageError {
                operation: "write_chunk_to_slot".to_string(),
                details: format!("Failed to seek to slot offset: {e}"),
            })?;

        file.write_all(&chunk_data)
            .map_err(|e| EldError::StorageError {
                operation: "write_chunk_to_slot".to_string(),
                details: format!("Failed to write chunk data: {e}"),
            })?;

        // Calculate hash of the written data
        let mut hasher = Sha256::new();
        hasher.update(b"CHUNK_HASH");
        hasher.update(&chunk_data);
        let hash = hasher.finalize().into();

        Ok(hash)
    }

    /// Read chunk data from a specific slot in the capacity file
    pub fn read_chunk_from_slot(&self, slot_index: usize) -> Result<Vec<u8>, EldError> {
        use std::fs::File;
        use std::io::{Read, Seek, SeekFrom};

        let slot_map = self.slot_map_ref()?;

        if slot_index >= slot_map.slots.len() {
            return Err(EldError::StorageError {
                operation: "read_chunk_from_slot".to_string(),
                details: format!("Slot index {slot_index} out of bounds"),
            });
        }

        let slot = &slot_map.slots[slot_index];

        // Open capacity file and read data at slot offset
        let mut file = File::open(&self.capacity_file).map_err(|e| EldError::StorageError {
            operation: "read_chunk_from_slot".to_string(),
            details: format!("Failed to open capacity file: {e}"),
        })?;

        file.seek(SeekFrom::Start(slot.offset))
            .map_err(|e| EldError::StorageError {
                operation: "read_chunk_from_slot".to_string(),
                details: format!("Failed to seek to slot offset: {e}"),
            })?;

        let mut chunk_data = vec![0u8; slot.size];
        file.read_exact(&mut chunk_data)
            .map_err(|e| EldError::StorageError {
                operation: "read_chunk_from_slot".to_string(),
                details: format!("Failed to read chunk data: {e}"),
            })?;

        Ok(chunk_data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capacity_proof::SlotState;
    use tempfile::TempDir;

    const TEST_PROVIDER: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn test_provider() -> Address {
        Address::parse_hex_str(TEST_PROVIDER).expect("test provider address")
    }

    #[test]
    fn slot_map_provider_id_round_trip_json() {
        let slot_map = SlotMap {
            slots: vec![],
            capacity_bytes: 0,
            seed: [0x42; 32],
            provider_id: test_provider(),
        };

        let json = slot_map.to_json().expect("serialize");
        assert!(json.contains(TEST_PROVIDER));

        let loaded = SlotMap::from_json(&json).expect("deserialize");
        assert_eq!(loaded.provider_id, slot_map.provider_id);
    }

    #[test]
    fn test_slot_allocator_new() {
        let temp_dir = TempDir::new().unwrap();
        let provider_id = test_provider();
        let allocator = SlotAllocator::new(temp_dir.path(), &provider_id);

        assert!(!allocator.slot_map_exists());
        assert!(allocator.get_slot_map().is_none());
    }

    #[test]
    fn test_slot_map_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let provider_id = test_provider();
        let mut allocator = SlotAllocator::new(temp_dir.path(), &provider_id);

        // Create a test slot map
        let slot_map = SlotMap {
            slots: vec![
                Slot {
                    offset: 0,
                    size: 1024,
                    state: SlotState::Proof,
                },
                Slot {
                    offset: 1024,
                    size: 1024,
                    state: SlotState::Open,
                },
            ],
            capacity_bytes: 2048,
            seed: [0x42; 32],
            provider_id,
        };

        allocator.set_slot_map(slot_map.clone());
        allocator.save_slot_map().unwrap();

        // Verify file exists
        assert!(allocator.slot_map_exists());

        // Load it back
        let mut allocator2 = SlotAllocator::new(temp_dir.path(), &provider_id);
        allocator2.load_slot_map().unwrap();

        let loaded_map = allocator2.get_slot_map().unwrap();
        assert_eq!(loaded_map.provider_id, slot_map.provider_id);
        assert_eq!(loaded_map.capacity_bytes, slot_map.capacity_bytes);
        assert_eq!(loaded_map.slots.len(), slot_map.slots.len());
        assert_eq!(loaded_map.seed, slot_map.seed);
    }

    #[test]
    fn test_slot_map_load_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let provider_id = test_provider();
        let mut allocator = SlotAllocator::new(temp_dir.path(), &provider_id);

        // Try to load non-existent slot map
        let result = allocator.load_slot_map();
        assert!(result.is_err());
    }

    #[test]
    fn test_find_open_slots() {
        let temp_dir = TempDir::new().unwrap();
        let provider_id = test_provider();
        let mut allocator = SlotAllocator::new(temp_dir.path(), &provider_id);

        // Create slot map with mix of Proof, Open, and Content slots
        let slot_map = SlotMap {
            slots: vec![
                Slot {
                    offset: 0,
                    size: 1024,
                    state: SlotState::Proof,
                },
                Slot {
                    offset: 1024,
                    size: 1024,
                    state: SlotState::Open,
                },
                Slot {
                    offset: 2048,
                    size: 1024,
                    state: SlotState::Open,
                },
                Slot {
                    offset: 3072,
                    size: 1024,
                    state: SlotState::Content {
                        deal_id: "deal1".to_string(),
                        committed_hash: [0xAA; 32],
                    },
                },
                Slot {
                    offset: 4096,
                    size: 1024,
                    state: SlotState::Open,
                },
            ],
            capacity_bytes: 5120,
            seed: [0x42; 32],
            provider_id,
        };

        allocator.set_slot_map(slot_map);

        // Find 2 open slots
        let open_slots = allocator.find_open_slots(2).unwrap();
        assert_eq!(open_slots.len(), 2);
        assert_eq!(open_slots, vec![1, 2]); // Indices 1 and 2 are Open

        // Find 3 open slots (should work)
        let open_slots = allocator.find_open_slots(3).unwrap();
        assert_eq!(open_slots.len(), 3);
        assert_eq!(open_slots, vec![1, 2, 4]);

        // Try to find 4 open slots (should fail - only 3 available)
        let result = allocator.find_open_slots(4);
        assert!(result.is_err());
    }

    #[test]
    fn test_find_open_slots_no_slot_map() {
        let temp_dir = TempDir::new().unwrap();
        let provider_id = test_provider();
        let allocator = SlotAllocator::new(temp_dir.path(), &provider_id);

        // Try to find open slots without slot map
        let result = allocator.find_open_slots(1);
        assert!(result.is_err());
    }

    #[test]
    fn test_mark_slots_as_content() {
        let temp_dir = TempDir::new().unwrap();
        let provider_id = test_provider();
        let mut allocator = SlotAllocator::new(temp_dir.path(), &provider_id);

        // Create slot map with open slots
        let slot_map = SlotMap {
            slots: vec![
                Slot {
                    offset: 0,
                    size: 1024,
                    state: SlotState::Open,
                },
                Slot {
                    offset: 1024,
                    size: 1024,
                    state: SlotState::Open,
                },
                Slot {
                    offset: 2048,
                    size: 1024,
                    state: SlotState::Proof,
                },
            ],
            capacity_bytes: 3072,
            seed: [0x42; 32],
            provider_id,
        };

        allocator.set_slot_map(slot_map);

        let deal_id = "test_deal".to_string();
        let committed_hash = [0xBB; 32];

        // Mark slots 0 and 1 as Content (with actual sizes)
        allocator
            .mark_slots_as_content(&[(0, 1024), (1, 1024)], deal_id.clone(), committed_hash)
            .unwrap();

        let updated_map = allocator.get_slot_map().unwrap();
        assert!(matches!(
            updated_map.slots[0].state,
            SlotState::Content { .. }
        ));
        assert!(matches!(
            updated_map.slots[1].state,
            SlotState::Content { .. }
        ));
        assert!(matches!(updated_map.slots[2].state, SlotState::Proof));

        // Verify deal_id and hash
        if let SlotState::Content {
            deal_id: d,
            committed_hash: h,
        } = &updated_map.slots[0].state
        {
            assert_eq!(d, &deal_id);
            assert_eq!(h, &committed_hash);
        }
    }

    #[test]
    fn test_mark_slots_as_content_out_of_bounds() {
        let temp_dir = TempDir::new().unwrap();
        let provider_id = test_provider();
        let mut allocator = SlotAllocator::new(temp_dir.path(), &provider_id);

        let slot_map = SlotMap {
            slots: vec![Slot {
                offset: 0,
                size: 1024,
                state: SlotState::Open,
            }],
            capacity_bytes: 1024,
            seed: [0x42; 32],
            provider_id,
        };

        allocator.set_slot_map(slot_map);

        // Try to mark slot index 10 (out of bounds)
        let result = allocator.mark_slots_as_content(&[(10, 1024)], "deal".to_string(), [0xAA; 32]);
        assert!(result.is_err());
    }

    #[test]
    fn test_slot_map_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let provider_id = test_provider();
        let mut allocator = SlotAllocator::new(temp_dir.path(), &provider_id);

        // Create and save slot map
        let slot_map = SlotMap {
            slots: vec![
                Slot {
                    offset: 0,
                    size: 1024,
                    state: SlotState::Proof,
                },
                Slot {
                    offset: 1024,
                    size: 1024,
                    state: SlotState::Open,
                },
            ],
            capacity_bytes: 2048,
            seed: [0x42; 32],
            provider_id,
        };

        allocator.set_slot_map(slot_map.clone());
        allocator.save_slot_map().unwrap();

        // Create new allocator instance and load
        let mut allocator2 = SlotAllocator::new(temp_dir.path(), &provider_id);
        allocator2.load_slot_map().unwrap();

        let loaded = allocator2.get_slot_map().unwrap();
        assert_eq!(loaded.slots.len(), slot_map.slots.len());
        assert_eq!(loaded.slots[0].offset, slot_map.slots[0].offset);
        assert_eq!(loaded.slots[0].size, slot_map.slots[0].size);
    }

    #[test]
    fn test_get_slot_map_mut() {
        let temp_dir = TempDir::new().unwrap();
        let provider_id = test_provider();
        let mut allocator = SlotAllocator::new(temp_dir.path(), &provider_id);

        let slot_map = SlotMap {
            slots: vec![Slot {
                offset: 0,
                size: 1024,
                state: SlotState::Open,
            }],
            capacity_bytes: 1024,
            seed: [0x42; 32],
            provider_id,
        };

        allocator.set_slot_map(slot_map);

        // Get mutable reference and modify
        if let Some(map) = allocator.get_slot_map_mut() {
            map.slots[0].offset = 2048;
        }

        let updated = allocator.get_slot_map().unwrap();
        assert_eq!(updated.slots[0].offset, 2048);
    }
}
