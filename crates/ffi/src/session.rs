//! `WalkSession`: append/finish, the `LiveExtractor` actor, batched board
//! events (Plan 07 D3/D7).

use std::sync::{Arc, Mutex as StdMutex};

use harness::{LlmProvider, Memory, MemoryStore};
use murmur_core::{doc_kind_for_template, LiveExtractOutcome, LiveExtractor, SessionProcessor, Store};
use tokio::sync::Mutex as TokioMutex;

use crate::convert;
use crate::document::DocumentPayload;
use crate::engine::MurmurEngine;
use crate::events::{WalkEvent, WalkEventListener};

/// One recording session's bridge state. `finish` lands in Task 8.
#[derive(uniffi::Object)]
pub struct WalkSession {
    session_id: String,
    store: Arc<StdMutex<Store>>,
    /// The `tokio::sync::Mutex` doubles as the tick/finish serialization point
    /// (D3b/D7): `finish()` acquires it and holds it across `process().await`,
    /// so no live tick can interleave with end-of-session processing.
    extractor: Arc<TokioMutex<LiveExtractor>>,
    listener: StdMutex<Option<Arc<dyn WalkEventListener>>>,
    processing_provider: Arc<dyn LlmProvider>,
    memory: Arc<StdMutex<Memory>>,
    memory_store: Arc<dyn MemoryStore>,
    runtime_handle: tokio::runtime::Handle,
    template: Option<String>,
}

impl WalkSession {
    #[allow(clippy::too_many_arguments)]
    fn new(
        session_id: String,
        store: Arc<StdMutex<Store>>,
        extractor: LiveExtractor,
        processing_provider: Arc<dyn LlmProvider>,
        memory: Arc<StdMutex<Memory>>,
        memory_store: Arc<dyn MemoryStore>,
        runtime_handle: tokio::runtime::Handle,
        template: Option<String>,
    ) -> Arc<Self> {
        Arc::new(WalkSession {
            session_id,
            store,
            extractor: Arc::new(TokioMutex::new(extractor)),
            listener: StdMutex::new(None),
            processing_provider,
            memory,
            memory_store,
            runtime_handle,
            template,
        })
    }

    /// Re-queries the board and emits exactly one `BoardUpdated` snapshot —
    /// the shared tail of both a live-pass tick and the finish-time swap (D3).
    fn emit_board_snapshot(&self) {
        let Some(listener) = self.listener.lock().unwrap().clone() else { return };
        let items = match self.store.lock().unwrap().list_items_for_session(&self.session_id) {
            Ok(items) => items,
            Err(_) => return,
        };
        let board_items = items.iter().map(convert::board_item).collect();
        listener.on_event(WalkEvent::BoardUpdated { items: board_items });
    }

    /// Builds a partial, all-gaps document from whatever is on the current
    /// live board. Shared by: the offline-degrade path (`queued: true`, D9)
    /// and the "nothing left to process" paths below (`queued: false`) — the
    /// empty-transcript short circuit and the double-finish degrade.
    fn partial_document(&self, queued: bool) -> DocumentPayload {
        let doc_kind = doc_kind_for_template(self.template.as_deref());
        let items = self
            .store
            .lock()
            .unwrap()
            .list_items_for_session(&self.session_id)
            .unwrap_or_default();
        convert::partial_document_from_items(doc_kind, &items, queued)
    }

    /// Degrade path for a `finish()` call that can't transition the session
    /// out of `Recording` — in practice, almost always a second `finish()`
    /// call on a session that already finished. This call has already
    /// crossed into async/FFI territory, so there is no safe panic here: any
    /// unwind here is fatal to the host app. Every failure mode (already
    /// ended, or a genuinely unexpected store error) degrades the same way:
    /// return the document that's already there if phase B built one, else
    /// project the current board into a partial (non-queued — there is
    /// nothing left pending) document.
    fn degraded_document(&self) -> DocumentPayload {
        let existing = {
            let store = self.store.lock().unwrap();
            store
                .list_artifacts_for_session(&self.session_id)
                .unwrap_or_default()
                .into_iter()
                .find(|a| a.kind == "document")
        };
        match existing.as_ref().map(convert::document_payload) {
            Some(Ok(payload)) => payload,
            _ => self.partial_document(false),
        }
    }
}

#[uniffi::export]
impl MurmurEngine {
    /// `Store::start_session` + persists the template key, hands back a
    /// fresh per-session `WalkSession` (D4).
    pub fn begin_walk(self: Arc<Self>, job_id: Option<String>, template: String) -> Arc<WalkSession> {
        let session_id = {
            let store = self.store.lock().unwrap();
            let session = store.start_session(job_id.as_deref()).expect("start_session");
            store
                .set_session_template(&session.id, &template)
                .expect("set_session_template on a freshly-recording session");
            session.id
        };
        let extractor = LiveExtractor::new(
            self.providers.live.clone(),
            self.store.clone(),
            self.memory.clone(),
            &session_id,
        );
        WalkSession::new(
            session_id,
            self.store.clone(),
            extractor,
            self.providers.processing.clone(),
            self.memory.clone(),
            self.memory_store.clone(),
            self.runtime_handle.clone(),
            Some(template),
        )
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalkSession {
    /// Stores the listener (fresh per session — D3/HANDOFF per-session
    /// streams).
    pub fn set_event_listener(self: Arc<Self>, listener: Arc<dyn WalkEventListener>) {
        *self.listener.lock().unwrap() = Some(listener);
    }

    /// Fire-and-forget (D7): writes the transcript chunk through a short
    /// scoped `Store` lock, then spawns the live-extraction tick. The tick
    /// acquires the EXTRACTOR mutex (never the `Store` lock) across
    /// `maybe_extract().await` — the `Store`'s own scoped guards inside
    /// `maybe_extract` are the only place it's locked during the tick.
    pub fn append_transcript(self: Arc<Self>, text: String) {
        {
            let store = self.store.lock().unwrap();
            // A stale append after the session has moved on is a harmless
            // no-op from the bridge's point of view — the store call itself
            // enforces the Recording-only invariant.
            let _ = store.append_transcript(&self.session_id, &text);
        }
        let session = self.clone();
        self.runtime_handle.spawn(async move {
            let outcome = {
                let mut extractor = session.extractor.lock().await;
                extractor.maybe_extract().await
            };
            if let Ok(LiveExtractOutcome::Extracted { .. }) = outcome {
                session.emit_board_snapshot();
            }
        });
    }

    /// D6/D9: `end_and_record_session` + `SessionProcessor::process`, then
    /// the terminal swap snapshot + the structured document.
    ///
    /// Three degrade paths, none of which may panic across the FFI boundary
    /// (a `uniffi::export`ed async fn returns a bare `DocumentPayload`, not a
    /// `Result` — an unwind here is a fatal crash in the host app, not a
    /// catchable error):
    /// - `end_and_record_session` fails (most commonly: a second `finish()`
    ///   call on an already-ended session) -> `degraded_document()`.
    /// - phase B ran but the transcript was empty/whitespace-only, so
    ///   `murmur-core`'s pipeline short-circuited before building a document
    ///   artifact -> a truthful, non-queued `partial_document`.
    /// - phase B failed outright (offline/LLM-down, D9) -> a queued partial
    ///   document built from the live board — capture is never lost.
    pub async fn finish(self: Arc<Self>) -> DocumentPayload {
        // D3b: hold the extractor mutex across the whole call so no live tick
        // can interleave with end-of-session processing.
        let _tick_guard = self.extractor.lock().await;

        let ended = {
            let store = self.store.lock().unwrap();
            store.end_and_record_session(&self.session_id)
        };
        if ended.is_err() {
            return self.degraded_document();
        }

        let processor = SessionProcessor::new(
            self.processing_provider.clone(),
            self.store.clone(),
            self.memory.clone(),
            self.memory_store.clone(),
        );
        match processor.process(&self.session_id).await {
            Ok(_) => {
                self.emit_board_snapshot();
                let doc = {
                    let store = self.store.lock().unwrap();
                    store
                        .list_artifacts_for_session(&self.session_id)
                        .expect("list_artifacts_for_session")
                        .into_iter()
                        .find(|a| a.kind == "document")
                };
                match doc {
                    // The common case: phase B ran and built a document.
                    Some(doc) => {
                        convert::document_payload(&doc).expect("document artifact body is valid JSON")
                    }
                    // The empty-transcript short circuit (murmur-core's
                    // pipeline skips phase B entirely for a
                    // whitespace-only/empty transcript): the session is
                    // genuinely Processed with nothing pending, so this is a
                    // truthful zero/items-only document — not queued.
                    None => self.partial_document(false),
                }
            }
            // Offline / LLM-down degradation (D9): the session did NOT reach
            // Processed, so there's real pending work — queued: true.
            Err(_) => self.partial_document(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};

    use harness::{
        CompletionRequest, CompletionResponse, ContentBlock, HarnessError, MockProvider,
        StopReason, Usage,
    };
    use murmur_core::ItemSource;
    use tokio::sync::mpsc;

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

    /// Forwards every `WalkEvent` onto an unbounded channel so async tests
    /// can `.await` a fire-and-forget tick instead of sleep-polling.
    struct ChannelListener(mpsc::UnboundedSender<WalkEvent>);
    impl WalkEventListener for ChannelListener {
        fn on_event(&self, event: WalkEvent) {
            let _ = self.0.send(event);
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

    fn document_response() -> CompletionResponse {
        tool_use(
            "build_document",
            serde_json::json!({"total_kind": "sum", "total_label_key": "total", "lines": []}),
        )
    }

    /// A provider whose FIRST call blocks on a barrier before answering —
    /// lets a test hold `process()` mid-flight to probe the tick/finish
    /// exclusion (D3b).
    struct BarrierProvider {
        barrier: Arc<tokio::sync::Barrier>,
        responses: StdMutex<VecDeque<CompletionResponse>>,
        first: AtomicBool,
    }

    #[async_trait::async_trait]
    impl LlmProvider for BarrierProvider {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, HarnessError> {
            if self.first.swap(false, Ordering::SeqCst) {
                self.barrier.wait().await;
            }
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| HarnessError::Provider("mock script exhausted".into()))
        }
    }

    fn test_session(
        sid: String,
        store: Arc<StdMutex<Store>>,
        extractor: LiveExtractor,
        processing_provider: Arc<dyn LlmProvider>,
        memory: Arc<StdMutex<Memory>>,
    ) -> Arc<WalkSession> {
        WalkSession::new(
            sid,
            store,
            extractor,
            processing_provider,
            memory,
            Arc::new(NullMemoryStore),
            tokio::runtime::Handle::current(),
            Some("landscape".into()),
        )
    }

    #[tokio::test]
    async fn begin_walk_wires_a_working_session() {
        let store = Store::open_in_memory("device-a").unwrap();
        let engine = MurmurEngine::with_providers(
            store,
            Memory::default(),
            Arc::new(NullMemoryStore),
            Providers {
                live: Arc::new(MockProvider::new(vec![
                    tool_use("add_item", serde_json::json!({"kind": "todo", "text": "order lumber"})),
                    end_turn("captured"),
                ])),
                processing: Arc::new(MockProvider::new(vec![])),
                reflection: Arc::new(MockProvider::new(vec![])),
            },
        );
        let session = engine.begin_walk(None, "landscape".into());

        let (tx, mut rx) = mpsc::unbounded_channel();
        session.clone().set_event_listener(Arc::new(ChannelListener(tx)));

        // Default min_new_chars (120) — pad past it so the tick actually fires.
        let long_text = "order twelve two by tens for the deck framing today. ".repeat(3);
        session.clone().append_transcript(long_text);

        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("tick did not fire in time")
            .expect("channel closed without an event");
        let WalkEvent::BoardUpdated { items } = event;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text, "order lumber");
    }

    #[tokio::test]
    async fn append_ticks_live_extractor_and_emits_one_board_snapshot_per_pass() {
        let store = Store::open_in_memory("device-a").unwrap();
        let sid = store.start_session(None).unwrap().id;
        let store = Arc::new(StdMutex::new(store));
        let memory = Arc::new(StdMutex::new(Memory::default()));

        let mut extractor = LiveExtractor::new(
            Arc::new(MockProvider::new(vec![
                tool_use("add_item", serde_json::json!({"kind": "todo", "text": "order lumber"})),
                end_turn("captured"),
            ])),
            store.clone(),
            memory.clone(),
            &sid,
        );
        extractor.min_new_chars = 1;

        let session = test_session(
            sid.clone(),
            store.clone(),
            extractor,
            Arc::new(MockProvider::new(vec![])),
            memory,
        );

        let (tx, mut rx) = mpsc::unbounded_channel();
        session.clone().set_event_listener(Arc::new(ChannelListener(tx)));

        session.clone().append_transcript("order twelve two by tens for the deck".into());

        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("tick did not fire in time")
            .expect("channel closed without an event");
        let WalkEvent::BoardUpdated { items } = event;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text, "order lumber");

        // A second append_transcript's tick is a no-op (below min_new_chars is
        // moot here since we set it to 1) but must not deadlock — proving the
        // Store lock is never held across `maybe_extract`.
        session.clone().append_transcript("more talk".into());
        let _ = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await;
    }

    #[tokio::test]
    async fn tick_cannot_interleave_with_finish() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session_row = store.start_session(None).unwrap();
        let sid = session_row.id.clone();
        store.add_item_with_source(&sid, "todo", "live capture", ItemSource::Live).unwrap();
        store.append_transcript(&sid, "order twelve two by tens for the deck framing today").unwrap();
        let store = Arc::new(StdMutex::new(store));
        let memory = Arc::new(StdMutex::new(Memory::default()));

        let mut extractor = LiveExtractor::new(
            Arc::new(MockProvider::new(vec![])),
            store.clone(),
            memory.clone(),
            &sid,
        );
        extractor.min_new_chars = 1;

        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let processing_provider: Arc<dyn LlmProvider> = Arc::new(BarrierProvider {
            barrier: barrier.clone(),
            responses: StdMutex::new(VecDeque::from(vec![
                tool_use("add_item", serde_json::json!({"kind": "todo", "text": "order 12 2x10s"})),
                end_turn("done"),
                summary_response("Lumber ordered."),
                document_response(),
            ])),
            first: AtomicBool::new(true),
        });

        let session = test_session(sid.clone(), store.clone(), extractor, processing_provider, memory);

        let (tx, mut rx) = mpsc::unbounded_channel();
        session.clone().set_event_listener(Arc::new(ChannelListener(tx)));

        let finishing = session.clone();
        let finish_task = tokio::spawn(async move { finishing.finish().await });

        // Give finish() a moment to acquire the extractor mutex and block the
        // processing provider's first call on the barrier.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // This tick can't run yet — finish() holds the extractor mutex across
        // the whole call (D3b). It queues behind finish, not ahead of it.
        session.clone().append_transcript("more talk".into());

        barrier.wait().await;
        let payload = finish_task.await.unwrap();
        assert_eq!(payload.lines.len(), 0); // the empty-lines document_response

        // Every snapshot actually delivered carries a non-empty board — the
        // authoritative swap never exposes the pre-06a empty window.
        while let Ok(event) = rx.try_recv() {
            let WalkEvent::BoardUpdated { items } = event;
            assert!(!items.is_empty(), "no snapshot should ever show an empty board (D3b)");
        }
    }
}
