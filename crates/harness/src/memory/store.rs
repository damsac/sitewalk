use std::path::PathBuf;

use crate::error::HarnessError;
use crate::memory::Memory;

/// Persistence seam for [`Memory`]. File-backed in production; swap for tests.
pub trait MemoryStore: Send + Sync {
    fn load(&self) -> Result<Memory, HarnessError>;
    fn save(&self, memory: &Memory) -> Result<(), HarnessError>;
}

/// JSON file store with atomic writes (write to `.tmp`, then rename) and
/// three rotating pre-save snapshots (`.1` newest … `.3` oldest) as the
/// rollback path if a reflection rewrite dropped something important.
pub struct FileMemoryStore {
    path: PathBuf,
}

impl FileMemoryStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        FileMemoryStore { path: path.into() }
    }

    fn rotate_snapshots(&self) {
        for i in (1..3usize).rev() {
            let from = self.path.with_extension(i.to_string());
            let to = self.path.with_extension((i + 1).to_string());
            if from.exists() {
                let _ = std::fs::rename(&from, &to);
            }
        }
        if self.path.exists() {
            let _ = std::fs::copy(&self.path, self.path.with_extension("1"));
        }
    }

    /// Prior memory versions, newest first (up to 3). Snapshots that fail to
    /// read or parse are skipped.
    pub fn snapshots(&self) -> Vec<Memory> {
        (1..=3usize)
            .filter_map(|i| std::fs::read_to_string(self.path.with_extension(i.to_string())).ok())
            .filter_map(|raw| serde_json::from_str(&raw).ok())
            .collect()
    }
}

impl MemoryStore for FileMemoryStore {
    fn load(&self) -> Result<Memory, HarnessError> {
        if !self.path.exists() {
            return Ok(Memory::default());
        }
        let raw = std::fs::read_to_string(&self.path)
            .map_err(|e| HarnessError::Storage(format!("read {}: {e}", self.path.display())))?;
        serde_json::from_str(&raw)
            .map_err(|e| HarnessError::Storage(format!("parse {}: {e}", self.path.display())))
    }

    fn save(&self, memory: &Memory) -> Result<(), HarnessError> {
        self.rotate_snapshots();
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| HarnessError::Storage(format!("mkdir {}: {e}", parent.display())))?;
        }
        let json = serde_json::to_string_pretty(memory)
            .map_err(|e| HarnessError::Storage(format!("serialize memory: {e}")))?;
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, json)
            .map_err(|e| HarnessError::Storage(format!("write {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| HarnessError::Storage(format!("rename to {}: {e}", self.path.display())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("harness-memstore-{tag}-{nanos}.json"))
    }

    #[test]
    fn load_missing_file_returns_default() {
        let store = FileMemoryStore::new(temp_path("missing"));
        assert_eq!(store.load().unwrap(), Memory::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = temp_path("roundtrip");
        let store = FileMemoryStore::new(path.clone());
        let mut m = Memory::default();
        m.remember("vocabulary", "french drain", 42);
        store.save(&m).unwrap();
        assert_eq!(store.load().unwrap(), m);
        assert!(!path.with_extension("tmp").exists(), "temp file cleaned up by rename");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn corrupt_file_is_a_storage_error() {
        let path = temp_path("corrupt");
        std::fs::write(&path, "not json").unwrap();
        let store = FileMemoryStore::new(path.clone());
        assert!(matches!(store.load(), Err(HarnessError::Storage(_))));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn save_creates_parent_dirs() {
        let dir = temp_path("nested");
        let path = dir.join("memory.json");
        let store = FileMemoryStore::new(path.clone());
        store.save(&Memory::default()).unwrap();
        assert!(path.exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn save_rotates_up_to_three_snapshots() {
        let path = temp_path("snapshots");
        let store = FileMemoryStore::new(path.clone());
        for i in 0..5u64 {
            let mut m = Memory::default();
            m.remember("v", &format!("version {i}"), i);
            store.save(&m).unwrap();
        }
        let snaps = store.snapshots();
        assert_eq!(snaps.len(), 3);
        assert_eq!(snaps[0].section_texts("v"), vec!["version 3"]);
        assert_eq!(snaps[2].section_texts("v"), vec!["version 1"]);
        for ext in ["1", "2", "3"] {
            std::fs::remove_file(path.with_extension(ext)).ok();
        }
        std::fs::remove_file(path).ok();
    }
}
