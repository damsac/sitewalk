//! Item CRUD across UniFFI (Plan 16): `update_item` / `add_item` /
//! `remove_item`. Engine-keyed by `session_id` (NOT `WalkSession`-scoped —
//! the walk is over at review time; the photos.rs / build_document
//! precedent) and **`Processed`-gated** (D3-16, `build_document`'s exact
//! rule): a session you can build a document for is precisely a session you
//! can edit. The status check and the mutation happen under ONE store lock
//! with no intervening await, so there is no TOCTOU window (the
//! `add_item_if_status` discipline).
//!
//! NO correction-learning side effect lands here (Rev 2): neither
//! `record_correction` nor the vocab suggest-card — both are deferred to
//! Plan 17 as one content-carrying signal (a bare counter could snap an
//! earlier reflection over an activity summary that still holds the
//! mis-heard term). A test below pins the counter untouched across every
//! edit path.
//!
//! Every method is panic-free across FFI (Plan 07 CANON): a poisoned lock or
//! a store/validation error surfaces as `EngineError::Item`, never a panic.

use murmur_core::{SessionStatus, Store, VALID_ITEM_KINDS};

use crate::convert;
use crate::engine::{EngineError, MurmurEngine};
use crate::events::BoardItem;

impl MurmurEngine {
    fn item_err(msg: impl Into<String>) -> EngineError {
        EngineError::Item(msg.into())
    }

    /// D3-16: mutations are allowed on `Processed` sessions only — the
    /// review surface. `Recording`/`AwaitingProcessing` edits race the
    /// authoritative sweep; `Failed` edits race a retry's re-process. Takes
    /// the already-locked store so the check shares the mutation's lock
    /// (no check-then-write window).
    fn require_processed(store: &Store, session_id: &str) -> Result<(), EngineError> {
        let session = store.get_session(session_id).map_err(|e| Self::item_err(e.to_string()))?;
        if session.status != SessionStatus::Processed {
            return Err(Self::item_err(format!(
                "cannot edit items on a {} session (edits are review-only)",
                session.status.as_str()
            )));
        }
        Ok(())
    }

    /// D5-16: `kind` must be in the ONE shared allowlist (R6: reject
    /// unknowns, never store them) — the same const the agent `AddItemTool`
    /// validates and advertises.
    fn require_valid_kind(kind: &str) -> Result<(), EngineError> {
        if !VALID_ITEM_KINDS.contains(&kind) {
            return Err(Self::item_err(format!(
                "invalid kind '{kind}'; must be one of: {}",
                VALID_ITEM_KINDS.join(", ")
            )));
        }
        Ok(())
    }

    /// The honest echo (Rev 2): the returned `BoardItem` carries the item's
    /// TRUE live `photo_count`, read under the same lock — never an empty
    /// counts map (a 0-count echo would invite the Swift layer to patch
    /// local state and vanish a real photo badge).
    fn echo_board_item(
        store: &Store,
        session_id: &str,
        item: &murmur_core::CapturedItem,
    ) -> Result<BoardItem, EngineError> {
        let counts = store
            .count_live_photos_by_item_for_session(session_id)
            .map_err(|e| Self::item_err(e.to_string()))?;
        Ok(convert::board_item(item, &counts))
    }
}

#[uniffi::export]
impl MurmurEngine {
    /// Partial update of an item's editable fields (text / kind / right —
    /// keeper D-#1). A `None` field is left unchanged; an all-`None` call is
    /// a harmless no-op that still bumps the row's `updated_at`. `right` is
    /// the free-form quantity/unit string ("3 CU YD") — NOT price — and any
    /// value including `""` is accepted (D5-16). Returns the fresh
    /// `BoardItem` echo (with its honest `photo_count`) — an OPTIMISTIC
    /// display aid only: the notes/edit screen must re-read from the engine
    /// after any mutation (keeper D-#7), never rebuild state from this echo.
    pub fn update_item(
        &self,
        session_id: String,
        item_id: String,
        text: Option<String>,
        kind: Option<String>,
        right: Option<String>,
    ) -> Result<BoardItem, EngineError> {
        let store = self.store.lock().map_err(|_| Self::item_err("store lock poisoned"))?;
        Self::require_processed(&store, &session_id)?;
        if let Some(t) = &text {
            if t.trim().is_empty() {
                return Err(Self::item_err("item text is empty"));
            }
        }
        if let Some(k) = &kind {
            Self::require_valid_kind(k)?;
        }
        let updated = store
            .update_item(&item_id, text.as_deref(), kind.as_deref(), right.as_deref())
            .map_err(|e| Self::item_err(e.to_string()))?;
        Self::echo_board_item(&store, &session_id, &updated)
    }

    /// Adds a manual line at review time. `source = Manual` (survives any
    /// future reprocess) and a fresh UUIDv7 id, which sorts AFTER every
    /// existing item — the new line is last in every list and every rebuilt
    /// document (D4-16/WE-D). `right` may be `""` ("no quantity").
    pub fn add_item(
        &self,
        session_id: String,
        kind: String,
        text: String,
        right: String,
    ) -> Result<BoardItem, EngineError> {
        let store = self.store.lock().map_err(|_| Self::item_err("store lock poisoned"))?;
        Self::require_processed(&store, &session_id)?;
        Self::require_valid_kind(&kind)?;
        if text.trim().is_empty() {
            return Err(Self::item_err("item text is empty"));
        }
        let item = store
            .add_item(&session_id, &kind, &text)
            .map_err(|e| Self::item_err(e.to_string()))?;
        // Set `right` through the ONE existing write path (same locked
        // scope; it bumps updated_at once more, harmless) rather than
        // growing a second insert signature.
        let item = store
            .update_item(&item.id, None, None, Some(&right))
            .map_err(|e| Self::item_err(e.to_string()))?;
        Self::echo_board_item(&store, &session_id, &item)
    }

    /// Retraction, distinct from `done` (keeper D-#4): tombstones the item
    /// (photos demoted to session-level, Plan 11 D3), dropping it from
    /// `list_items_for_session`, `list_open_todos`, AND every rebuilt
    /// document — where a `done` item stays in the document and only leaves
    /// the open-todos glance (WE-C pins the contrast). A second remove of
    /// the same id errors (the store's tombstone `NotFound`).
    pub fn remove_item(&self, session_id: String, item_id: String) -> Result<(), EngineError> {
        let store = self.store.lock().map_err(|_| Self::item_err("store lock poisoned"))?;
        Self::require_processed(&store, &session_id)?;
        store.delete_item(&item_id).map_err(|e| Self::item_err(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};

    use harness::{
        CompletionResponse, ContentBlock, HarnessError, Memory, MemoryStore, MockProvider,
        StopReason, Usage,
    };

    use crate::engine::Providers;

    use super::*;

    // Inline SpyStore + literal Providers — the photos.rs/vocabulary.rs
    // pattern (no shared helpers exist in this crate by design).
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

    fn tool_use(name: &str, input: serde_json::Value) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::ToolUse { id: "tu".into(), name: name.into(), input }],
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 10, output_tokens: 5 },
        }
    }

    fn end_turn(text: &str) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::Text { text: text.into() }],
            stop_reason: StopReason::EndTurn,
            usage: Usage { input_tokens: 10, output_tokens: 5 },
        }
    }

    fn engine_with(store: murmur_core::Store, processing: Vec<CompletionResponse>) -> Arc<MurmurEngine> {
        MurmurEngine::with_providers(
            store,
            Memory::default(),
            Arc::new(SpyStore { saved: StdMutex::new(Vec::new()) }),
            Providers {
                live: Arc::new(MockProvider::new(vec![])),
                processing: Arc::new(MockProvider::new(processing)),
                reflection: Arc::new(MockProvider::new(vec![])),
            },
        )
    }

    /// Drives `begin_walk -> append_transcript -> finish()` (the
    /// document_build.rs::processed_landscape_session shape) with one
    /// scripted `add_item` per fixture row, leaving the session `Processed`
    /// with exactly those authoritative items, in order.
    async fn processed_session_with_items(
        items: &[(&str, &str)],
    ) -> (Arc<MurmurEngine>, String) {
        let store = murmur_core::Store::open_in_memory("device-a").unwrap();
        let mut responses: Vec<CompletionResponse> = items
            .iter()
            .map(|(kind, text)| {
                tool_use("add_item", serde_json::json!({"kind": kind, "text": text}))
            })
            .collect();
        responses.push(end_turn("done"));
        responses.push(tool_use("write_notes", serde_json::json!({"summary": "Walked the site."})));
        let engine = engine_with(store, responses);
        let session = engine.clone().begin_walk(None, "landscape".into()).unwrap();
        session.clone().append_transcript("site walk".into());
        let sid = session.session_id();
        let _notes = session.finish().await;
        (engine, sid)
    }

    /// The session's live item ids, insertion order (UUIDv7 = creation order).
    fn item_ids(engine: &MurmurEngine, sid: &str) -> Vec<String> {
        engine
            .store
            .lock()
            .unwrap()
            .list_items_for_session(sid)
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect()
    }

    fn open_todo_ids(engine: &MurmurEngine) -> Vec<String> {
        engine
            .store
            .lock()
            .unwrap()
            .list_open_todos()
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect()
    }

    // ---- Task 3: status gate (D3-16) ------------------------------------

    #[tokio::test]
    async fn mutations_are_rejected_on_a_recording_session() {
        let engine = engine_with(murmur_core::Store::open_in_memory("device-a").unwrap(), vec![]);
        let session = engine.clone().begin_walk(None, "landscape".into()).unwrap();
        let sid = session.session_id();
        assert!(matches!(
            engine.update_item(sid.clone(), "any".into(), Some("x".into()), None, None),
            Err(EngineError::Item(_))
        ));
        assert!(matches!(
            engine.add_item(sid.clone(), "todo".into(), "x".into(), "".into()),
            Err(EngineError::Item(_))
        ));
        assert!(matches!(engine.remove_item(sid, "any".into()), Err(EngineError::Item(_))));
    }

    #[tokio::test]
    async fn mutations_are_rejected_on_an_awaiting_processing_session() {
        let store = murmur_core::Store::open_in_memory("device-a").unwrap();
        let sid = store.start_session_with_template(None, "landscape").unwrap().id;
        let item = store.add_item(&sid, "todo", "order lumber").unwrap();
        store.end_and_record_session(&sid).unwrap(); // Recording -> AwaitingProcessing
        let engine = engine_with(store, vec![]);
        assert!(matches!(
            engine.update_item(sid.clone(), item.id.clone(), Some("x".into()), None, None),
            Err(EngineError::Item(_))
        ));
        assert!(matches!(
            engine.add_item(sid.clone(), "todo".into(), "x".into(), "".into()),
            Err(EngineError::Item(_))
        ));
        assert!(matches!(engine.remove_item(sid, item.id), Err(EngineError::Item(_))));
        // Nothing was written or tombstoned while gated.
    }

    #[tokio::test]
    async fn mutations_succeed_on_a_processed_session() {
        let (engine, sid) = processed_session_with_items(&[("todo", "Power edger")]).await;
        let id = item_ids(&engine, &sid).remove(0);
        engine.update_item(sid.clone(), id.clone(), Some("Mower".into()), None, None).unwrap();
        engine.add_item(sid.clone(), "safety".into(), "cracked walkway".into(), "".into()).unwrap();
        engine.remove_item(sid, id).unwrap();
    }

    // ---- Task 3: validation (D5-16) --------------------------------------

    #[tokio::test]
    async fn update_item_validates_and_returns_the_fresh_board_item() {
        let (engine, sid) = processed_session_with_items(&[("todo", "Power edger")]).await;
        let id = item_ids(&engine, &sid).remove(0);

        assert!(matches!(
            engine.update_item(sid.clone(), id.clone(), Some("   ".into()), None, None),
            Err(EngineError::Item(_))
        ));
        assert!(matches!(
            engine.update_item(sid.clone(), id.clone(), None, Some("bogus".into()), None),
            Err(EngineError::Item(_))
        ));
        assert!(matches!(
            engine.update_item(sid.clone(), "no-such-item".into(), Some("x".into()), None, None),
            Err(EngineError::Item(_))
        ));

        let board = engine
            .update_item(sid, id.clone(), Some("Mower".into()), Some("part".into()), Some("× 1".into()))
            .unwrap();
        assert_eq!(board.id, id);
        assert_eq!(board.text, "Mower");
        assert_eq!(board.kind, "part");
        assert_eq!(board.right, "× 1");
    }

    #[tokio::test]
    async fn add_item_validates_kind_and_text() {
        let (engine, sid) = processed_session_with_items(&[]).await;
        assert!(matches!(
            engine.add_item(sid.clone(), "bogus".into(), "x".into(), "".into()),
            Err(EngineError::Item(_))
        ));
        assert!(matches!(
            engine.add_item(sid.clone(), "todo".into(), "  ".into(), "".into()),
            Err(EngineError::Item(_))
        ));
        assert!(item_ids(&engine, &sid).is_empty(), "nothing written on rejection");
    }

    #[tokio::test]
    async fn update_item_right_projects_through_the_board_item() {
        let (engine, sid) = processed_session_with_items(&[("part", "bark mulch")]).await;
        let id = item_ids(&engine, &sid).remove(0);
        let board =
            engine.update_item(sid, id, None, None, Some("3 CU YD".into())).unwrap();
        assert_eq!(board.right, "3 CU YD", "board_item now reads item.right");
    }

    // ---- Task 3: no correction wiring (Rev 2 negative pin) ---------------

    #[tokio::test]
    async fn no_edit_path_touches_the_correction_counter() {
        let (engine, sid) =
            processed_session_with_items(&[("todo", "Power edger"), ("part", "bark mulch")]).await;
        let ids = item_ids(&engine, &sid);
        engine.update_item(sid.clone(), ids[0].clone(), Some("Mower".into()), None, None).unwrap();
        engine.update_item(sid.clone(), ids[0].clone(), None, Some("part".into()), None).unwrap();
        engine.update_item(sid.clone(), ids[1].clone(), None, None, Some("3 CU YD".into())).unwrap();
        engine.add_item(sid.clone(), "safety".into(), "cracked walkway".into(), "".into()).unwrap();
        engine.remove_item(sid, ids[1].clone()).unwrap();
        let corrections = engine
            .store
            .lock()
            .unwrap()
            .reflection_signals()
            .unwrap()
            .corrections_since_reflection;
        assert_eq!(
            corrections, 0,
            "Plan 16 wires NO record_correction — the content-carrying signal is Plan 17's \
             (a bare counter could snap a reflection that re-learns the mis-heard term)"
        );
    }

    // ---- Task 3: add appends / remove tombstones at the FFI layer --------

    #[tokio::test]
    async fn add_item_appends_and_remove_item_tombstones() {
        let (engine, sid) = processed_session_with_items(&[("todo", "call supplier")]).await;
        let first = item_ids(&engine, &sid).remove(0);

        let added = engine
            .add_item(sid.clone(), "safety".into(), "cracked walkway".into(), "× 2".into())
            .unwrap();
        assert_eq!(added.text, "cracked walkway");
        assert_eq!(added.right, "× 2", "a line can be added with a quantity in one call");
        assert_eq!(
            item_ids(&engine, &sid),
            vec![first.clone(), added.id.clone()],
            "the fresh UUIDv7 sorts after every existing item — append"
        );
        {
            let store = engine.store.lock().unwrap();
            let item = store.get_item(&added.id).unwrap();
            assert_eq!(item.source, murmur_core::ItemSource::Manual, "survives reprocess");
        }

        engine.remove_item(sid.clone(), added.id.clone()).unwrap();
        assert_eq!(item_ids(&engine, &sid), vec![first]);
        assert!(
            matches!(engine.remove_item(sid, added.id), Err(EngineError::Item(_))),
            "a second remove of the tombstoned id errors"
        );
    }

    // ---- Task 3: honest photo_count echo (Rev 2) --------------------------

    #[tokio::test]
    async fn update_item_echo_carries_the_honest_photo_count() {
        let (engine, sid) = processed_session_with_items(&[("todo", "Power edger")]).await;
        let id = item_ids(&engine, &sid).remove(0);
        engine.add_photo(sid.clone(), Some(id.clone()), "a.jpg".into(), None).unwrap();
        engine.add_photo(sid.clone(), Some(id.clone()), "b.jpg".into(), None).unwrap();
        let board = engine.update_item(sid, id, Some("Mower".into()), None, None).unwrap();
        assert_eq!(
            board.photo_count, 2,
            "the echo reads count_live_photos_by_item_for_session under the same lock — \
             never an empty counts map"
        );
    }
}
