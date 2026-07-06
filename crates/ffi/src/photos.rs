//! Photo CRUD across UniFFI (Plan 11 D7). Engine-keyed by `session_id` —
//! photos are attachable during a walk AND at review time (there is no live
//! `WalkSession` once processed), so this follows the vocabulary.rs precedent
//! (`crates/ffi/src/vocabulary.rs`), not a `WalkSession`-scoped method. Every
//! method is panic-free across FFI (Plan 07 CANON): a poisoned lock or a
//! store error surfaces as `EngineError::Photo`, never a panic.

use crate::engine::{EngineError, MurmurEngine};

/// A display-copy-free projection of `murmur_core::Photo` (D-Plan07 posture):
/// the shell resolves `filename` to a real file URL under its own
/// `<Documents>/photos/` directory (Plan 11 D4) — core never touches bytes.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct PhotoRef {
    pub id: String,
    pub session_id: String,
    pub item_id: Option<String>,
    pub filename: String,
    pub captured_at: u64,
}

fn photo_ref(p: &murmur_core::Photo) -> PhotoRef {
    PhotoRef {
        id: p.id.clone(),
        session_id: p.session_id.clone(),
        item_id: p.item_id.clone(),
        filename: p.filename.clone(),
        captured_at: p.captured_at,
    }
}

impl MurmurEngine {
    fn photo_err(msg: impl Into<String>) -> EngineError {
        EngineError::Photo(msg.into())
    }
}

#[uniffi::export]
impl MurmurEngine {
    /// Attaches a photo to a session, optionally to a specific captured item.
    /// The shell writes the bytes FIRST (Plan 11 D4 write order), then calls
    /// this with the resulting filename. `captured_at` is `None` → core
    /// stamps `now()`. Errors: missing/tombstoned session or item -> `Photo`;
    /// an `item_id` not in `session_id` -> `Photo` (InvalidState); a poisoned
    /// lock or persistence failure -> `Photo`.
    pub fn add_photo(
        &self,
        session_id: String,
        item_id: Option<String>,
        filename: String,
        captured_at: Option<u64>,
    ) -> Result<PhotoRef, EngineError> {
        let store = self.store.lock().map_err(|_| Self::photo_err("store lock poisoned"))?;
        let p = store
            .add_photo(&session_id, item_id.as_deref(), &filename, captured_at)
            .map_err(|e| Self::photo_err(e.to_string()))?;
        Ok(photo_ref(&p))
    }

    /// Photos attached to a session, insertion order.
    pub fn list_photos(&self, session_id: String) -> Result<Vec<PhotoRef>, EngineError> {
        let store = self.store.lock().map_err(|_| Self::photo_err("store lock poisoned"))?;
        let photos = store
            .list_photos_for_session(&session_id)
            .map_err(|e| Self::photo_err(e.to_string()))?;
        Ok(photos.iter().map(photo_ref).collect())
    }

    /// Tombstones a photo's metadata row. The bytes are reclaimed by the
    /// shell's reconciling sweep (Plan 11 D4), not deleted here.
    pub fn remove_photo(&self, photo_id: String) -> Result<(), EngineError> {
        let store = self.store.lock().map_err(|_| Self::photo_err("store lock poisoned"))?;
        store.remove_photo(&photo_id).map_err(|e| Self::photo_err(e.to_string()))
    }

    /// Core's entire file contract (Plan 11 D4): every live photo filename,
    /// across all sessions. The shell sweep deletes any file on disk not in
    /// this set.
    pub fn list_live_photo_filenames(&self) -> Result<Vec<String>, EngineError> {
        let store = self.store.lock().map_err(|_| Self::photo_err("store lock poisoned"))?;
        store.list_live_photo_filenames().map_err(|e| Self::photo_err(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{MurmurEngine, Providers};
    use harness::{HarnessError, Memory, MemoryStore, MockProvider};
    use std::sync::{Arc, Mutex as StdMutex};

    // Inline SpyStore + literal Providers — the exact vocabulary.rs:84-108
    // pattern (no `NullStore`/`providers()` helpers exist in this crate).
    struct SpyStore {
        saved: StdMutex<Vec<Memory>>,
    }
    impl MemoryStore for SpyStore {
        fn load(&self) -> Result<Memory, HarnessError> {
            Ok(Memory::default())
        }
        fn save(&self, m: &Memory) -> Result<(), HarnessError> {
            self.saved.lock().unwrap().push(m.clone());
            Ok(())
        }
    }
    // Takes an already-opened Store so tests can start a session on it first,
    // then hand ownership to the engine (with_providers consumes the Store).
    fn engine_with(store: murmur_core::Store) -> Arc<MurmurEngine> {
        MurmurEngine::with_providers(
            store,
            Memory::default(),
            Arc::new(SpyStore { saved: StdMutex::new(Vec::new()) }),
            Providers {
                live: Arc::new(MockProvider::new(vec![])),
                processing: Arc::new(MockProvider::new(vec![])),
                reflection: Arc::new(MockProvider::new(vec![])),
            },
        )
    }

    #[tokio::test]
    async fn add_list_remove_round_trip() {
        let store = murmur_core::Store::open_in_memory("device-a").unwrap();
        let sid = store.start_session(None).unwrap().id;
        let e = engine_with(store);
        let p = e.add_photo(sid.clone(), None, "a.jpg".into(), Some(42)).unwrap();
        assert_eq!(p.session_id, sid);
        assert_eq!(p.item_id, None);
        assert_eq!(p.filename, "a.jpg");
        assert_eq!(p.captured_at, 42);
        assert_eq!(e.list_photos(sid.clone()).unwrap(), vec![p.clone()]);
        e.remove_photo(p.id.clone()).unwrap();
        assert!(e.list_photos(sid).unwrap().is_empty());
    }

    #[tokio::test]
    async fn add_photo_to_missing_session_is_a_photo_error() {
        let e = engine_with(murmur_core::Store::open_in_memory("device-a").unwrap());
        assert!(matches!(e.add_photo("nope".into(), None, "z.jpg".into(), None), Err(EngineError::Photo(_))));
    }

    #[tokio::test]
    async fn list_live_photo_filenames_feeds_the_sweep() {
        let store = murmur_core::Store::open_in_memory("device-a").unwrap();
        let sid = store.start_session(None).unwrap().id;
        let e = engine_with(store);
        e.add_photo(sid.clone(), None, "keep.jpg".into(), None).unwrap();
        let gone = e.add_photo(sid.clone(), None, "drop.jpg".into(), None).unwrap();
        e.remove_photo(gone.id).unwrap();
        assert_eq!(e.list_live_photo_filenames().unwrap(), vec!["keep.jpg".to_string()]);
    }
}
