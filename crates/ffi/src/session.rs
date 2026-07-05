//! `WalkSession`: append/finish, the `LiveExtractor` actor, batched board
//! events (Plan 07 D3/D7).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex as StdMutex};

use harness::{LlmProvider, Memory, MemoryStore};
use murmur_core::{doc_kind_for_template, LiveExtractOutcome, LiveExtractor, SessionProcessor, Store};
use tokio::sync::Mutex as TokioMutex;

use crate::convert;
use crate::document::DocumentPayload;
use crate::engine::{EngineError, MurmurEngine};
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
    /// Count of STORE faults swallowed by fire-and-forget live ticks
    /// (carry-note 4). A live pass that fails on the *model* is intentionally
    /// swallowed (D9: `maybe_extract` returns `Ok(Failed)`, capture is safe);
    /// this counts only genuine store faults (a poisoned lock, a sqlite/NotFound
    /// error) that the tick would otherwise discard silently. Surfaced via
    /// `tick_store_fault_count()` so the UI can show a "capture degraded" hint.
    /// Never crashes the tick loop.
    tick_store_faults: AtomicU64,
    /// The STT stream, present only for audio sessions (D3). `None` → a
    /// text-only walk (Plan 07 path, unchanged): `push_audio` is a no-op and
    /// no pump thread is spawned.
    stt: Option<Arc<stt::SttStream>>,
    /// DONE flush toggle (D6): when `true`, `finish()` flushes the final
    /// buffered utterance through the append path before processing.
    flush_on_finish: bool,
    /// Pump-thread control (D2): the dedicated OS thread parks on the `Condvar`
    /// between polls. `finish()`/`cancel()` set `stop` + notify, then join.
    pump: Arc<(StdMutex<PumpState>, Condvar)>,
    /// The join handle for the pump thread, taken by `stop_pump`.
    pump_handle: StdMutex<Option<std::thread::JoinHandle<()>>>,
    /// Set by the FIRST of `finish()`/`cancel()` to run. Guards the two
    /// lifecycle exits against each other: a `cancel()` after `finish()` (or a
    /// double-`cancel()`) is a harmless no-op, and a `finish()` after
    /// `cancel()` degrades instead of resurrecting/reprocessing a tombstoned
    /// session.
    terminated: std::sync::atomic::AtomicBool,
}

/// Shared state the pump thread parks on. `wake` = new PCM was pushed; `stop`
/// = the session is finishing/cancelling and the pump must exit (Task 4).
#[derive(Default)]
struct PumpState {
    wake: bool,
    stop: bool,
}

/// Best-effort safety net (review finding 1): if the host drops its last
/// handle WITHOUT `finish()`/`cancel()` (e.g. a defensive `session = nil` on
/// the Swift side), signal the pump to stop so the thread — and with it the
/// `SttStream`/whisper Metal context — is released instead of parking forever.
/// This is reachable because the pump holds only a `Weak` (see `start_pump`).
/// NON-JOINING by design: `Drop` can run on any thread (including the pump
/// thread itself, when it drops the last upgraded `Arc`) and must never block
/// on an in-flight decode — the thread is detached and exits at its next loop
/// check. `finish()`/`cancel()` remain the deterministic, joining exits.
impl Drop for WalkSession {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.pump;
        if let Ok(mut state) = lock.lock() {
            state.stop = true;
            cvar.notify_all();
        }
        // Detach: taking the JoinHandle and dropping it never blocks. After a
        // finish()/cancel() this is already None (stop_pump joined) — no-op.
        if let Ok(mut handle) = self.pump_handle.lock() {
            drop(handle.take());
        }
    }
}

/// Assemble the ≤100-term STT bias vocabulary at `begin_walk` (D8): the user's
/// memory `vocabulary` section plus an optional small per-template seed, capped
/// at `SttConfig::max_bias_terms`. Reads an existing memory section — no new
/// memory plumbing. `template` is reserved for a future per-template seed list;
/// v1 seeds nothing template-specific.
fn collect_bias_terms(memory: &Memory, _template: Option<&str>) -> Vec<String> {
    let max = stt::SttConfig::default().max_bias_terms;
    memory
        .section_texts(harness::VOCABULARY_SECTION)
        .into_iter()
        .map(str::to_string)
        .take(max)
        .collect()
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
        stt: Option<Arc<stt::SttStream>>,
        flush_on_finish: bool,
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
            tick_store_faults: AtomicU64::new(0),
            stt,
            flush_on_finish,
            pump: Arc::new((StdMutex::new(PumpState::default()), Condvar::new())),
            pump_handle: StdMutex::new(None),
            terminated: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// Test-support constructor injecting a `ScriptedDecoder`-backed
    /// `SttStream` so the pump + append wiring is exercised hermetically (no
    /// model, no `whisper` feature, no mic). `pub`, not `#[cfg(test)]`, because
    /// an integration-test binary (`tests/audio_pump_e2e.rs`) compiles this
    /// crate as an ordinary dependency and needs to call it — same reasoning as
    /// `MurmurEngine::with_providers`. Starts the pump before returning.
    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn new_audio_test_session(
        session_id: String,
        store: Arc<StdMutex<Store>>,
        extractor: LiveExtractor,
        processing_provider: Arc<dyn LlmProvider>,
        memory: Arc<StdMutex<Memory>>,
        memory_store: Arc<dyn MemoryStore>,
        runtime_handle: tokio::runtime::Handle,
        template: Option<String>,
        stt: Arc<stt::SttStream>,
        flush_on_finish: bool,
    ) -> Arc<Self> {
        let session = WalkSession::new(
            session_id,
            store,
            extractor,
            processing_provider,
            memory,
            memory_store,
            runtime_handle,
            template,
            Some(stt),
            flush_on_finish,
        );
        session.clone().start_pump();
        session
    }

    /// Spawn the dedicated STT pump thread (D2) — one OS thread per audio
    /// session. No-op for a text-only (`stt: None`) session. The thread parks
    /// on the `Condvar` between polls (cheap when idle).
    ///
    /// The pump holds only a WEAK reference (review finding 1): a strong `Arc`
    /// here would cycle (session → pump thread → session) and keep the session
    /// and `SttStream` (a ~60MB whisper Metal context) alive forever if the
    /// host dropped its last handle without `finish()`/`cancel()`. With a `Weak`,
    /// the last external drop runs `WalkSession::drop` (the safety net below),
    /// which signals `stop`; the pump also exits on its own if an upgrade ever
    /// fails. It upgrades per pass and drops the strong `Arc` before re-parking,
    /// so it never pins the session across an idle park. `finish()`/`cancel()`
    /// remain the deterministic (joining) exits; `Drop` is best-effort only.
    fn start_pump(self: &Arc<Self>) {
        if self.stt.is_none() {
            return;
        }
        let weak = Arc::downgrade(self);
        let pump = self.pump.clone();
        let handle = std::thread::spawn(move || {
            let (lock, cvar) = &*pump;
            loop {
                // 1. Park until new PCM (`wake`) or shutdown (`stop`).
                let mut state = lock.lock().unwrap();
                while !state.wake && !state.stop {
                    state = cvar.wait(state).unwrap();
                }
                if state.stop {
                    break;
                }
                state.wake = false;
                drop(state); // release before the long decode

                // 2. Session gone (last external Arc dropped) → exit.
                let Some(session) = weak.upgrade() else { break };
                let Some(stt) = session.stt.as_ref() else {
                    break; // unreachable (guarded at spawn) but never panic
                };

                // 3. Poll the STT stream — the long, BLOCKING Metal decode.
                // Lock order (D2): `poll()` takes and RELEASES SttStream's
                // internal engine→input locks and returns the segments before
                // `feed_segments`/`append_transcript` runs, so the STT engine
                // lock is never held across the Store/extractor locks — no lock
                // inversion, no new deadlock surface.
                match stt.poll() {
                    Ok(segs) => session.feed_segments(segs),
                    // A decode error must NOT kill the pump (capture-never-lost):
                    // log and continue to the next poll.
                    Err(e) => eprintln!(
                        "murmur-ffi: stt pump decode error (session {}): {e}",
                        session.session_id
                    ),
                }
                // The strong `Arc` (`session`) drops HERE, before the next park.
            }
        });
        *self.pump_handle.lock().unwrap() = Some(handle);
    }

    /// Feed finalized STT segments into the EXISTING append path (D2). Shared
    /// by the pump loop and the `finish()` flush (Task 4). Non-empty finalized
    /// text only — extraction sees finalized text exactly as the Swift text
    /// path delivered it. Task 3 adds the transcript-event emission here.
    fn feed_segments(self: &Arc<Self>, segs: Vec<stt::FinalizedSegment>) {
        let mut committed = String::new();
        for seg in &segs {
            if seg.text.trim().is_empty() {
                continue;
            }
            // The SAME text that feeds extraction is what the UI commits (D4).
            let chunk = format!("{} ", seg.text);
            committed.push_str(&chunk);
            self.clone().append_transcript(chunk);
        }
        // One TranscriptCommitted per pump pass carrying all finalized text —
        // synchronous from the pump (the board tick is async on runtime_handle;
        // this event is independent of it). Same string that fed extraction.
        if !committed.is_empty() {
            self.emit(WalkEvent::TranscriptCommitted { text: committed });
        }
        // The greyed preview tail (D4): never persisted, never extracted.
        if let Some(stt) = self.stt.as_ref() {
            let tail = stt.preview_tail();
            if !tail.is_empty() {
                self.emit(WalkEvent::TranscriptPreview { text: tail });
            }
        }
    }

    /// Deliver one event to the listener (if any). Central so the pump's
    /// transcript events and the board snapshot share one code path.
    fn emit(&self, event: WalkEvent) {
        if let Some(listener) = self.listener.lock().unwrap().clone() {
            listener.on_event(event);
        }
    }

    /// The single terminal-transition primitive shared by `finish()` and
    /// `cancel()` (review finding 2): returns `true` for exactly ONE caller
    /// over the session's lifetime. INVARIANT — both lifecycle exits call this
    /// FIRST, before acquiring the tick guard: swap-first on BOTH sides is
    /// what guarantees a concurrent finish+cancel resolves to one winner
    /// (finish proceeds and cancel no-ops, or cancel proceeds and finish
    /// degrades) — never "finish degrades AND cancel tombstones", which would
    /// lose the user's DONE and delete the data in the same instant. A losing
    /// `finish()` may return a degraded document while the winner is still
    /// mid-flight; that is the pre-existing double-finish semantics and is
    /// display-safe (the store is read under its own lock).
    fn try_enter_terminal(&self) -> bool {
        !self.terminated.swap(true, Ordering::SeqCst)
    }

    /// Stop the pump thread and JOIN it — the single teardown shared by the
    /// two lifecycle exits `finish()` and `cancel()` (findings 2/3). The pump's
    /// `poll()` runs a long BLOCKING Metal decode, so a bare `.join()` inside an
    /// async fn would block a tokio worker for the duration of an in-flight
    /// decode. Instead we set `stop` + notify, then perform the `join()` on the
    /// blocking pool via `spawn_blocking` — off the async workers (and, for
    /// `cancel()` from Swift's detached `Task`, off the main actor). Returning
    /// from here GUARANTEES the pump is fully stopped, so no detached thread can
    /// call `append_transcript` while we then flush / `delete_session` / process.
    /// Idempotent: a second call finds no handle and no-ops.
    async fn stop_pump(&self) {
        {
            let (lock, cvar) = &*self.pump;
            let mut state = lock.lock().unwrap();
            state.stop = true;
            cvar.notify_all();
        }
        let handle = self.pump_handle.lock().unwrap().take();
        if let Some(handle) = handle {
            let _ = tokio::task::spawn_blocking(move || handle.join()).await;
        }
    }

    /// Flush the final buffered utterance through the transcript on DONE (D6).
    /// Called by `finish()` AFTER `stop_pump()` (so the pump can't race) and
    /// while the extractor mutex is held — so the flushed text is written with a
    /// DIRECT scoped `Store::append_transcript`, NOT the async tick (a tick would
    /// deadlock on the held extractor mutex, D3b). The authoritative `process()`
    /// then reads a transcript that includes the last utterance. A decode error
    /// is logged and skipped — never fatal across FFI.
    fn flush_stt(&self) {
        let Some(stt) = self.stt.as_ref() else { return };
        let segs = match stt.end() {
            Ok(segs) => segs,
            Err(e) => {
                eprintln!("murmur-ffi: stt flush decode error (session {}): {e}", self.session_id);
                return;
            }
        };
        let mut flushed = String::new();
        if let Ok(store) = self.store.lock() {
            for seg in &segs {
                if seg.text.trim().is_empty() {
                    continue;
                }
                let chunk = format!("{} ", seg.text);
                let _ = store.append_transcript(&self.session_id, &chunk);
                flushed.push_str(&chunk);
            }
        }
        // A final committed event for the UI (the board is refreshed by the
        // authoritative process() swap that follows).
        if !flushed.is_empty() {
            self.emit(WalkEvent::TranscriptCommitted { text: flushed });
        }
    }

    /// Records a store fault a tick would otherwise swallow (carry-note 4).
    /// Increments the queryable counter and logs to stderr. There is no logging
    /// crate in this workspace (CI stays dependency-light / hermetic), so stderr
    /// is the honest side channel and the counter is the queryable surface.
    fn record_tick_fault(&self, context: &str) {
        self.tick_store_faults.fetch_add(1, Ordering::Relaxed);
        eprintln!(
            "murmur-ffi: live tick store fault (session {}): {context}",
            self.session_id
        );
    }

    /// Re-queries the board and emits exactly one `BoardUpdated` snapshot —
    /// the shared tail of both a live-pass tick and the finish-time swap (D3).
    fn emit_board_snapshot(&self) {
        let Some(listener) = self.listener.lock().unwrap().clone() else { return };
        // Don't panic across FFI on a poisoned lock (this is also called from
        // finish()): count the degradation and skip the snapshot instead.
        let items = match self.store.lock() {
            Ok(store) => match store.list_items_for_session(&self.session_id) {
                Ok(items) => items,
                Err(e) => {
                    self.record_tick_fault(&format!("list_items_for_session: {e}"));
                    return;
                }
            },
            Err(_) => {
                self.record_tick_fault("store lock poisoned");
                return;
            }
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
            // Scoped to the session's document artifact (carry-note 6), not a
            // sweep of every artifact.
            store.latest_document_artifact(&self.session_id).unwrap_or_default()
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
    /// fresh per-session `WalkSession` (D4). Fallible across FFI (no panics):
    /// a poisoned store lock or a store error surfaces to Swift as
    /// `EngineError::BeginWalk` rather than crashing the host app.
    pub fn begin_walk(
        self: Arc<Self>,
        job_id: Option<String>,
        template: String,
    ) -> Result<Arc<WalkSession>, EngineError> {
        let session_id = {
            let store = self
                .store
                .lock()
                .map_err(|_| EngineError::BeginWalk("store lock poisoned".into()))?;
            // One transaction (review follow-up): a template failure after the
            // insert must not leak an unreachable Recording row.
            store
                .start_session_with_template(job_id.as_deref(), &template)
                .map_err(|e| EngineError::BeginWalk(e.to_string()))?
                .id
        };
        let extractor = LiveExtractor::new(
            self.providers.live.clone(),
            self.store.clone(),
            self.memory.clone(),
            &session_id,
        );
        // Assemble the ≤100-term bias vocabulary once, here, where both the
        // template and Memory are in hand (D8), then build the audio stream
        // (whisper feature + model path) and start the pump. Text-only when
        // there is no model / the feature is off — `stt: None`, pump not spawned.
        let bias = {
            let memory = self
                .memory
                .lock()
                .map_err(|_| EngineError::BeginWalk("memory lock poisoned".into()))?;
            collect_bias_terms(&memory, Some(&template))
        };
        let stt = self.build_stt_stream(&bias)?;
        let session = WalkSession::new(
            session_id,
            self.store.clone(),
            extractor,
            self.providers.processing.clone(),
            self.memory.clone(),
            self.memory_store.clone(),
            self.runtime_handle.clone(),
            Some(template),
            stt,
            self.stt_flush_on_finish,
        );
        session.start_pump();
        Ok(session)
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
        // `Handle::spawn` PANICS if the backing runtime has shut down (the
        // engine was dropped while a pump pass is in flight — review finding
        // 1b). This method is called from the pump's OS thread, which has no
        // unwind boundary; under panic=abort that panic kills the host app.
        // Catch it and degrade to a counted fault instead: the transcript
        // chunk is already persisted above (capture is safe), only the live
        // tick is lost.
        let spawned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.runtime_handle.spawn(async move {
                let outcome = {
                    let mut extractor = session.extractor.lock().await;
                    extractor.maybe_extract().await
                };
                match outcome {
                    Ok(LiveExtractOutcome::Extracted { .. }) => session.emit_board_snapshot(),
                    // Skipped (too little new transcript / not recording) and a
                    // model-side Failed pass (D9: offline/LLM-down) are swallowed by
                    // design — capture is safe and the next tick retries.
                    Ok(LiveExtractOutcome::Skipped | LiveExtractOutcome::Failed { .. }) => {}
                    // A genuine store fault — surfaced (carry-note 4) instead of
                    // silently discarded. Never crashes the tick loop.
                    Err(e) => session.record_tick_fault(&format!("maybe_extract: {e}")),
                }
            })
        }));
        if spawned.is_err() {
            self.record_tick_fault("runtime shut down: live tick not spawned");
        }
    }

    /// Enqueue mic PCM for the STT pump (D1/D2). A CHEAP enqueue: buffers the
    /// samples under a short lock and wakes the pump thread — the long Metal
    /// decode happens on the pump thread, never here. No-op for a text-only
    /// (`stt: None`) session. Must not block: Swift calls this from a
    /// background task fed by the audio render thread.
    pub fn push_audio(self: Arc<Self>, samples: Vec<f32>) {
        let Some(stt) = self.stt.as_ref() else {
            return; // text-only session — the append path handles the walk
        };
        stt.push_pcm(&samples);
        let (lock, cvar) = &*self.pump;
        let mut state = lock.lock().unwrap();
        state.wake = true;
        cvar.notify_one();
    }

    /// Number of store faults swallowed by fire-and-forget live ticks so far
    /// (carry-note 4). A nonzero count means a tick's store access failed (a
    /// poisoned lock, a sqlite/NotFound error) — never a model/offline pass,
    /// which is swallowed by design (D9). The UI can poll this to surface a
    /// "capture degraded" hint. Lock-free read.
    pub fn tick_store_fault_count(&self) -> u64 {
        self.tick_store_faults.load(Ordering::Relaxed)
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
        // Terminal transition FIRST, guard second — the SAME order as
        // `cancel()` (see `try_enter_terminal`). A second finish() (or a
        // finish() after cancel()) degrades to the already-built document
        // rather than reprocessing or resurrecting a tombstoned session.
        if !self.try_enter_terminal() {
            return self.degraded_document();
        }

        // D3b: hold the extractor mutex across the rest of the call so no
        // live tick can interleave with end-of-session processing.
        let _tick_guard = self.extractor.lock().await;

        // Stop the pump (spawn_blocking join — never blocks a worker), THEN
        // flush the final utterance so process() reads the complete transcript
        // (D6). Both happen while the tick guard is held. Order: stop → flush →
        // end_and_record → process.
        self.stop_pump().await;
        if self.flush_on_finish {
            self.flush_stt();
        }

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
            Ok(outcome) => {
                self.emit_board_snapshot();
                // Read EXACTLY the document this run built (carry-note 6) — never
                // sweep the session's artifacts, so a future non-processing
                // `document` writer can't be misread as the document.
                match outcome.document_artifact_id {
                    // The common case: phase B ran and built a document. If the
                    // artifact is somehow unreadable, degrade rather than panic
                    // across FFI (this is a bare `DocumentPayload` return).
                    Some(id) => {
                        let art = {
                            let store = self.store.lock().unwrap();
                            store.get_artifact(&id)
                        };
                        match art.as_ref().map(convert::document_payload) {
                            Ok(Ok(payload)) => payload,
                            _ => self.partial_document(false),
                        }
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

    /// DISCARD (findings 2 + issue #3): stop the pump AND tombstone the session.
    /// The close cousin of `finish()` — SAME shape (async, tick-guard-holding,
    /// `stop_pump().await`) differing only in the tail (`delete_session` instead
    /// of `process()`). Async, NOT a sync export: Swift calls it from a detached
    /// `Task` in `discardWalk()`, and the pump join can land mid-decode — a sync
    /// export would block the UI thread. Idempotent: a second `cancel()`, or a
    /// `cancel()` after `finish()`, is a harmless no-op.
    pub async fn cancel(self: Arc<Self>) {
        // Terminal transition FIRST, guard second — the SAME order as
        // `finish()` (see `try_enter_terminal`).
        if !self.try_enter_terminal() {
            return; // already finished or cancelled — nothing to do
        }
        // Exclude ticks (D3b), then stop the pump deterministically before we
        // touch the store — no detached thread can append mid-tombstone.
        let _tick_guard = self.extractor.lock().await;
        self.stop_pump().await;
        // Tombstone via the EXISTING cascade delete: it removes the session +
        // its items + its artifacts in one transaction (the COMPLETE issue #3
        // fix — a bare "Abandoned" status would leave zombie items). A stale
        // tick that slips through later fails cleanly at get_session's
        // `deleted_at IS NULL` filter.
        if let Ok(store) = self.store.lock() {
            let _ = store.delete_session(&self.session_id);
        }
        // Release the foreign listener. The pump thread holds only a Weak (see
        // start_pump) and has exited via stop_pump; the session frees once
        // Swift drops its handle.
        *self.listener.lock().unwrap() = None;
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
            None,
            true,
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
        let session = engine.begin_walk(None, "landscape".into()).unwrap();

        let (tx, mut rx) = mpsc::unbounded_channel();
        session.clone().set_event_listener(Arc::new(ChannelListener(tx)));

        // Default min_new_chars (120) — pad past it so the tick actually fires.
        let long_text = "order twelve two by tens for the deck framing today. ".repeat(3);
        session.clone().append_transcript(long_text);

        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("tick did not fire in time")
            .expect("channel closed without an event");
        let WalkEvent::BoardUpdated { items } = event else {
            panic!("expected BoardUpdated, got {event:?}");
        };
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
        let WalkEvent::BoardUpdated { items } = event else {
            panic!("expected BoardUpdated, got {event:?}");
        };
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text, "order lumber");

        // A second append_transcript's tick is a no-op (below min_new_chars is
        // moot here since we set it to 1) but must not deadlock — proving the
        // Store lock is never held across `maybe_extract`.
        session.clone().append_transcript("more talk".into());
        let _ = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await;
    }

    #[tokio::test]
    async fn tick_store_fault_is_counted_not_swallowed() {
        let store = Store::open_in_memory("device-a").unwrap();
        // A real session for the WalkSession's own transcript writes...
        let sid = store.start_session(None).unwrap().id;
        let store = Arc::new(StdMutex::new(store));
        let memory = Arc::new(StdMutex::new(Memory::default()));

        // ...but the extractor points at a session that does not exist, so its
        // tick's `get_session` returns NotFound — a genuine store fault that
        // `maybe_extract` surfaces as `Err` (NOT a swallowed model failure).
        let mut extractor = LiveExtractor::new(
            Arc::new(MockProvider::new(vec![])),
            store.clone(),
            memory.clone(),
            "ghost-session",
        );
        extractor.min_new_chars = 1;

        let session = test_session(
            sid.clone(),
            store.clone(),
            extractor,
            Arc::new(MockProvider::new(vec![])),
            memory,
        );

        assert_eq!(session.tick_store_fault_count(), 0);
        session.clone().append_transcript("anything at all".into());

        // Wait for the fire-and-forget tick to land.
        for _ in 0..100 {
            if session.tick_store_fault_count() > 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(
            session.tick_store_fault_count(),
            1,
            "a store fault in the tick must be counted (surfaced), not silently swallowed"
        );
    }

    /// A `ScriptedDecoder`-backed audio session (no model, no feature): pushing
    /// PCM drives the pump, whose finalized text reaches the EXISTING append
    /// path and produces exactly the same board tick a text append would.
    fn scripted_audio_stt() -> Arc<stt::SttStream> {
        use stt::{RawSegment, ScriptedDecoder, SttConfig, SttStream};
        // Realistic time-shifted composition (Plan 06): window k+1 restarts at
        // chunk-relative cs=0; only the 1 s overlap repeats. 9 s of PCM → both
        // 5 s/1 s windows drained in one poll().
        let seg = |cs0: i64, cs1: i64, t: &str| RawSegment { start_cs: cs0, end_cs: cs1, text: t.into(), no_speech_prob: 0.0, words: vec![] };
        let decoder = ScriptedDecoder::new(vec![
            vec![seg(0, 180, "order twelve"), seg(180, 360, "two by tens"), seg(360, 480, "for the")],
            vec![seg(0, 80, "for the"), seg(80, 300, "deck framing"), seg(300, 480, "today")],
        ]);
        Arc::new(SttStream::with_decoder(Box::new(decoder), SttConfig::default(), &[]))
    }

    #[tokio::test]
    async fn push_audio_pumps_stt_and_feeds_the_append_path() {
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

        let session = WalkSession::new_audio_test_session(
            sid,
            store,
            extractor,
            Arc::new(MockProvider::new(vec![])),
            memory,
            Arc::new(NullMemoryStore),
            tokio::runtime::Handle::current(),
            Some("landscape".into()),
            scripted_audio_stt(),
            true,
        );

        let (tx, mut rx) = mpsc::unbounded_channel();
        session.clone().set_event_listener(Arc::new(ChannelListener(tx)));

        // 9 s of PCM → the pump wakes, polls, drains both windows, and feeds
        // the finalized text through append_transcript → a live tick.
        session.clone().push_audio(vec![0.0; 144_000]);

        // The pump's finalized text drives a live tick → one BoardUpdated
        // (transcript events also share the channel — skip them here).
        let items = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                match rx.recv().await {
                    Some(WalkEvent::BoardUpdated { items }) => return items,
                    Some(_) => continue,
                    None => panic!("channel closed without a BoardUpdated"),
                }
            }
        })
        .await
        .expect("pump did not drive a board tick in time");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text, "order lumber");

        // A second push after the first tick must not hang (no deadlock): the
        // pump has no more scripted windows, so poll() is a no-op.
        session.clone().push_audio(vec![0.0; 1000]);
        let _ = tokio::time::timeout(std::time::Duration::from_millis(300), rx.recv()).await;
    }

    #[tokio::test]
    async fn pump_emits_transcript_committed_matching_the_extracted_text() {
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

        let session = WalkSession::new_audio_test_session(
            sid,
            store,
            extractor,
            Arc::new(MockProvider::new(vec![])),
            memory,
            Arc::new(NullMemoryStore),
            tokio::runtime::Handle::current(),
            Some("landscape".into()),
            scripted_audio_stt(),
            true,
        );

        let (tx, mut rx) = mpsc::unbounded_channel();
        session.clone().set_event_listener(Arc::new(ChannelListener(tx)));
        session.clone().push_audio(vec![0.0; 144_000]);

        // Collect events until we see a TranscriptCommitted AND a BoardUpdated.
        let mut committed: Option<String> = None;
        let mut board_text: Option<String> = None;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                match rx.recv().await {
                    Some(WalkEvent::TranscriptCommitted { text }) => committed = Some(text),
                    Some(WalkEvent::BoardUpdated { items }) => {
                        if let Some(item) = items.first() {
                            board_text = Some(item.text.clone());
                        }
                    }
                    Some(WalkEvent::TranscriptPreview { .. }) => {}
                    None => break,
                }
                if committed.is_some() && board_text.is_some() {
                    break;
                }
            }
        })
        .await;

        let committed = committed.expect("pump must emit a TranscriptCommitted event");
        // Finalized words the scripted decoder produced feed BOTH the committed
        // event and the append/extraction path — same finalized text (D4).
        assert!(committed.contains("order"), "committed text carries the finalized words: {committed:?}");
        assert!(committed.contains("deck framing"), "committed text: {committed:?}");
        assert_eq!(board_text.as_deref(), Some("order lumber"), "same finalized text drove extraction");
    }

    #[tokio::test]
    async fn begin_walk_without_model_path_is_text_only() {
        // with_providers defaults stt_model_path=None → the session's stt slot
        // is None, push_audio is a no-op, and the text append path is intact.
        let store = Store::open_in_memory("device-a").unwrap();
        let engine = MurmurEngine::with_providers(
            store,
            Memory::default(),
            Arc::new(NullMemoryStore),
            Providers {
                live: Arc::new(MockProvider::new(vec![])),
                processing: Arc::new(MockProvider::new(vec![])),
                reflection: Arc::new(MockProvider::new(vec![])),
            },
        );
        let session = engine.begin_walk(None, "landscape".into()).unwrap();
        assert!(session.stt.is_none(), "no model path → text-only session (stt: None)");
        // Neither call panics; the text path still writes a transcript.
        session.clone().push_audio(vec![0.0; 16_000]);
        session.clone().append_transcript("hello there".into());
    }

    /// Env + feature gated real-model smoke (never runs in CI — like stt's
    /// `real_model_decodes_silence`): with `--features whisper` and
    /// `MURMUR_WHISPER_MODEL` set, begin_walk builds a real SttStream and a
    /// silent push drives a pump pass without error.
    // A plain `#[test]` (not `#[tokio::test]`): `MurmurEngine::new` owns its own
    // tokio Runtime, and dropping that Runtime inside another runtime's async
    // context panics — so this smoke test stays synchronous and drives the async
    // cancel via the engine's own runtime handle.
    #[cfg(feature = "whisper")]
    #[test]
    #[ignore = "needs the whisper feature + MURMUR_WHISPER_MODEL; never in CI"]
    fn real_model_begin_walk_builds_an_stt_session() {
        let Ok(model) = std::env::var("MURMUR_WHISPER_MODEL") else {
            return; // no model provided — nothing to smoke
        };
        let db = std::env::temp_dir().join("murmur-stt-smoke.db");
        let cfg = crate::engine::EngineConfig {
            db_path: db.to_string_lossy().into_owned(),
            device_id: "smoke".into(),
            api_key: "sk-test".into(),
            base_url: None,
            model_live: "claude-haiku-4-5".into(),
            model_processing: "claude-sonnet-4-5".into(),
            model_reflection: "claude-haiku-4-5".into(),
            stt_model_path: Some(model),
            stt_flush_on_finish: true,
            stt_use_gpu: true, // host-side smoke — Metal is fine here
            stt_vad_rms_threshold: 0.0,
            stt_no_speech_prob_threshold: 0.6,
        };
        let engine = MurmurEngine::new(cfg).expect("engine construction");
        let session =
            engine.clone().begin_walk(None, "landscape".into()).expect("begin_walk with a real model");
        assert!(session.stt.is_some(), "a valid model path builds an SttStream");
        session.clone().push_audio(vec![0.0; 16_000]); // 1 s of silence
        std::thread::sleep(std::time::Duration::from_millis(300));
        // Clean teardown (stops the pump) on the engine's runtime.
        engine.runtime_handle.clone().block_on(session.cancel());
    }

    #[tokio::test]
    async fn push_audio_on_a_text_only_session_is_a_noop() {
        let store = Store::open_in_memory("device-a").unwrap();
        let sid = store.start_session(None).unwrap().id;
        let store = Arc::new(StdMutex::new(store));
        let memory = Arc::new(StdMutex::new(Memory::default()));
        let extractor = LiveExtractor::new(
            Arc::new(MockProvider::new(vec![])),
            store.clone(),
            memory.clone(),
            &sid,
        );
        let session = test_session(sid, store, extractor, Arc::new(MockProvider::new(vec![])), memory);
        // `stt: None` → push_audio does nothing and never panics.
        session.clone().push_audio(vec![0.0; 16_000]);
    }

    /// A ScriptedDecoder SttStream whose LAST utterance ("today") straddles the
    /// final horizon and is only finalized by `end()` — so the flush path is
    /// observable: with flush, "today" reaches the transcript; without, it does
    /// not. Mirrors the stt crate's `poll_finalizes_incrementally_and_end_flushes`.
    fn scripted_flush_stt() -> Arc<stt::SttStream> {
        use stt::{RawSegment, ScriptedDecoder, SttConfig, SttStream};
        let seg = |cs0: i64, cs1: i64, t: &str| RawSegment { start_cs: cs0, end_cs: cs1, text: t.into(), no_speech_prob: 0.0, words: vec![] };
        let decoder = ScriptedDecoder::new(vec![
            vec![seg(0, 180, "order twelve"), seg(180, 360, "two by tens"), seg(360, 480, "for the")],
            vec![seg(0, 80, "for the"), seg(80, 300, "deck framing"), seg(300, 480, "today")],
            vec![seg(0, 80, "today")], // flush window: only end() decodes this
        ]);
        Arc::new(SttStream::with_decoder(Box::new(decoder), SttConfig::default(), &[]))
    }

    /// Push audio and wait until the pump has emitted its first
    /// TranscriptCommitted — proof the pump drained the ready windows and
    /// appended before we finish/cancel (removes the pump-vs-teardown race).
    async fn push_and_await_commit(session: &Arc<WalkSession>, rx: &mut mpsc::UnboundedReceiver<WalkEvent>) {
        session.clone().push_audio(vec![0.0; 144_000]);
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                match rx.recv().await {
                    Some(WalkEvent::TranscriptCommitted { .. }) => return,
                    Some(_) => continue,
                    None => panic!("channel closed before a TranscriptCommitted"),
                }
            }
        })
        .await
        .expect("pump did not commit finalized text in time");
    }

    fn flush_test_session(flush_on_finish: bool) -> (Arc<WalkSession>, String, Arc<StdMutex<Store>>) {
        let store = Store::open_in_memory("device-a").unwrap();
        let sid = store.start_session(None).unwrap().id;
        let store = Arc::new(StdMutex::new(store));
        let memory = Arc::new(StdMutex::new(Memory::default()));
        let mut extractor = LiveExtractor::new(
            Arc::new(MockProvider::new(vec![])),
            store.clone(),
            memory.clone(),
            &sid,
        );
        // Keep the live extractor quiet — this test is about the flush, not ticks.
        extractor.min_new_chars = 100_000;
        let session = WalkSession::new_audio_test_session(
            sid.clone(),
            store.clone(),
            extractor,
            Arc::new(MockProvider::new(vec![])),
            memory,
            Arc::new(NullMemoryStore),
            tokio::runtime::Handle::current(),
            Some("landscape".into()),
            scripted_flush_stt(),
            flush_on_finish,
        );
        (session, sid, store)
    }

    #[tokio::test]
    async fn finish_flushes_the_final_utterance_by_default() {
        let (session, sid, store) = flush_test_session(true);
        let (tx, mut rx) = mpsc::unbounded_channel();
        session.clone().set_event_listener(Arc::new(ChannelListener(tx)));
        push_and_await_commit(&session, &mut rx).await;

        session.clone().finish().await;

        let transcript = store.lock().unwrap().get_session(&sid).unwrap().transcript;
        assert!(transcript.contains("deck framing"), "live-pass text is present: {transcript:?}");
        assert!(transcript.contains("today"), "flush finalized the last utterance (D6): {transcript:?}");
    }

    #[tokio::test]
    async fn finish_without_flush_drops_the_final_utterance() {
        let (session, sid, store) = flush_test_session(false);
        let (tx, mut rx) = mpsc::unbounded_channel();
        session.clone().set_event_listener(Arc::new(ChannelListener(tx)));
        push_and_await_commit(&session, &mut rx).await;

        session.clone().finish().await;

        let transcript = store.lock().unwrap().get_session(&sid).unwrap().transcript;
        assert!(transcript.contains("deck framing"), "live-pass text is still present: {transcript:?}");
        assert!(!transcript.contains("today"), "speed path does NOT flush the held tail: {transcript:?}");
    }

    #[tokio::test]
    async fn cancel_stops_the_pump_and_tombstones_the_session() {
        let (session, sid, store) = flush_test_session(true);
        let (tx, mut rx) = mpsc::unbounded_channel();
        session.clone().set_event_listener(Arc::new(ChannelListener(tx)));
        push_and_await_commit(&session, &mut rx).await;

        session.clone().cancel().await;

        // (b) the session is tombstoned — get_session's `deleted_at IS NULL`
        // filter now returns NotFound.
        assert!(
            store.lock().unwrap().get_session(&sid).is_err(),
            "cancel() must tombstone the session (issue #3)"
        );
        // (a) the pump has exited: drain any events buffered before cancel, then
        // a further push produces no NEW event (nothing polls the stream).
        while rx.try_recv().is_ok() {}
        session.clone().push_audio(vec![0.0; 144_000]);
        let after = tokio::time::timeout(std::time::Duration::from_millis(300), rx.recv()).await;
        // Either a timeout (Err) or a closed channel (Ok(None), since cancel drops
        // the listener) — but NEVER a new event: the pump has stopped.
        assert!(!matches!(after, Ok(Some(_))), "no new event after cancel — the pump has stopped");

        // Idempotent: a second cancel(), and a finish() after cancel(), do not panic.
        session.clone().cancel().await;
        let _ = session.clone().finish().await;
    }

    #[tokio::test]
    async fn dropping_the_last_handle_stops_the_pump() {
        // Review finding 1: no finish(), no cancel() — the host just drops its
        // last Arc. The pump holds only a Weak, and WalkSession::drop signals
        // stop, so the thread must exit (releasing the SttStream) instead of
        // parking forever.
        let (session, _sid, _store) = flush_test_session(true);
        // Take the JoinHandle so the exit is observable; Drop's own take then
        // finds None (harmless — Drop only detaches, never joins).
        let handle = session
            .pump_handle
            .lock()
            .unwrap()
            .take()
            .expect("audio session must have a running pump");

        drop(session); // the LAST strong Arc — runs the Drop safety net

        let joined = tokio::task::spawn_blocking(move || handle.join());
        tokio::time::timeout(std::time::Duration::from_secs(2), joined)
            .await
            .expect("pump thread did not exit after the last Arc dropped")
            .expect("join task panicked")
            .expect("pump thread panicked");
    }

    #[tokio::test]
    async fn bias_terms_from_memory_vocabulary() {
        let mut memory = Memory::default();
        memory.remember("vocabulary", "french drain", 0);
        memory.remember("vocabulary", "ledger board", 0);
        memory.remember("people", "not a term", 0);
        let terms = collect_bias_terms(&memory, Some("landscape"));
        assert_eq!(terms, vec!["french drain".to_string(), "ledger board".to_string()]);

        // Cap at SttConfig::max_bias_terms (100).
        let mut big = Memory::default();
        for i in 0..150 {
            big.remember("vocabulary", &format!("term{i}"), 0);
        }
        assert_eq!(collect_bias_terms(&big, None).len(), 100);
    }

    #[tokio::test]
    async fn vocabulary_added_via_ffi_feeds_begin_walk_bias_assembly() {
        let store = Store::open_in_memory("device-a").unwrap();
        let engine = MurmurEngine::with_providers(
            store,
            Memory::default(),
            Arc::new(NullMemoryStore),
            Providers {
                live: Arc::new(MockProvider::new(vec![])),
                processing: Arc::new(MockProvider::new(vec![])),
                reflection: Arc::new(MockProvider::new(vec![])),
            },
        );
        // WRITE half (the new FFI path):
        engine.add_vocabulary_term("french drain".into()).unwrap();
        engine.add_vocabulary_term("ledger board".into()).unwrap();

        // READ half (what begin_walk assembles): the terms flow through
        // collect_bias_terms → build_bias_prompt exactly as begin_walk uses them.
        let bias = {
            let mem = engine.memory.lock().unwrap();
            collect_bias_terms(&mem, Some("landscape"))
        };
        assert_eq!(bias, vec!["french drain".to_string(), "ledger board".to_string()]);
        let prompt = stt::build_bias_prompt(&bias, stt::SttConfig::default().max_bias_terms).unwrap();
        assert!(
            prompt.contains("french drain") && prompt.contains("ledger board"),
            "the whisper initial_prompt carries the user's added terms: {prompt}"
        );
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
            // Transcript events may now share this channel — skip them; only
            // board snapshots carry the empty-board invariant.
            if let WalkEvent::BoardUpdated { items } = event {
                assert!(!items.is_empty(), "no snapshot should ever show an empty board (D3b)");
            }
        }
    }
}
