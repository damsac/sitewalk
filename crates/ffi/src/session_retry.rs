//! `MurmurEngine::retry_failed_sessions` — the offline banner ("SAVED
//! OFFLINE — DOCUMENTS UNLOCK WHEN YOU RECONNECT") makes a promise that,
//! until this file, no code path kept: nothing ever re-drove a `Failed`
//! session back through the pipeline. `process_pending` only drains
//! `AwaitingProcessing` (its own no-retry pin stays exactly as-is); this is
//! the separate, explicit retry call the Swift app-open hook invokes.
//!
//! Engine-keyed (not `WalkSession`-scoped), same precedent as
//! `build_document`: a `Failed` session's `WalkSession` handle is long gone
//! by the time the user reopens the app.

use murmur_core::SessionProcessor;

use crate::engine::{EngineError, MurmurEngine};

#[uniffi::export(async_runtime = "tokio")]
impl MurmurEngine {
    /// Retries every `Failed` session once (oldest-first, capped — see
    /// `SessionProcessor::retry_failed_sessions`'s doc comment for the
    /// no-loop / cap-ordering rationale). Returns the count that reached
    /// `Processed` — thin on purpose; the host re-reads its own session
    /// list/history view rather than this call threading payloads back.
    ///
    /// Lock hygiene: `SessionProcessor::retry_failed_sessions` takes the
    /// store lock only for the short synchronous list-query inside its own
    /// loop body, never across a `process().await` — same discipline as
    /// `build_document`, which never holds the engine's `std::sync::Mutex`
    /// across an `.await` either.
    ///
    /// A still-Failed session (still offline, LLM still down) is not an
    /// error here — it's simply not counted in the returned total; only a
    /// poisoned lock or a store fault surfaces as `EngineError::Session`.
    pub async fn retry_failed_sessions(&self) -> Result<u32, EngineError> {
        let processor = SessionProcessor::new(
            self.providers.processing.clone(),
            self.store.clone(),
            self.memory.clone(),
            self.memory_store.clone(),
        );
        let results = processor
            .retry_failed_sessions()
            .await
            .map_err(|e| EngineError::Session(e.to_string()))?;
        Ok(results.iter().filter(|(_, r)| r.is_ok()).count() as u32)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use harness::{
        CompletionResponse, ContentBlock, HarnessError, Memory, MemoryStore, MockProvider,
        StopReason, Usage,
    };
    use murmur_core::{SessionStatus, Store};

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

    fn end_turn(text: &str) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::Text { text: text.into() }],
            stop_reason: StopReason::EndTurn,
            usage: Usage { input_tokens: 10, output_tokens: 5, ..Default::default() },
        }
    }

    fn summary_response(text: &str) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tu".into(),
                name: "write_notes".into(),
                input: serde_json::json!({"summary": text}),
            }],
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 10, output_tokens: 5, ..Default::default() },
        }
    }

    fn engine_with(
        store: Store,
        processing_responses: Vec<CompletionResponse>,
    ) -> Arc<MurmurEngine> {
        MurmurEngine::with_providers(
            store,
            Memory::default(),
            Arc::new(NullMemoryStore),
            Providers {
                live: Arc::new(MockProvider::new(vec![])),
                processing: Arc::new(MockProvider::new(processing_responses)),
                reflection: Arc::new(MockProvider::new(vec![])),
            },
        )
    }

    #[tokio::test]
    async fn retry_failed_sessions_recovers_and_counts_a_failed_session() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        store.append_transcript(&session.id, "we need lumber").unwrap();
        store.end_and_record_session(&session.id).unwrap();
        store.mark_session_failed(&session.id).unwrap();

        let engine =
            engine_with(store, vec![end_turn("nothing to extract"), summary_response("recovered")]);

        let recovered = engine.retry_failed_sessions().await.unwrap();
        assert_eq!(recovered, 1);

        let store = engine.store.lock().unwrap();
        assert_eq!(store.get_session(&session.id).unwrap().status, SessionStatus::Processed);
    }

    #[tokio::test]
    async fn retry_failed_sessions_zero_when_still_failing() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        store.append_transcript(&session.id, "we need lumber").unwrap();
        store.end_and_record_session(&session.id).unwrap();
        store.mark_session_failed(&session.id).unwrap();

        let engine = engine_with(
            store,
            vec![end_turn("nothing to extract"), end_turn("still no summary tool")],
        );

        let recovered = engine.retry_failed_sessions().await.unwrap();
        assert_eq!(recovered, 0, "a still-failing retry is not an EngineError, just not counted");

        let store = engine.store.lock().unwrap();
        assert_eq!(store.get_session(&session.id).unwrap().status, SessionStatus::Failed);
    }

    #[tokio::test]
    async fn retry_failed_sessions_zero_with_nothing_failed() {
        let store = Store::open_in_memory("device-a").unwrap();
        let engine = engine_with(store, vec![]);
        assert_eq!(engine.retry_failed_sessions().await.unwrap(), 0);
    }

    /// Zombie-recovery-for-free: a crash-orphaned `Recording` session, once
    /// swept by `sweep_zombie_sessions`, is a `Failed` session like any
    /// other — a retry picks it up in the same app-open.
    #[tokio::test]
    async fn retry_failed_sessions_recovers_a_zombie_swept_session() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        store.append_transcript(&session.id, "we need lumber").unwrap();
        // still Recording — simulates a crash mid-walk

        let engine =
            engine_with(store, vec![end_turn("nothing to extract"), summary_response("recovered")]);
        let swept = engine.sweep_zombie_sessions().unwrap();
        assert_eq!(swept, 1);

        let recovered = engine.retry_failed_sessions().await.unwrap();
        assert_eq!(recovered, 1);

        let store = engine.store.lock().unwrap();
        assert_eq!(store.get_session(&session.id).unwrap().status, SessionStatus::Processed);
    }

    /// The empty-transcript zombie case (a crash before any speech was
    /// captured) must hit the existing empty-session contract, not panic.
    #[tokio::test]
    async fn retry_failed_sessions_empty_zombie_session_never_panics() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap(); // no transcript

        let engine = engine_with(store, vec![]);
        let swept = engine.sweep_zombie_sessions().unwrap();
        assert_eq!(swept, 1);

        let recovered = engine.retry_failed_sessions().await.unwrap();
        assert_eq!(recovered, 1);

        let store = engine.store.lock().unwrap();
        let s = store.get_session(&session.id).unwrap();
        assert_eq!(s.status, SessionStatus::Processed);
        assert_eq!(s.summary.as_deref(), Some("(empty session)"));
    }
}
