//! `MurmurEngine::build_document` (Plan 13 Stage 1, additive): the on-demand
//! document build. Engine-keyed (D1), NOT `WalkSession`-scoped — `finish()`
//! nils out its `WalkSession` handle, so a later tap (possibly after
//! relaunch/from history) has no session object. Photos already solved this
//! (`add_photo` is engine-keyed); this follows the same precedent.
//!
//! Stage 1 is purely additive: `process()`/`finish()` are untouched, so a
//! `finish()`ed session in this stage already has a `document`-kind artifact
//! from the OLD forced phase-B call. `build_document` mints and appends a
//! NEW one (D7: burn-per-tap) and reads back EXACTLY the artifact id the
//! builder just wrote — never `latest_document_artifact`, since multiple
//! documents can now coexist for one session.

use murmur_core::DocumentBuilder;

use crate::convert;
use crate::document::DocumentPayload;
use crate::engine::{EngineError, MurmurEngine};

#[uniffi::export(async_runtime = "tokio")]
impl MurmurEngine {
    pub async fn build_document(
        &self,
        session_id: String,
        kind: String,
    ) -> Result<DocumentPayload, EngineError> {
        let builder = DocumentBuilder::new(
            self.providers.processing.clone(),
            self.store.clone(),
            self.memory.clone(),
            self.memory_store.clone(),
        );
        let outcome = builder
            .build(&session_id, &kind)
            .await
            .map_err(|e| EngineError::Document(e.to_string()))?;
        let artifact = {
            let store = self
                .store
                .lock()
                .map_err(|_| EngineError::Document("store lock poisoned".into()))?;
            store.get_artifact(&outcome.document_artifact_id)
        }
        .map_err(|e| EngineError::Document(e.to_string()))?;
        convert::document_payload(&artifact).map_err(|e| EngineError::Document(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use harness::{
        CompletionResponse, ContentBlock, HarnessError, Memory, MemoryStore, MockProvider,
        StopReason, Usage,
    };

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

    fn tool_use(name: &str, input: serde_json::Value) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::ToolUse { id: "tu".into(), name: name.into(), input }],
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 10, output_tokens: 5, ..Default::default() },
        }
    }

    fn end_turn(text: &str) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::Text { text: text.into() }],
            stop_reason: StopReason::EndTurn,
            usage: Usage { input_tokens: 10, output_tokens: 5, ..Default::default() },
        }
    }

    fn summary_response(text: &str) -> CompletionResponse {
        tool_use("write_notes", serde_json::json!({"summary": text}))
    }

    /// Drives a walk through `begin_walk` -> `append_transcript` ->
    /// `finish()` on `processing` responses `[add_item, end_turn, summary]`
    /// (Stage 2: `finish()` = notes only, no phase B), leaving the session
    /// `Processed` with exactly one authoritative item. Returns the engine +
    /// session id.
    async fn processed_landscape_session(
        extra_processing_responses: Vec<CompletionResponse>,
    ) -> (std::sync::Arc<MurmurEngine>, String) {
        let store = murmur_core::Store::open_in_memory("device-a").unwrap();
        let mut responses = vec![
            tool_use("add_item", serde_json::json!({"kind": "todo", "text": "order lumber"})),
            end_turn("done"),
            summary_response("Lumber ordered."),
        ];
        responses.extend(extra_processing_responses);
        let engine = MurmurEngine::with_providers(
            store,
            Memory::default(),
            Arc::new(NullMemoryStore),
            Providers {
                live: Arc::new(MockProvider::new(vec![])),
                processing: Arc::new(MockProvider::new(responses)),
                reflection: Arc::new(MockProvider::new(vec![])),
            },
        );
        let session = engine.clone().begin_walk(None, "landscape".into()).unwrap();
        session.clone().append_transcript("order twelve two by tens for the deck".into());
        let sid = session.session_id();
        let _notes = session.finish().await; // Stage 2: NotesPayload, no document yet
        (engine, sid)
    }

    /// A work order comes back with no money and WITH its assignment block —
    /// the whole point of the document. The field keys validate (they are
    /// authored, so the response can pre-know them); the per-line write
    /// cannot, since item ids are minted at runtime (the Plan 12 C2 pattern),
    /// and dropping it is exactly the echo-validation working.
    #[tokio::test]
    async fn build_document_work_order_carries_its_assignment_and_no_money() {
        let (engine, sid) = processed_landscape_session(vec![tool_use(
            "compose_document",
            serde_json::json!({
                "fields": [
                    {"key": "crew", "value": "Jose, Michael"},
                    {"key": "access", "value": "Gate code 4412; park on the street."}
                ],
                "lines": [{"item_id": "placeholder", "detail": "Pick up from the yard first.",
                           "assignee": "Jose"}]
            }),
        )])
        .await;

        let payload = engine.build_document(sid.clone(), "work_order".into()).await.unwrap();
        assert_eq!(payload.doc_kind, "work_order");
        assert_eq!(payload.doc_number, 1, "a fresh mint for this build_document call");
        assert_eq!(payload.lines.len(), 1, "the one authoritative item survives finish()'s swap");
        assert_eq!(payload.lines[0].title, "order lumber");
        assert_eq!(payload.lines[0].amount_cents, None, "a work order carries no money");
        assert_eq!(
            payload.lines[0].assignee, None,
            "the placeholder id degrades — a name is never attached to a line we cannot match"
        );
        let crew = payload.fields.iter().find(|f| f.key == "crew").expect("crew field");
        assert_eq!(crew.value.as_deref(), Some("Jose, Michael"));
        assert!(!crew.is_gap);
        let access = payload.fields.iter().find(|f| f.key == "access").expect("access field");
        assert_eq!(access.value.as_deref(), Some("Gate code 4412; park on the street."));
        let safety = payload.fields.iter().find(|f| f.key == "safety").expect("safety field");
        assert!(safety.is_gap, "nothing was said about hazards — a truthful gap, not filler");
        assert!(!payload.queued);

        // Exactly one document artifact for the session: phase B is gone, so
        // this build_document call is the only writer.
        let store = engine.store.lock().unwrap();
        let docs: Vec<_> = store
            .list_artifacts_for_session(&sid)
            .unwrap()
            .into_iter()
            .filter(|a| a.kind == "document")
            .collect();
        assert_eq!(docs.len(), 1, "build_document is the only document writer now (phase B is gone)");
    }

    /// Empty-walk guard at the FFI seam (field fix, jefe-2026-07-24): a walk
    /// that captured nothing finishes Processed with zero items; tapping to
    /// build a work order must surface `EngineError::Document` and mint NO
    /// document artifact — not the blank "work order" ghost the operator hit.
    #[tokio::test]
    async fn build_document_on_empty_walk_errors_and_mints_no_artifact() {
        let store = murmur_core::Store::open_in_memory("device-a").unwrap();
        let engine = MurmurEngine::with_providers(
            store,
            Memory::default(),
            Arc::new(NullMemoryStore),
            Providers {
                live: Arc::new(MockProvider::new(vec![])),
                // A silent walk short-circuits to "(empty session)"; a spare
                // summary response is harmless if the path never calls out.
                processing: Arc::new(MockProvider::new(vec![summary_response("(empty session)")])),
                reflection: Arc::new(MockProvider::new(vec![])),
            },
        );
        // Finish WITHOUT any append_transcript -> Processed, zero items.
        let session = engine.clone().begin_walk(None, "landscape".into()).unwrap();
        let sid = session.session_id();
        let _notes = session.finish().await;

        let err = engine.build_document(sid.clone(), "work_order".into()).await.unwrap_err();
        assert!(matches!(err, EngineError::Document(_)), "empty walk -> Document error: {err:?}");

        let store = engine.store.lock().unwrap();
        let docs: Vec<_> = store
            .list_artifacts_for_session(&sid)
            .unwrap()
            .into_iter()
            .filter(|a| a.kind == "document")
            .collect();
        assert!(docs.is_empty(), "no ghost document artifact for an empty walk");
    }

    #[tokio::test]
    async fn build_document_illegal_kind_for_template_is_an_engine_error() {
        let (engine, sid) = processed_landscape_session(vec![]).await;
        // "condition" is a property-only kind, not legal for landscape.
        let err = engine.build_document(sid, "condition".into()).await.unwrap_err();
        assert!(matches!(err, EngineError::Document(_)));
    }

    #[tokio::test]
    async fn build_document_pricing_kind_feeds_the_real_item_id_and_lands_a_document() {
        // The pricing response can't pre-know the run's real minted item id
        // (Plan 12's C2 pattern), so it echoes a placeholder that will fail
        // echo-validation — proving the wiring (a price_items request was
        // made, fed the real id) without needing to pre-script it.
        let (engine, sid) = processed_landscape_session(vec![
            tool_use(
                "price_items",
                serde_json::json!({"prices": [{"item_id": "placeholder", "amount_cents": 28500}]}),
            ),
            // Pricing first, then the prose — an estimate now writes its scope
            // paragraph in a second, separate call.
            tool_use(
                "compose_document",
                serde_json::json!({"fields": [
                    {"key": "scope_summary", "value": "Deck lumber ordered and delivered."}
                ]}),
            ),
        ])
        .await;

        let payload = engine.build_document(sid, "estimate".into()).await.unwrap();
        assert_eq!(payload.doc_kind, "estimate");
        assert_eq!(payload.lines.len(), 1);
        assert_eq!(payload.lines[0].amount_cents, None, "placeholder id degrades, never crashes");
        let scope = payload.fields.iter().find(|f| f.key == "scope_summary").expect("scope field");
        assert_eq!(scope.value.as_deref(), Some("Deck lumber ordered and delivered."));
        assert!(!payload.queued, "the pricing call itself succeeded (a validation miss, not R7)");
    }
}
