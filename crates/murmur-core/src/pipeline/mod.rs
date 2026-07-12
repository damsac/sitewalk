//! End-of-session processing (spec §6): transcript in, structured records +
//! summary out. Reprocessing is idempotent — the old board is **swapped out
//! in the finish transaction** (source-aware), so a Failed retry can't
//! duplicate todos and a *failure* leaves the live board intact.

pub mod tools;

pub mod live;

pub mod document;

pub mod notes;

pub(crate) mod prompts;

use std::sync::{Arc, Mutex};

use harness::{
    Agent, AgentConfig, ContextAssembler, ContextSection, LlmProvider, Memory, MemoryStore,
    Message, ToolRegistry, UpdateMemoryTool, Usage,
};

use crate::domain::{Session, SessionStatus};
use crate::error::CoreError;
use crate::store::Store;
use tools::{AddItemTool, UpsertContactTool, WriteReportTool};

/// Plan 13 D8: the legal `doc_kind` vocabulary for a session's template, in
/// priority order (`[0]` becomes the template's default kind when Stage 2
/// redefines `doc_kind_for_template` as `[0]` — deferred, see that
/// function's doc comment). This is core's concern (kind vocabulary +
/// pricing flags); which button *leads* and its label copy are sac's.
pub fn doc_kinds_for_template(template: Option<&str>) -> &'static [&'static str] {
    match template {
        Some("landscape") => &["estimate", "invoice", "work_order"],
        Some("property") => &["condition", "move_out"],
        Some("inspection") => &["inspection"],
        _ => &["report"],
    }
}

/// Plan 13 D5: whether a `doc_kind` needs a pricing pass (an amount on every
/// line). Only `estimate`/`invoice` are pricing kinds.
pub fn is_pricing_kind(kind: &str) -> bool {
    matches!(kind, "estimate" | "invoice")
}

/// Maps a session's template key (D4: `landscape`|`property`|`inspection`) to
/// its DEFAULT `doc_kind` — carried as the advisory `NotesPayload.doc_kind`
/// hint and used by the FFI offline fallback (`ffi/src/session.rs::partial_notes`).
///
/// **Plan 13 N3 — the Stage 2 flip.** Redefined as `doc_kinds_for_template(t)[0]`
/// (property's default is now `condition`, not `report` — property's own
/// legal-kind list starts with `condition`). This is safe now that phase B
/// is gone (this function is no longer the LIVE build path) and Swift's
/// `switch docKind` chrome tables gained `condition`/`move_out`/`invoice`/
/// `work_order` arms alongside this flip (`MurmurEngine.swift`). `doc_kind`
/// is advisory only — Swift's button wiring keys off the client-known
/// template (D2), never off this value, so the copy switch cannot mis-route.
/// `DocumentBuilder` never calls this — it validates against
/// `doc_kinds_for_template` (plural) + `is_pricing_kind` directly.
pub fn doc_kind_for_template(template: Option<&str>) -> &'static str {
    doc_kinds_for_template(template)[0]
}

#[derive(Debug)]
pub struct ProcessOutcome {
    pub session: Session,
    pub usage: Usage,
}

pub struct SessionProcessor {
    provider: Arc<dyn LlmProvider>,
    pub(crate) store: Arc<Mutex<Store>>,
    memory: Arc<Mutex<Memory>>,
    memory_store: Arc<dyn MemoryStore>,
    /// Extraction-pass agent budget.
    pub max_turns: usize,
    pub max_tokens: u32,
    /// Transcript token budget for both passes (chars/4 approximation).
    pub transcript_budget_tokens: usize,
    /// Summary-call output budget.
    pub summary_max_tokens: u32,
}

impl SessionProcessor {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        store: Arc<Mutex<Store>>,
        memory: Arc<Mutex<Memory>>,
        memory_store: Arc<dyn MemoryStore>,
    ) -> Self {
        SessionProcessor {
            provider,
            store,
            memory,
            memory_store,
            max_turns: 16,
            max_tokens: 4096,
            transcript_budget_tokens: 12_000,
            // C2: 512 -> 1024 so the narrative summary + up to 12 notes
            // entries fit in one write_notes response without truncation.
            // No new call (D1-14) — this is an output-token budget bump on
            // the one pass that already ran.
            summary_max_tokens: 1024,
        }
    }

    fn locked(&self) -> Result<std::sync::MutexGuard<'_, Store>, CoreError> {
        self.store
            .lock()
            .map_err(|_| CoreError::InvalidState("store lock poisoned".into()))
    }

    /// Processes one ended session. Valid from AwaitingProcessing or Failed
    /// (retry). On success: outputs written, summary set, status Processed.
    /// On LLM failure: status Failed, cost still logged (R9), error returned.
    ///
    /// The app shell must not delete or mutate a session while it is being
    /// processed — status is re-validated only at the exit write, so a
    /// concurrent tombstone would produce a silent no-op or a store error.
    pub async fn process(&self, session_id: &str) -> Result<ProcessOutcome, CoreError> {
        // Phase 0: validate, sweep prior FAILED-run authoritative leftovers
        // (never the live board), and snapshot the transcript. Plan 13 Stage
        // 2 dropped phase B (the forced build_document call), so the
        // template/existing-doc-number snapshot that fed it is gone too —
        // documents are now built on demand (`DocumentBuilder::build`,
        // engine-keyed, not part of `process()`).
        let transcript = {
            let store = self.locked()?;
            let session = store.get_session(session_id)?;
            if !matches!(
                session.status,
                SessionStatus::AwaitingProcessing | SessionStatus::Failed
            ) {
                return Err(CoreError::InvalidState(format!(
                    "cannot process a {} session",
                    session.status.as_str()
                )));
            }
            // Sweep a prior FAILED attempt's authoritative leftovers (+ artifacts,
            // including any `session_meta` spoken-total artifact) so repeated
            // retries can't accumulate duplicate todos or a stale hint. Never
            // touches the live board (the safety net) or manual items.
            store.clear_authoritative_outputs(session_id)?;
            session.transcript
        };

        // Empty guard: an empty/whitespace-only transcript would send empty
        // content blocks to the real API (rejected). Skip the LLM phase and
        // process with a placeholder summary; zero usage is correct — no call
        // was made, and the tx helper's contract is status+usage together.
        if transcript.trim().is_empty() {
            let usage = Usage::default();
            let session = self.locked()?.finish_session_processed(
                session_id,
                "(empty session)",
                &usage,
                &[],
            )?;
            return Ok(ProcessOutcome { session, usage });
        }

        // Memory lock in its own scope — never held alongside the store guard
        // (no store→memory lock ordering for a second caller to deadlock on).
        let memory_prompt = self
            .memory
            .lock()
            .map_err(|_| CoreError::InvalidState("memory lock poisoned".into()))?
            .to_prompt();

        let assembled = ContextAssembler::assemble(&[ContextSection {
            title: "transcript".into(),
            content: transcript,
            budget_tokens: self.transcript_budget_tokens,
        }]);

        // Phase 1+2: extraction agent pass, forced summary (D5a: the summary
        // call may also return an optional spoken grand-total scalar). The id
        // sink records which items THIS run created, for the finish swap.
        let mut usage = Usage::default();
        let created_ids = Arc::new(Mutex::new(Vec::<String>::new()));
        let result = self
            .run_llm_phases(session_id, &assembled.text, &memory_prompt, &mut usage, created_ids.clone())
            .await;

        // Exit: persist outcome + cost atomically, success or not.
        let store = self.locked()?;
        match result {
            Ok((summary, spoken_total_cents, buckets)) => {
                let ids = created_ids
                    .lock()
                    .map_err(|_| CoreError::InvalidState("created-ids lock poisoned".into()))?
                    .clone();
                // D5a: persist the spoken grand-total scalar (if any) as a tiny
                // per-session artifact BEFORE the finish swap — no migration,
                // `kind` is free-form (artifacts.rs:24). Absent unless the
                // model clearly heard a stated total (R6).
                if let Some(cents) = spoken_total_cents {
                    store.add_artifact(
                        session_id,
                        "session_meta",
                        "session_meta",
                        &serde_json::json!({ "spoken_total_cents": cents }).to_string(),
                    )?;
                }
                // Plan 14 D5-14: persist buckets as a notes artifact BEFORE the
                // finish swap, only when non-empty (mirrors session_meta above).
                // clear_authoritative_outputs already sweeps it on reprocess.
                if !buckets.is_empty() {
                    store.add_artifact(
                        session_id,
                        "notes",
                        "notes",
                        &notes::serialize_buckets(&buckets),
                    )?;
                }
                let session = store.finish_session_processed(session_id, &summary, &usage, &ids)?;
                Ok(ProcessOutcome { session, usage })
            }
            Err(e) => {
                // Bookkeeping errors are secondary: the original LLM error is
                // what the caller must see — never mask it with a DB failure.
                let _ = store.finish_session_failed(session_id, &usage);
                Err(e.into())
            }
        }
    }

    async fn run_llm_phases(
        &self,
        session_id: &str,
        assembled_transcript: &str,
        memory_prompt: &str,
        usage: &mut Usage,
        created_ids: Arc<Mutex<Vec<String>>>,
    ) -> Result<(String, Option<i64>, Vec<notes::NotesEntry>), harness::HarnessError> {
        let mut registry = ToolRegistry::new();
        registry.register(AddItemTool::authoritative(
            self.store.clone(),
            session_id,
            created_ids.clone(),
        ));
        registry.register(UpsertContactTool::new(self.store.clone()));
        registry.register(WriteReportTool::new(self.store.clone(), session_id));
        registry.register(
            UpdateMemoryTool::new(self.memory.clone(), self.memory_store.clone())
                .for_session(session_id),
        );

        let agent = Agent::new(
            self.provider.clone(),
            registry,
            AgentConfig {
                system_prompt: prompts::extraction_system_prompt(memory_prompt),
                max_turns: self.max_turns,
                max_tokens: self.max_tokens,
            },
        );
        let outcome = match agent
            .run(vec![Message::user_text(format!(
                "Process this session.\n\n{assembled_transcript}"
            ))])
            .await
        {
            Ok(o) => o,
            Err(run_err) => {
                // Accumulate partial usage before propagating (R9: cost is measured
                // from day one, even when the agent aborts mid-run).
                usage.add(&run_err.usage);
                return Err(run_err.source);
            }
        };
        usage.add(&outcome.usage);

        let (summary, spoken_total_cents, buckets, summary_usage) = prompts::summarize(
            self.provider.clone(),
            assembled_transcript,
            self.summary_max_tokens,
        )
        .await?;
        // Count the summary/notes call's tokens BEFORE judging its content
        // (R9: a model that skipped the tool still cost us the call).
        usage.add(&summary_usage);
        let summary = summary.ok_or_else(|| {
            harness::HarnessError::Provider("summary response missing write_notes call".into())
        })?;

        Ok((summary, spoken_total_cents, buckets))
    }

    /// Drains the awaiting_processing queue (spec §6: offline sessions queue
    /// and process on reconnect). One session at a time — failures mark that
    /// session Failed and the drain continues. Failed sessions are NOT
    /// auto-retried here; retry is an explicit `process()` call (user-visible
    /// retry affordance, R7).
    ///
    /// Drain order: newest-first — the most recent session is what the user
    /// is waiting on; a reconnect backlog processes LIFO.
    pub async fn process_pending(
        &self,
    ) -> Result<Vec<(String, Result<ProcessOutcome, CoreError>)>, CoreError> {
        let queued = self
            .locked()?
            .list_session_summaries_by_status(SessionStatus::AwaitingProcessing)?;
        let mut results = Vec::with_capacity(queued.len());
        for summary in queued {
            let outcome = self.process(&summary.id).await;
            results.push((summary.id, outcome));
        }
        Ok(results)
    }

    /// The offline-banner promise, made true: retries every `Failed` session
    /// (offline drop, LLM error, or a `sweep_zombie_sessions` crash-orphan —
    /// they land in the same bucket and are indistinguishable here) once per
    /// call. Separate from `process_pending` on purpose — that drain's
    /// no-retry pin (`process_pending_does_not_retry_failed_sessions`) stays
    /// exactly as written; this is a distinct, explicit call site (the app's
    /// app-open hook), not a weakening of that guarantee.
    ///
    /// One attempt per session per call — a session that fails again STAYS
    /// Failed for the next app-open to try, never looped on and never
    /// silently dropped (`process()`'s own Failed-marking + cost logging,
    /// R9, is unchanged).
    ///
    /// Capped at `MAX_RETRIES_PER_CALL`, oldest-first: an app-open is a
    /// latency-sensitive moment (this must not turn into a dozen serial LLM
    /// calls blocking the UI), and R9 means a huge backlog isn't retried for
    /// free — the longest-waiting walk gets first crack, the rest wait for a
    /// future app-open.
    pub async fn retry_failed_sessions(
        &self,
    ) -> Result<Vec<(String, Result<ProcessOutcome, CoreError>)>, CoreError> {
        const MAX_RETRIES_PER_CALL: usize = 5;

        // `list_session_summaries_by_status` is newest-first (started_at DESC);
        // reverse to oldest-first before capping so the cap drops the NEWEST
        // stragglers, not the ones that have been waiting longest.
        let mut failed = self.locked()?.list_session_summaries_by_status(SessionStatus::Failed)?;
        failed.reverse();
        failed.truncate(MAX_RETRIES_PER_CALL);

        let mut results = Vec::with_capacity(failed.len());
        for summary in failed {
            let outcome = self.process(&summary.id).await;
            results.push((summary.id, outcome));
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use harness::{
        CompletionResponse, ContentBlock, FactSource, HarnessError, Memory, MemoryStore,
        MockProvider, StopReason, Usage,
    };

    use crate::domain::SessionStatus;
    use crate::error::CoreError;
    use crate::store::Store;

    use super::*;

    /// Memory store stub — pipeline tests don't touch disk.
    struct NullMemoryStore;
    impl MemoryStore for NullMemoryStore {
        fn load(&self) -> Result<Memory, HarnessError> {
            Ok(Memory::default())
        }
        fn save(&self, _m: &Memory) -> Result<(), HarnessError> {
            Ok(())
        }
    }

    fn processor_with(
        responses: Vec<CompletionResponse>,
    ) -> (SessionProcessor, Arc<Mutex<Store>>, String) {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        store
            .append_transcript(&session.id, "we need lumber. call Dev the framer.")
            .unwrap();
        store.end_and_record_session(&session.id).unwrap();
        let store = Arc::new(Mutex::new(store));
        let processor = SessionProcessor::new(
            Arc::new(MockProvider::new(responses)),
            store.clone(),
            Arc::new(Mutex::new(Memory::default())),
            Arc::new(NullMemoryStore),
        );
        (processor, store, session.id)
    }

    fn tool_use(name: &str, input: serde_json::Value) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::ToolUse { id: "tu_1".into(), name: name.into(), input }],
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 100, output_tokens: 20 },
        }
    }

    fn end_turn(text: &str) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::Text { text: text.into() }],
            stop_reason: StopReason::EndTurn,
            usage: Usage { input_tokens: 50, output_tokens: 10 },
        }
    }

    fn summary_response(text: &str) -> CompletionResponse {
        tool_use("write_notes", serde_json::json!({"summary": text}))
    }

    #[tokio::test]
    async fn processes_a_session_end_to_end() {
        let (processor, store, sid) = processor_with(vec![
            tool_use("add_item", serde_json::json!({"kind": "todo", "text": "order lumber"})),
            tool_use("upsert_contact", serde_json::json!({"name": "Dev", "trade": "framer"})),
            end_turn("done"),
            summary_response("Ordered lumber; Dev handles framing."),
        ]);
        let outcome = processor.process(&sid).await.unwrap();
        assert_eq!(outcome.session.status, SessionStatus::Processed);
        assert_eq!(outcome.session.summary.as_deref(), Some("Ordered lumber; Dev handles framing."));
        // usage: 100+20, 100+20, 50+10 agent + 100+20 summary — NO build_document
        // call (Plan 13 Stage 2 drops phase B): finish is strictly cheaper.
        assert_eq!(outcome.usage, Usage { input_tokens: 350, output_tokens: 70 });

        let store = store.lock().unwrap();
        assert_eq!(store.list_items_for_session(&sid).unwrap().len(), 1);
        assert_eq!(store.list_contacts().unwrap().len(), 1);
        let usage_rows = store.list_llm_usage_for_session(&sid).unwrap();
        assert_eq!(usage_rows.len(), 1);
        assert_eq!(usage_rows[0].purpose, "processing");
        assert_eq!(usage_rows[0].input_tokens, 350);
        let artifacts = store.list_artifacts_for_session(&sid).unwrap();
        assert!(!artifacts.iter().any(|a| a.kind == "document"), "phase B is gone — no document artifact");
    }

    #[tokio::test]
    async fn no_build_document_request_is_made_across_a_run() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        store.append_transcript(&session.id, "we need lumber. call Dev the framer.").unwrap();
        store.end_and_record_session(&session.id).unwrap();
        let provider = Arc::new(MockProvider::new(vec![
            tool_use("add_item", serde_json::json!({"kind": "todo", "text": "order lumber"})),
            end_turn("done"),
            summary_response("Ordered lumber."),
        ]));
        let processor = SessionProcessor::new(
            provider.clone(),
            Arc::new(Mutex::new(store)),
            Arc::new(Mutex::new(Memory::default())),
            Arc::new(NullMemoryStore),
        );
        processor.process(&session.id).await.unwrap();
        assert!(
            !provider.requests().iter().any(|r| r.tool_choice.as_deref() == Some("build_document")),
            "phase B is gone — process() must never request build_document"
        );
    }

    #[tokio::test]
    async fn failure_marks_failed_and_still_logs_usage() {
        // agent pass succeeds, summary response has no tool call -> Provider error
        let (processor, store, sid) = processor_with(vec![
            end_turn("nothing to extract"),
            end_turn("I refuse to call tools"),
        ]);
        let err = processor.process(&sid).await.unwrap_err();
        assert!(matches!(err, CoreError::Agent(_)));
        let store = store.lock().unwrap();
        assert_eq!(store.get_session(&sid).unwrap().status, SessionStatus::Failed);
        let usage_rows = store.list_llm_usage_for_session(&sid).unwrap();
        assert_eq!(usage_rows.len(), 1, "cost is logged even on failure (R9)");
        // agent pass (50) + summary call that skipped the tool (50) — the
        // failed summary call still cost tokens and they must be counted
        assert_eq!(usage_rows[0].input_tokens, 100);
        assert_eq!(usage_rows[0].output_tokens, 20);
    }

    #[tokio::test]
    async fn retry_after_failure_does_not_duplicate_outputs() {
        let (processor, store, sid) = processor_with(vec![
            // attempt 1: extracts one item, then summary fails
            tool_use("add_item", serde_json::json!({"kind": "todo", "text": "order lumber"})),
            end_turn("done"),
            end_turn("no summary tool"),
            // attempt 2: extracts the same item again, summary succeeds
            tool_use("add_item", serde_json::json!({"kind": "todo", "text": "order lumber"})),
            end_turn("done"),
            summary_response("Lumber ordered."),
        ]);
        assert!(processor.process(&sid).await.is_err());
        processor.process(&sid).await.unwrap();
        let store = store.lock().unwrap();
        assert_eq!(
            store.list_items_for_session(&sid).unwrap().len(),
            1,
            "attempt 1's item was cleared before retry"
        );
    }

    #[tokio::test]
    async fn live_item_survives_a_failed_process_then_is_swapped_on_retry() {
        use crate::domain::ItemSource;
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        store.add_item_with_source(&session.id, "todo", "live capture", ItemSource::Live).unwrap();
        store.append_transcript(&session.id, "order the framing lumber today").unwrap();
        store.end_and_record_session(&session.id).unwrap();
        let sid = session.id.clone();
        let store = Arc::new(Mutex::new(store));
        // attempt 1 fails (summary returns no tool); attempt 2 succeeds.
        let processor = SessionProcessor::new(
            Arc::new(MockProvider::new(vec![
                end_turn("no extraction"), end_turn("no summary tool"),
                tool_use("add_item", serde_json::json!({"kind":"todo","text":"order 12 2x10s"})),
                end_turn("done"),
                summary_response("Lumber ordered."),
            ])),
            store.clone(), Arc::new(Mutex::new(Memory::default())), Arc::new(NullMemoryStore),
        );
        assert!(processor.process(&sid).await.is_err());
        // R7: the live board survived the failure.
        assert_eq!(store.lock().unwrap().list_items_for_session(&sid).unwrap().len(), 1);
        processor.process(&sid).await.unwrap();
        // swap: live capture gone, exactly the authoritative item remains.
        let items = store.lock().unwrap().list_items_for_session(&sid).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text, "order 12 2x10s");
        assert_eq!(items[0].source, ItemSource::Authoritative);
    }

    /// REQUIRED (review): repeated FAILED retries must not accumulate duplicate
    /// authoritative todos. The Phase-0 scoped clear bounds them to one in-flight
    /// attempt while the live board (safety net) and manual items persist.
    #[tokio::test]
    async fn repeated_failed_retries_do_not_accumulate_authoritative_dupes() {
        use crate::domain::ItemSource;
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        let sid = session.id.clone();
        store.add_item_with_source(&sid, "todo", "live capture", ItemSource::Live).unwrap();
        store.add_item_with_source(&sid, "note", "manual note", ItemSource::Manual).unwrap();
        store.append_transcript(&sid, "order the framing lumber today").unwrap();
        store.end_and_record_session(&sid).unwrap();
        let store = Arc::new(Mutex::new(store));

        // Two attempts that extract one authoritative item then fail on summary,
        // then a success.
        let processor = SessionProcessor::new(
            Arc::new(MockProvider::new(vec![
                tool_use("add_item", serde_json::json!({"kind":"todo","text":"order lumber"})),
                end_turn("done"), end_turn("no summary tool"),          // attempt 1 fails
                tool_use("add_item", serde_json::json!({"kind":"todo","text":"order lumber"})),
                end_turn("done"), end_turn("no summary tool"),          // attempt 2 fails
                tool_use("add_item", serde_json::json!({"kind":"todo","text":"order 12 2x10s"})),
                end_turn("done"), summary_response("Lumber ordered."),            ])),
            store.clone(), Arc::new(Mutex::new(Memory::default())), Arc::new(NullMemoryStore),
        );

        let auth = |s: &Store| s.list_items_for_session(&sid).unwrap()
            .into_iter().filter(|i| i.source == ItemSource::Authoritative).count();
        let live = |s: &Store| s.list_items_for_session(&sid).unwrap()
            .into_iter().any(|i| i.source == ItemSource::Live);

        assert!(processor.process(&sid).await.is_err());
        { let s = store.lock().unwrap();
          assert!(live(&s), "live board survives failure #1");
          assert_eq!(auth(&s), 1, "one attempt's authoritative items after failure #1"); }

        assert!(processor.process(&sid).await.is_err());
        { let s = store.lock().unwrap();
          assert!(live(&s), "live board survives failure #2");
          assert_eq!(auth(&s), 1, "still one attempt's worth — the entry clear bounds dupes"); }

        processor.process(&sid).await.unwrap();
        let s = store.lock().unwrap();
        let items = s.list_items_for_session(&sid).unwrap();
        assert_eq!(items.len(), 2, "exactly this-run authoritative + the manual entry");
        assert!(items.iter().any(|i| i.text == "order 12 2x10s" && i.source == ItemSource::Authoritative));
        assert!(items.iter().any(|i| i.text == "manual note" && i.source == ItemSource::Manual));
        assert!(!items.iter().any(|i| i.source == ItemSource::Live), "live board swapped out on success");
    }

    #[tokio::test]
    async fn recording_session_is_rejected() {
        let (processor, store, _sid) = processor_with(vec![]);
        let recording = store.lock().unwrap().start_session(None).unwrap();
        let err = processor.process(&recording.id).await.unwrap_err();
        assert!(matches!(err, CoreError::InvalidState(_)));
    }

    #[tokio::test]
    async fn memory_reaches_the_system_prompt() {
        let provider = Arc::new(MockProvider::new(vec![
            end_turn("done"),
            summary_response("s"),
        ]));
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        store.append_transcript(&session.id, "talk about the french drain").unwrap();
        store.end_and_record_session(&session.id).unwrap();
        let mut memory = Memory::default();
        memory.remember_from("vocabulary", "french drain", 1, FactSource::Stated, None);
        let processor = SessionProcessor::new(
            provider.clone(),
            Arc::new(Mutex::new(store)),
            Arc::new(Mutex::new(memory)),
            Arc::new(NullMemoryStore),
        );
        processor.process(&session.id).await.unwrap();
        let reqs = provider.requests();
        assert!(reqs[0].system.contains("french drain"));
    }

    #[tokio::test]
    async fn unknown_session_is_not_found() {
        let (processor, _store, _sid) = processor_with(vec![]);
        let err = processor.process("no-such-session").await.unwrap_err();
        assert!(matches!(err, CoreError::NotFound { entity: "session", .. }));
    }

    /// MaxTurns burns real tokens and those tokens must appear in the usage row
    /// even though the agent never returned a successful TurnOutcome (R9).
    #[tokio::test]
    async fn max_turns_logs_partial_usage() {
        // agent pass: one tool_use response with real usage, then MaxTurns fires
        // max_turns = 1 so the loop fires after the first tool_use response
        let (mut processor, store, sid) = processor_with(vec![
            tool_use("add_item", serde_json::json!({"kind": "todo", "text": "order lumber"})),
        ]);
        processor.max_turns = 1;

        let err = processor.process(&sid).await.unwrap_err();
        assert!(matches!(err, CoreError::Agent(harness::HarnessError::MaxTurns(1))));

        let store = store.lock().unwrap();
        assert_eq!(store.get_session(&sid).unwrap().status, SessionStatus::Failed);
        let usage_rows = store.list_llm_usage_for_session(&sid).unwrap();
        assert_eq!(usage_rows.len(), 1, "usage logged even when agent hits MaxTurns");
        // exact tokens from the one scripted tool_use response
        assert_eq!(usage_rows[0].input_tokens, 100);
        assert_eq!(usage_rows[0].output_tokens, 20);
    }

    /// A failed session stays Failed and process_pending does NOT re-pull it on
    /// a second call — only AwaitingProcessing sessions are drained.
    #[tokio::test]
    async fn process_pending_does_not_retry_failed_sessions() {
        let (processor, store, sid) = processor_with(vec![
            end_turn("nothing to extract"),
            end_turn("no summary tool"), // summary call returns no tool → Provider error
        ]);
        // First drain: session goes Failed
        let results1 = processor.process_pending().await.unwrap();
        assert_eq!(results1.len(), 1);
        assert!(results1[0].1.is_err());
        assert_eq!(
            store.lock().unwrap().get_session(&sid).unwrap().status,
            SessionStatus::Failed
        );

        // Second drain: nothing in AwaitingProcessing → empty
        let results2 = processor.process_pending().await.unwrap();
        assert!(
            results2.is_empty(),
            "failed session must not be re-pulled by a second process_pending call"
        );
    }

    /// The offline-banner promise: a `Failed` session (offline / LLM error)
    /// is picked back up and can reach `Processed` — without `process_pending`
    /// having to weaken its own no-retry pin (that test above stays as-is).
    #[tokio::test]
    async fn retry_failed_sessions_recovers_a_failed_session() {
        let (processor, store, sid) = processor_with(vec![
            end_turn("nothing to extract"),
            end_turn("no summary tool"), // summary call returns no tool → Provider error
        ]);
        let results = processor.process_pending().await.unwrap();
        assert!(results[0].1.is_err());
        assert_eq!(
            store.lock().unwrap().get_session(&sid).unwrap().status,
            SessionStatus::Failed
        );

        // Reconnect: this time the LLM cooperates.
        let retry_processor = SessionProcessor::new(
            Arc::new(MockProvider::new(vec![
                end_turn("nothing to extract"),
                summary_response("recovered on retry"),
            ])),
            store.clone(),
            Arc::new(Mutex::new(Memory::default())),
            Arc::new(NullMemoryStore),
        );
        let retried = retry_processor.retry_failed_sessions().await.unwrap();
        assert_eq!(retried.len(), 1);
        assert_eq!(retried[0].0, sid);
        assert!(retried[0].1.is_ok());
        assert_eq!(
            store.lock().unwrap().get_session(&sid).unwrap().status,
            SessionStatus::Processed
        );

        // Nothing left in Failed → a second call is a no-op.
        let again = retry_processor.retry_failed_sessions().await.unwrap();
        assert!(again.is_empty());
    }

    /// A still-offline retry makes exactly ONE attempt per app-open — it does
    /// not loop, and the session stays Failed (not silently dropped) for the
    /// next app-open to try again.
    #[tokio::test]
    async fn retry_failed_sessions_leaves_a_still_failing_session_failed() {
        let (processor, store, sid) = processor_with(vec![
            end_turn("nothing to extract"),
            end_turn("no summary tool"),
        ]);
        processor.process_pending().await.unwrap();
        assert_eq!(
            store.lock().unwrap().get_session(&sid).unwrap().status,
            SessionStatus::Failed
        );

        let retry_processor = SessionProcessor::new(
            Arc::new(MockProvider::new(vec![
                end_turn("nothing to extract"),
                end_turn("still no summary tool"),
            ])),
            store.clone(),
            Arc::new(Mutex::new(Memory::default())),
            Arc::new(NullMemoryStore),
        );
        let results = retry_processor.retry_failed_sessions().await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].1.is_err());
        assert_eq!(
            store.lock().unwrap().get_session(&sid).unwrap().status,
            SessionStatus::Failed,
            "a failed retry leaves the session Failed, not lost"
        );
    }

    /// App-open latency + R9 (cost is measured, not free): a reconnect
    /// backlog is capped per call, oldest-first — the longest-waiting walk
    /// gets first crack, and a huge backlog can't turn one app-open into a
    /// dozen LLM calls.
    #[tokio::test]
    async fn retry_failed_sessions_caps_at_five_oldest_first() {
        let counter = Arc::new(std::sync::atomic::AtomicU64::new(1000));
        let clock_counter = counter.clone();
        let store = Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(move || {
            clock_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        }));
        let mut ids = Vec::new();
        for _ in 0..6 {
            let session = store.start_session(None).unwrap();
            store.append_transcript(&session.id, "we need lumber").unwrap();
            store.end_and_record_session(&session.id).unwrap();
            store.mark_session_failed(&session.id).unwrap();
            ids.push(session.id);
        }
        let store = Arc::new(Mutex::new(store));

        let mut responses = Vec::new();
        for _ in 0..5 {
            responses.push(end_turn("nothing to extract"));
            responses.push(summary_response("recovered"));
        }
        let processor = SessionProcessor::new(
            Arc::new(MockProvider::new(responses)),
            store.clone(),
            Arc::new(Mutex::new(Memory::default())),
            Arc::new(NullMemoryStore),
        );
        let results = processor.retry_failed_sessions().await.unwrap();
        assert_eq!(results.len(), 5, "capped at 5 per call");
        let retried_ids: Vec<_> = results.iter().map(|(id, _)| id.clone()).collect();
        assert_eq!(retried_ids, ids[0..5], "oldest five retried, oldest first");
        assert!(results.iter().all(|(_, r)| r.is_ok()));

        let locked = store.lock().unwrap();
        for id in &ids[0..5] {
            assert_eq!(locked.get_session(id).unwrap().status, SessionStatus::Processed);
        }
        assert_eq!(
            locked.get_session(&ids[5]).unwrap().status,
            SessionStatus::Failed,
            "the newest session is left for a future app-open, untouched by this call's cap"
        );
    }

    /// Crash-recovery for free: `sweep_zombie_sessions` marks a crash-orphaned
    /// `Recording` session `Failed`; a retry run afterward (same app-open)
    /// picks it up like any other Failed session.
    #[tokio::test]
    async fn retry_failed_sessions_recovers_a_zombie_swept_session() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        store.append_transcript(&session.id, "we need lumber").unwrap();
        // No end_and_record_session: this session is still Recording, i.e. a
        // crash left the app mid-walk.
        let swept = store.sweep_zombie_sessions().unwrap();
        assert_eq!(swept, 1);
        assert_eq!(
            store.get_session(&session.id).unwrap().status,
            SessionStatus::Failed
        );

        let store = Arc::new(Mutex::new(store));
        let processor = SessionProcessor::new(
            Arc::new(MockProvider::new(vec![
                end_turn("nothing to extract"),
                summary_response("recovered from a crash"),
            ])),
            store.clone(),
            Arc::new(Mutex::new(Memory::default())),
            Arc::new(NullMemoryStore),
        );
        let results = processor.retry_failed_sessions().await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].1.is_ok());
        assert_eq!(
            store.lock().unwrap().get_session(&session.id).unwrap().status,
            SessionStatus::Processed
        );
    }

    /// The empty-session contract holds for a zombie-swept session too — an
    /// interrupted walk with no captured transcript never panics or reaches
    /// the LLM; it resolves to Processed with the placeholder summary.
    #[tokio::test]
    async fn retry_failed_sessions_empty_zombie_session_never_panics() {
        let store = Store::open_in_memory("device-a").unwrap();
        let _session = store.start_session(None).unwrap(); // no transcript appended
        let swept = store.sweep_zombie_sessions().unwrap();
        assert_eq!(swept, 1);

        let store = Arc::new(Mutex::new(store));
        let processor = SessionProcessor::new(
            Arc::new(MockProvider::new(vec![])),
            store.clone(),
            Arc::new(Mutex::new(Memory::default())),
            Arc::new(NullMemoryStore),
        );
        let results = processor.retry_failed_sessions().await.unwrap();
        assert_eq!(results.len(), 1);
        let outcome = results[0].1.as_ref().unwrap();
        assert_eq!(outcome.session.status, SessionStatus::Processed);
        assert_eq!(outcome.session.summary.as_deref(), Some("(empty session)"));
    }

    /// Empty (or whitespace-only) transcripts never reach the LLM — the real
    /// Anthropic API rejects empty content blocks. The session is processed
    /// directly with a placeholder summary and zero usage.
    #[tokio::test]
    async fn empty_transcript_skips_llm_and_processes_with_placeholder() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        store.end_and_record_session(&session.id).unwrap();
        let provider = Arc::new(MockProvider::new(vec![]));
        let processor = SessionProcessor::new(
            provider.clone(),
            Arc::new(Mutex::new(store)),
            Arc::new(Mutex::new(Memory::default())),
            Arc::new(NullMemoryStore),
        );
        let outcome = processor.process(&session.id).await.unwrap();
        assert_eq!(outcome.session.status, SessionStatus::Processed);
        assert_eq!(outcome.session.summary.as_deref(), Some("(empty session)"));
        assert_eq!(outcome.usage, Usage::default());
        assert!(provider.requests().is_empty(), "no LLM calls for an empty session");
    }

    #[tokio::test]
    async fn whitespace_only_transcript_also_skips_llm() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        store.append_transcript(&session.id, "  \n\t  ").unwrap();
        store.end_and_record_session(&session.id).unwrap();
        let provider = Arc::new(MockProvider::new(vec![]));
        let processor = SessionProcessor::new(
            provider.clone(),
            Arc::new(Mutex::new(store)),
            Arc::new(Mutex::new(Memory::default())),
            Arc::new(NullMemoryStore),
        );
        let outcome = processor.process(&session.id).await.unwrap();
        assert_eq!(outcome.session.summary.as_deref(), Some("(empty session)"));
        assert!(provider.requests().is_empty());
    }

    #[tokio::test]
    async fn process_pending_on_empty_queue_is_ok_and_empty() {
        let processor = SessionProcessor::new(
            Arc::new(MockProvider::new(vec![])),
            Arc::new(Mutex::new(Store::open_in_memory("device-a").unwrap())),
            Arc::new(Mutex::new(Memory::default())),
            Arc::new(NullMemoryStore),
        );
        assert!(processor.process_pending().await.unwrap().is_empty());
    }

    /// Plan 13 Task 1: the legal `doc_kind` vocabulary + pricing flags per
    /// template.
    #[test]
    fn doc_kinds_for_template_lists_the_legal_kinds_per_template() {
        assert_eq!(
            doc_kinds_for_template(Some("landscape")),
            &["estimate", "invoice", "work_order"]
        );
        assert_eq!(doc_kinds_for_template(Some("property")), &["condition", "move_out"]);
        assert_eq!(doc_kinds_for_template(Some("inspection")), &["inspection"]);
        assert_eq!(doc_kinds_for_template(None), &["report"]);
    }

    #[test]
    fn is_pricing_kind_flags_only_estimate_and_invoice() {
        assert!(is_pricing_kind("estimate"));
        assert!(is_pricing_kind("invoice"));
        assert!(!is_pricing_kind("work_order"));
        assert!(!is_pricing_kind("inspection"));
        assert!(!is_pricing_kind("report"));
        assert!(!is_pricing_kind("condition"));
    }

    /// Plan 13 N3 (Stage 2 flip): `doc_kind_for_template` is now
    /// `doc_kinds_for_template(t)[0]` — property's default flips from
    /// `"report"` to `"condition"` (property's own legal-kind list starts
    /// with `condition`, not `report`). `doc_kind` is advisory-only
    /// (`NotesPayload.doc_kind`); Swift's button wiring keys off the
    /// client-known template, never off this value.
    #[test]
    fn doc_kind_for_template_is_the_templates_first_legal_kind() {
        assert_eq!(doc_kind_for_template(Some("landscape")), "estimate");
        assert_eq!(doc_kind_for_template(Some("inspection")), "inspection");
        assert_eq!(doc_kind_for_template(None), "report");
        assert_eq!(
            doc_kind_for_template(Some("property")),
            "condition",
            "N3: property's default flips to condition in Stage 2"
        );
    }

    #[tokio::test]
    async fn process_pending_drains_the_queue_and_survives_failures() {
        let store = Store::open_in_memory("device-a").unwrap();
        let a = store.start_session(None).unwrap();
        store.append_transcript(&a.id, "session a words").unwrap();
        store.end_and_record_session(&a.id).unwrap();
        let b = store.start_session(None).unwrap();
        store.append_transcript(&b.id, "session b words").unwrap();
        store.end_and_record_session(&b.id).unwrap();
        // still recording — must be untouched
        let c = store.start_session(None).unwrap();

        // queue order is reverse-chron (b first): b succeeds, a fails on summary
        let processor = SessionProcessor::new(
            Arc::new(MockProvider::new(vec![
                end_turn("done b"),
                summary_response("B done."),
                end_turn("done a"),
                end_turn("no summary tool"),
            ])),
            Arc::new(Mutex::new(store)),
            Arc::new(Mutex::new(Memory::default())),
            Arc::new(NullMemoryStore),
        );

        let results = processor.process_pending().await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|(id, r)| id == &b.id && r.is_ok()));
        assert!(results.iter().any(|(id, r)| id == &a.id && r.is_err()));

        let store = processor.store.lock().unwrap();
        assert_eq!(store.get_session(&b.id).unwrap().status, SessionStatus::Processed);
        assert_eq!(store.get_session(&a.id).unwrap().status, SessionStatus::Failed);
        assert_eq!(store.get_session(&c.id).unwrap().status, SessionStatus::Recording);
    }

    /// D5a: a session whose transcript states a total captures a
    /// `session_meta` artifact carrying `spoken_total_cents` — written by
    /// `process()` on success, BEFORE the finish swap.
    #[tokio::test]
    async fn process_captures_the_spoken_total_as_a_session_meta_artifact() {
        let (processor, store, sid) = processor_with(vec![
            end_turn("nothing to extract"),
            tool_use(
                "write_notes",
                serde_json::json!({
                    "summary": "Mulch and railing; keep it under twelve hundred.",
                    "spoken_total_cents": 120000
                }),
            ),
        ]);
        processor.process(&sid).await.unwrap();
        let store = store.lock().unwrap();
        let artifacts = store.list_artifacts_for_session(&sid).unwrap();
        let meta = artifacts.iter().find(|a| a.kind == "session_meta").expect("session_meta written");
        let v: serde_json::Value = serde_json::from_str(&meta.body).unwrap();
        assert_eq!(v["spoken_total_cents"], 120000);
    }

    /// No total stated -> no `session_meta` artifact at all (D5a: absent
    /// unless the model clearly heard a stated total, R6).
    #[tokio::test]
    async fn process_writes_no_session_meta_artifact_when_no_total_was_stated() {
        let (processor, store, sid) = processor_with(vec![
            end_turn("nothing to extract"),
            summary_response("Walked the site."),
        ]);
        processor.process(&sid).await.unwrap();
        let store = store.lock().unwrap();
        let artifacts = store.list_artifacts_for_session(&sid).unwrap();
        assert!(!artifacts.iter().any(|a| a.kind == "session_meta"), "no total stated -> no artifact");
    }

    /// Plan 14 D5-14/Task 3: a `write_notes` response carrying buckets
    /// persists as a `kind="notes"` artifact, parseable back to the same
    /// entries.
    #[tokio::test]
    async fn process_persists_notes_buckets_as_an_artifact() {
        let (processor, store, sid) = processor_with(vec![
            end_turn("nothing to extract"),
            tool_use(
                "write_notes",
                serde_json::json!({
                    "summary": "Estimate walk on the front yard.",
                    "notes": [
                        {"bucket": "scope_of_work", "label": "Mulch", "detail": "Darker than last year."},
                        {"bucket": "constraints", "label": "Budget", "detail": "Under $1,200."}
                    ]
                }),
            ),
        ]);
        processor.process(&sid).await.unwrap();
        let store = store.lock().unwrap();
        let artifacts = store.list_artifacts_for_session(&sid).unwrap();
        let notes = artifacts.iter().find(|a| a.kind == "notes").expect("notes artifact written");
        let parsed = notes::parse_notes_artifact(&notes.body);
        assert_eq!(
            parsed,
            vec![
                notes::NotesEntry {
                    bucket: "scope_of_work".into(),
                    label: "Mulch".into(),
                    detail: "Darker than last year.".into(),
                },
                notes::NotesEntry {
                    bucket: "constraints".into(),
                    label: "Budget".into(),
                    detail: "Under $1,200.".into(),
                },
            ]
        );
    }

    /// No buckets in the response -> no `notes` artifact at all (mirrors the
    /// `session_meta` absence test).
    #[tokio::test]
    async fn process_writes_no_notes_artifact_when_no_buckets_were_returned() {
        let (processor, store, sid) = processor_with(vec![
            end_turn("nothing to extract"),
            summary_response("Walked the site."),
        ]);
        processor.process(&sid).await.unwrap();
        let store = store.lock().unwrap();
        let artifacts = store.list_artifacts_for_session(&sid).unwrap();
        assert!(!artifacts.iter().any(|a| a.kind == "notes"), "no buckets -> no notes artifact");
    }

    /// R6 pin (D3-14): buckets never become item rows. The extraction agent
    /// created exactly one item (`add_item`); the notes pass's two buckets
    /// must add zero rows on top of that.
    #[tokio::test]
    async fn process_notes_pass_never_adds_item_rows() {
        let (processor, store, sid) = processor_with(vec![
            tool_use("add_item", serde_json::json!({"kind": "todo", "text": "order lumber"})),
            end_turn("done"),
            tool_use(
                "write_notes",
                serde_json::json!({
                    "summary": "Estimate walk.",
                    "notes": [
                        {"bucket": "scope_of_work", "label": "Mulch", "detail": "Darker."},
                        {"bucket": "constraints", "label": "Budget", "detail": "Under $1,200."}
                    ]
                }),
            ),
        ]);
        processor.process(&sid).await.unwrap();
        let store = store.lock().unwrap();
        assert_eq!(
            store.list_items_for_session(&sid).unwrap().len(),
            1,
            "buckets must never become board items"
        );
    }
}
