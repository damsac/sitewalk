//! Vocabulary CRUD across UniFFI (Plan 10). The write half of the vocabulary →
//! STT biasing loop: these mutate the `Memory` "vocabulary" section that
//! `begin_walk`'s `collect_bias_terms` reads. Lock-then-save discipline mirrors
//! `harness::UpdateMemoryTool` (mutate under the lock, clamp the global cap,
//! snapshot, release, persist). Panic-free across FFI (Plan 07 CANON).

use harness::{FactSource, VocabAdd, DEFAULT_WORD_CAP};

use crate::engine::{EngineError, MurmurEngine};

impl MurmurEngine {
    fn memory_err(msg: impl Into<String>) -> EngineError {
        EngineError::Memory(msg.into())
    }
}

#[uniffi::export]
impl MurmurEngine {
    /// The user's vocabulary terms, insertion order. Read-only — no lock held
    /// across FFI beyond the clone.
    pub fn list_vocabulary(&self) -> Result<Vec<String>, EngineError> {
        let mem = self.memory.lock().map_err(|_| Self::memory_err("memory lock poisoned"))?;
        Ok(mem.vocabulary_terms().into_iter().map(str::to_string).collect())
    }

    /// Add one user vocabulary term (`FactSource::Stated`, D3). Idempotent
    /// (case-insensitive). Errors: `Full` at 100 terms, `Empty` for blank input,
    /// a poisoned lock, or a persistence failure. Returns the resulting list so
    /// the editor updates in one round-trip.
    pub fn add_vocabulary_term(&self, term: String) -> Result<Vec<String>, EngineError> {
        let snapshot = {
            let mut mem = self.memory.lock().map_err(|_| Self::memory_err("memory lock poisoned"))?;
            let now = now_secs();
            match mem.add_vocabulary_term(&term, now, FactSource::Stated) {
                VocabAdd::Added | VocabAdd::Duplicate => {}
                VocabAdd::Full => {
                    return Err(Self::memory_err(format!(
                        "vocabulary is full ({} terms); remove one first",
                        harness::MAX_VOCABULARY_TERMS
                    )))
                }
                VocabAdd::Empty => return Err(Self::memory_err("term is empty")),
                VocabAdd::TooLong => {
                    return Err(Self::memory_err(format!(
                        "term is too long (max {} words)",
                        harness::MAX_VOCABULARY_TERM_WORDS
                    )))
                }
            }
            mem.clamp_to_cap(DEFAULT_WORD_CAP); // global 500-word invariant, like UpdateMemoryTool
            mem.clone()
        };
        self.memory_store.save(&snapshot).map_err(|e| EngineError::Store(e.to_string()))?;
        Ok(snapshot.vocabulary_terms().into_iter().map(str::to_string).collect())
    }

    /// Remove one vocabulary term (case-insensitive). Returns the resulting list.
    /// Removing a term that isn't present is not an error (idempotent).
    pub fn remove_vocabulary_term(&self, term: String) -> Result<Vec<String>, EngineError> {
        let snapshot = {
            let mut mem = self.memory.lock().map_err(|_| Self::memory_err("memory lock poisoned"))?;
            mem.remove_vocabulary_term(&term);
            mem.clone()
        };
        self.memory_store.save(&snapshot).map_err(|e| EngineError::Store(e.to_string()))?;
        Ok(snapshot.vocabulary_terms().into_iter().map(str::to_string).collect())
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{MurmurEngine, Providers};
    use harness::{HarnessError, Memory, MemoryStore, MockProvider};
    use std::sync::{Arc, Mutex as StdMutex};

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
    fn engine(store: Arc<SpyStore>) -> Arc<MurmurEngine> {
        let s = murmur_core::Store::open_in_memory("device-a").unwrap();
        MurmurEngine::with_providers(
            s,
            Memory::default(),
            store,
            Providers {
                live: Arc::new(MockProvider::new(vec![])),
                processing: Arc::new(MockProvider::new(vec![])),
                reflection: Arc::new(MockProvider::new(vec![])),
            },
        )
    }

    #[tokio::test]
    async fn add_list_remove_round_trip_and_persist() {
        let store = Arc::new(SpyStore { saved: StdMutex::new(Vec::new()) });
        let e = engine(store.clone());
        assert_eq!(e.add_vocabulary_term("french drain".into()).unwrap(), vec!["french drain"]);
        assert_eq!(e.list_vocabulary().unwrap(), vec!["french drain"]);
        // persisted: the last save carries the term
        assert!(store.saved.lock().unwrap().last().unwrap().vocabulary_terms().contains(&"french drain"));
        assert!(e.remove_vocabulary_term("French Drain".into()).unwrap().is_empty(), "case-insensitive remove");
    }

    #[tokio::test]
    async fn add_is_idempotent_and_full_is_an_error() {
        let store = Arc::new(SpyStore { saved: StdMutex::new(Vec::new()) });
        let e = engine(store);
        e.add_vocabulary_term("term".into()).unwrap();
        assert_eq!(e.add_vocabulary_term("TERM".into()).unwrap(), vec!["term"], "duplicate is Ok, not an error");
        // fill to the cap, then the next add throws
        for i in 0..harness::MAX_VOCABULARY_TERMS {
            let _ = e.add_vocabulary_term(format!("t{i}"));
        }
        assert!(matches!(e.add_vocabulary_term("overflow".into()), Err(EngineError::Memory(_))));
        assert!(matches!(e.add_vocabulary_term("   ".into()), Err(EngineError::Memory(_))), "empty is an error");
    }

    #[test]
    fn read_side_cap_matches_the_write_side_constant() {
        // D2: the mirrored consts must agree across the crate boundary.
        assert_eq!(harness::MAX_VOCABULARY_TERMS, stt::SttConfig::default().max_bias_terms);
    }
}
