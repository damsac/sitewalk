//! Plan 20 Half A (D2/D3): the board walk-log projection across FFI.
//! `MurmurEngine::list_sessions()` — a lightweight, transcript-free listing
//! of finished walks (Recording + tombstoned rows excluded in core's
//! `Store::list_walk_summaries`), reverse-chronological. The reopen READ
//! (`load_notes`) lives in `session.rs` next to the shared reconstruction
//! funnel. This file is deliberately disjoint from Plan 19's document/schema
//! surfaces (Conflict Note).

use crate::engine::{EngineError, MurmurEngine};

/// A finished walk's status at the FFI boundary (D3). Core's
/// `AwaitingProcessing` maps to `Processing` (Swift's `.processing`);
/// `Recording` never crosses — `list_walk_summaries` excludes it.
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkStatus {
    Processing,
    Processed,
    Failed,
}

/// One board walk-log row (D2): a lightweight projection — NO transcript.
/// `queued = status != Processed` (the same predicate `NotesPayload.queued`
/// carries); `has_document` = a live `document` artifact exists (a
/// built-and-kept walk).
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct WalkSummary {
    pub id: String,
    /// `doc_kind_for_template(template)` — advisory, for row labeling.
    pub doc_kind: String,
    pub status: WalkStatus,
    /// `session.summary`, `""` when the session never reached Processed.
    pub summary: String,
    /// Epoch-ms, the same clock `walkStart` uses.
    pub started_at: u64,
    /// Live items only.
    pub item_count: u32,
    pub has_document: bool,
    pub queued: bool,
}

fn walk_status(status: murmur_core::SessionStatus) -> WalkStatus {
    match status {
        murmur_core::SessionStatus::Processed => WalkStatus::Processed,
        murmur_core::SessionStatus::Failed => WalkStatus::Failed,
        // Recording is filtered out by the core query (D3); defensive map to
        // Processing rather than a panic across FFI if that ever regressed.
        murmur_core::SessionStatus::AwaitingProcessing | murmur_core::SessionStatus::Recording => {
            WalkStatus::Processing
        }
    }
}

pub(crate) fn walk_summary(core: &murmur_core::WalkSummary) -> WalkSummary {
    WalkSummary {
        id: core.id.clone(),
        doc_kind: murmur_core::doc_kind_for_template(core.template.as_deref()).to_string(),
        status: walk_status(core.status),
        summary: core.summary.clone().unwrap_or_default(),
        started_at: core.started_at,
        item_count: core.item_count.min(u32::MAX as u64) as u32,
        has_document: core.has_document,
        queued: core.status != murmur_core::SessionStatus::Processed,
    }
}

#[uniffi::export]
impl MurmurEngine {
    /// The board walk log (D2/D3): every reopenable walk, newest first —
    /// never a transcript (Plan 04 lesson), never a Recording or tombstoned
    /// row. Read-only: safe at app-open alongside the sweeps (R4).
    pub fn list_sessions(&self) -> Result<Vec<WalkSummary>, EngineError> {
        let store = self
            .store
            .lock()
            .map_err(|_| EngineError::Session("store lock poisoned".into()))?;
        let walks = store
            .list_walk_summaries()
            .map_err(|e| EngineError::Session(e.to_string()))?;
        Ok(walks.iter().map(walk_summary).collect())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use harness::{HarnessError, Memory, MemoryStore, MockProvider};
    use murmur_core::Store;

    use crate::engine::Providers;

    use super::*;

    struct NullMemoryStore;
    impl MemoryStore for NullMemoryStore {
        fn load(&self) -> Result<Memory, HarnessError> {
            Ok(Memory::default())
        }
        fn save(&self, _m: &Memory) -> Result<(), HarnessError> {
            Ok(())
        }
    }

    fn engine_over(store: Store) -> Arc<MurmurEngine> {
        MurmurEngine::with_providers(
            store,
            Memory::default(),
            Arc::new(NullMemoryStore),
            Providers {
                live: Arc::new(MockProvider::new(vec![])),
                processing: Arc::new(MockProvider::new(vec![])),
                reflection: Arc::new(MockProvider::new(vec![])),
            },
        )
    }

    /// WE-C at the FFI boundary: projection fields, status/queued mapping,
    /// and the Recording/tombstoned filters.
    #[tokio::test]
    async fn list_sessions_projects_and_gates() {
        let store = Store::open_in_memory("device-a").unwrap();
        // P: Processed, 2 live items, 1 live document artifact.
        let p = store.start_session_with_template(None, "landscape").unwrap();
        store.add_item(&p.id, "todo", "one").unwrap();
        store.add_item(&p.id, "note", "two").unwrap();
        store.end_session(&p.id).unwrap();
        store.mark_session_processed(&p.id, "Estimate walk.").unwrap();
        store.add_artifact(&p.id, "document", "estimate", "{}").unwrap();
        // F: Failed, 3 items, no document.
        let f = store.start_session_with_template(None, "inspection").unwrap();
        for t in ["a", "b", "c"] {
            store.add_item(&f.id, "todo", t).unwrap();
        }
        store.end_session(&f.id).unwrap();
        store.mark_session_failed(&f.id).unwrap();
        // A: AwaitingProcessing -> maps to Processing, queued.
        let a = store.start_session_with_template(None, "property").unwrap();
        store.end_session(&a.id).unwrap();
        // R: Recording — excluded.
        store.start_session_with_template(None, "landscape").unwrap();
        // D: tombstoned — excluded.
        let d = store.start_session_with_template(None, "landscape").unwrap();
        store.end_session(&d.id).unwrap();
        store.delete_session(&d.id).unwrap();

        let engine = engine_over(store);
        let walks = engine.list_sessions().unwrap();
        let ids: Vec<&str> = walks.iter().map(|w| w.id.as_str()).collect();
        assert!(!ids.iter().any(|id| *id == d.id), "tombstoned excluded");
        assert_eq!(walks.len(), 3, "P, F, A listed; R and D excluded");

        let wp = walks.iter().find(|w| w.id == p.id).unwrap();
        assert_eq!(wp.status, WalkStatus::Processed);
        assert!(!wp.queued);
        assert_eq!(wp.summary, "Estimate walk.");
        assert_eq!(wp.item_count, 2);
        assert!(wp.has_document);
        assert_eq!(wp.doc_kind, "estimate", "landscape template -> estimate kind");

        let wf = walks.iter().find(|w| w.id == f.id).unwrap();
        assert_eq!(wf.status, WalkStatus::Failed);
        assert!(wf.queued, "Failed -> queued (gating predicate)");
        assert_eq!(wf.summary, "", "never-Processed summary reads back empty");
        assert_eq!(wf.item_count, 3);
        assert!(!wf.has_document);

        let wa = walks.iter().find(|w| w.id == a.id).unwrap();
        assert_eq!(wa.status, WalkStatus::Processing, "AwaitingProcessing -> Processing");
        assert!(wa.queued);
    }

    /// D2 (Plan 04 lesson), compile-enforced at the FFI boundary too: the
    /// record has no transcript field — exhaustive destructuring fails to
    /// compile if one is ever added.
    #[tokio::test]
    async fn list_sessions_carries_no_transcript() {
        let store = Store::open_in_memory("device-a").unwrap();
        let s = store.start_session(None).unwrap();
        store.append_transcript(&s.id, &"x".repeat(50_000)).unwrap();
        store.end_session(&s.id).unwrap();
        let engine = engine_over(store);
        let walks = engine.list_sessions().unwrap();
        let WalkSummary {
            id: _,
            doc_kind: _,
            status: _,
            summary: _,
            started_at: _,
            item_count: _,
            has_document: _,
            queued: _,
        } = walks[0].clone();
    }
}
