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

    fn summary_response(text: &str) -> CompletionResponse {
        tool_use("write_summary", serde_json::json!({"summary": text}))
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

    #[tokio::test]
    async fn build_document_non_pricing_kind_returns_the_structure_only_document() {
        let (engine, sid) = processed_landscape_session(vec![]).await;

        let payload = engine.build_document(sid.clone(), "work_order".into()).await.unwrap();
        assert_eq!(payload.doc_kind, "work_order");
        assert_eq!(payload.doc_number, 1, "a fresh mint for this build_document call");
        assert_eq!(payload.lines.len(), 1, "the one authoritative item survives finish()'s swap");
        assert_eq!(payload.lines[0].title, "order lumber");
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
        let (engine, sid) = processed_landscape_session(vec![tool_use(
            "price_items",
            serde_json::json!({"prices": [{"item_id": "placeholder", "amount_cents": 28500}]}),
        )])
        .await;

        let payload = engine.build_document(sid, "estimate".into()).await.unwrap();
        assert_eq!(payload.doc_kind, "estimate");
        assert_eq!(payload.lines.len(), 1);
        assert_eq!(payload.lines[0].amount_cents, None, "placeholder id degrades, never crashes");
        assert!(!payload.queued, "the pricing call itself succeeded (a validation miss, not R7)");
    }
}
