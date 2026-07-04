# Murmur Rust Core — Plan 05: Live In-Session Extraction

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add incremental agent passes that run *during* recording and lift clearly-stated items onto a live "Captured" board as they're spoken (spec Rev 2 §2). End-of-session `process()` (Plan 04) stays the source of truth: at process() start it tombstones every live item and re-creates the authoritative set — the live board **swaps** when processing lands. Live passes are add-item-only, use a *cheaper* provider, are billed to the session (R9, purpose `"live_extraction"`), and never disrupt recording.

**Architecture:** One new type, `LiveExtractor`, in `crates/murmur-core/src/pipeline/live.rs` at `/Users/claude/murmur-rmp`. It is **purely additive** — no new store method, no migration, no domain change. It reuses the exact seams Plan 04 built: `AddItemTool` (the only tool it registers), `Store::add_item` / `list_items_for_session` / `get_session` / `record_llm_usage`, the harness `Agent` loop, and `ContextAssembler`. Design decisions, justified:

1. **Same `items` table, tombstoned at process().** Live items are ordinary `items` rows. `Store::clear_session_outputs` (Plan 04, `sessions.rs`) already tombstones a session's items+artifacts at the top of `process()`, and `list_items_for_session` already filters `deleted_at IS NULL`. So the vanish/reappear contract needs zero new storage code — a separate ephemeral surface would duplicate the board query, the tombstone cascade, and the manual-parity story (story 10) for no gain. The UI re-queries `list_items_for_session` on the session's status change (Recording → Processed), which is the simplest correct swap.
2. **The status gate is the serialization boundary.** `maybe_extract` runs ONLY while the session is `Recording`; `SessionProcessor::process` accepts only `AwaitingProcessing | Failed`. Live passes and end-of-session processing are temporally disjoint *by construction*. A pass that finds the session no longer `Recording` skips silently (`LiveExtractOutcome::Skipped`) — this also handles the stop-button race (the app ticks the extractor just as the user ends the session).
3. **Cursor lives in-memory on the extractor.** A `usize` char offset marks how much transcript a *successful* pass has covered. Not persisted: a crashed app just re-extracts from 0 on the next `LiveExtractor`, or waits for end-of-session truth. Persisting would add a migration for marginal value (the board is disposable — process() rebuilds it).
4. **Add-item-only tool surface.** Reports (`write_report`), contacts (`upsert_contact`), and memory (`update_memory`) are end-of-session/reflection concerns. A single-tool registry keeps live calls cheap and safe (R6 doubly: partial transcript → even more conservative than Plan 04's extraction pass).
5. **Delta-aware context, kept simple (YAGNI).** Each pass sends only the *new* transcript slice (`transcript[cursor..]`, budgeted) plus a newest-first "already captured" list (the session's existing item texts, budgeted so the oldest is cut first). The already-captured list is the dedup mechanism — no cross-request de-dup bookkeeping. A reference that spans the cursor boundary might be missed live; that's fine, `process()` is the source of truth.
6. **Failure never disrupts recording.** A failed pass is swallowed: non-zero usage is logged (R9), the cursor is **not** advanced (next tick retries the same window), and `LiveExtractOutcome::Failed` is returned as `Ok` — `Err` is reserved for genuine store faults (lock poisoned, session vanished). Any items a failed pass already wrote stay on the board and are de-duplicated by the already-captured list on retry.
7. **`&mut self`, not interior mutability.** The cursor is per-extractor mutable state. `maybe_extract(&mut self)` makes "sequential calls only" a compile-time guarantee instead of a runtime hazard — one extractor per recording session, ticked by the app shell. (Plan 07 FFI can wrap it if it needs `&self`.) The **trigger** — every N seconds / on pause detection — stays app-shell policy; core's only gate is `min_new_chars` (a cheap floor, not the cadence).

**Tech Stack:** existing deps only — no new dependencies. Reuses harness (`Agent`, `AgentConfig`, `ContextAssembler`, `ContextSection`, `LlmProvider`, `Memory`, `Message`, `ToolRegistry`, `Usage`) and the Plan 04 `AddItemTool` + prompts module. All tests hermetic — `MockProvider` only, no network.

**Spec:** vision spec Rev 2 §2 (live in-session extraction, offline-degradable, end-of-session pass is truth), §6 (<8s transformation budget context), R6 (under-extraction bias — doubly for partial transcripts), R7 (inspectable outcomes — live items are real store rows), R9 (cost per call measured and logged — live passes session-tagged, purpose `"live_extraction"`). Plan 04 Deferred item 1 (live extraction scope) and the "status gate is the serialization boundary" review note.

---

## File Structure

```
crates/murmur-core/
  src/
    pipeline/
      prompts.rs   # MODIFY: live_extraction_system_prompt, format_already_captured
      live.rs      # NEW: LiveExtractor, LiveExtractOutcome, maybe_extract
      mod.rs       # MODIFY: pub mod live;
    lib.rs         # MODIFY: re-export LiveExtractor, LiveExtractOutcome
  tests/
    live_extraction_e2e.rs   # NEW: live-board → process() swap contract
README.md          # MODIFY: plan-series line
```

Run cargo via the dev shell or `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo <cmd>` from the repo root. No migration is touched — Plan 05 adds no columns or tables.

---

### Task 1: Live-extraction prompt + already-captured dedup list

**Files:**
- Modify: `src/pipeline/prompts.rs`

- [ ] **Step 1: Write the failing tests** (add to the existing tests module in `src/pipeline/prompts.rs`)

```rust
    #[test]
    fn live_prompt_is_conservative_and_add_item_only() {
        let p = live_extraction_system_prompt("## vocabulary\n- french drain\n");
        assert!(p.contains("already captured"), "dedup instruction");
        assert!(p.contains("partial transcript"), "names the partial-transcript risk");
        assert!(p.contains("add_item is your only tool"));
        assert!(p.contains("Bias hard toward fewer items"), "R6 doubly");
        assert!(p.contains("french drain"), "memory is injected");
        // live passes must NOT be told to write reports or save contacts
        assert!(p.contains("do not summarize, write reports, or save"));
    }

    #[test]
    fn live_prompt_without_memory_omits_the_block() {
        let p = live_extraction_system_prompt("");
        assert!(!p.contains("What you know about this user"));
    }

    #[test]
    fn already_captured_is_newest_first_and_tagged() {
        let s = crate::store::Store::open_in_memory("device-a").unwrap();
        let session = s.start_session(None).unwrap();
        s.add_item(&session.id, "todo", "order lumber").unwrap();
        s.add_item(&session.id, "safety", "loose railing").unwrap();
        let items = s.list_items_for_session(&session.id).unwrap();
        let rendered = format_already_captured(&items);
        // newest first: safety before todo
        assert_eq!(rendered, "- [safety] loose railing\n- [todo] order lumber");
    }

    #[test]
    fn already_captured_is_empty_for_no_items() {
        assert_eq!(format_already_captured(&[]), "");
    }
```

- [ ] **Step 2: Run to see failure**

Run: `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo test -p murmur-core prompts`
Expected: compile FAIL (functions don't exist).

- [ ] **Step 3: Implement** (in `src/pipeline/prompts.rs`)

Add the domain import at the top of the file, alongside the existing `use harness::{...}`:
```rust
use crate::domain::CapturedItem;
```

Add these two functions above the `#[cfg(test)]` module:
```rust
/// Formats a session's existing items as a newest-first dedup list for a live
/// pass. Newest-first so budget truncation drops the *oldest* entries (least
/// likely to be re-mentioned in the newest transcript slice). Empty string when
/// there are no items — the context assembler elides empty sections.
pub(crate) fn format_already_captured(items: &[CapturedItem]) -> String {
    items
        .iter()
        .rev()
        .map(|i| format!("- [{}] {}", i.kind, i.text))
        .collect::<Vec<_>>()
        .join("\n")
}

/// System prompt for a live in-session pass (spec Rev 2 §2). Even more
/// conservative than `extraction_system_prompt`: the transcript is partial, so
/// R6's under-extraction bias applies doubly. `add_item` is the only tool —
/// reports, contacts, and memory are end-of-session concerns. `memory_prompt`
/// is `Memory::to_prompt()` output ("" when empty).
pub(crate) fn live_extraction_system_prompt(memory_prompt: &str) -> String {
    let memory_block = if memory_prompt.trim().is_empty() {
        String::new()
    } else {
        format!("\n\nWhat you know about this user:\n{memory_prompt}")
    };
    format!(
        "You extract items LIVE from an in-progress field-work session while the \
         tradesperson is still talking. You see only the newest slice of a running \
         transcript plus the items already captured so far.\n\
         Rules:\n\
         - Extract ONLY clearly-completed, unambiguous items with add_item (todos, \
         decisions, notes, safety issues, parts, prices). This is a partial \
         transcript: when a thought is mid-sentence, cut off, or unclear, SKIP it — \
         the end-of-session pass is the source of truth and will catch it. Bias hard \
         toward fewer items.\n\
         - NEVER repeat anything under 'already captured'. When unsure whether it is \
         a duplicate, skip it.\n\
         - Never invent assignees, prices, dates, or details that were not spoken.\n\
         - add_item is your only tool — do not summarize, write reports, or save \
         contacts. When nothing new is worth capturing, reply with a short \
         acknowledgement and call no tools.\n\
         - Transcripts are speech-to-text: expect misrecognized jargon and names; \
         prefer terms from what you know about the user.{memory_block}"
    )
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo test -p murmur-core prompts`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): live-extraction prompt (R6 doubly) and already-captured dedup list"
```

---

### Task 2: LiveExtractor — construction, gates, happy path

**Files:**
- Create: `src/pipeline/live.rs`
- Modify: `src/pipeline/mod.rs`, `src/lib.rs`

- [ ] **Step 1: Write the failing tests** (bottom of `src/pipeline/live.rs`)

```rust
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use harness::{
        CompletionResponse, ContentBlock, FactSource, Memory, MockProvider, StopReason, Usage,
    };

    use crate::domain::SessionStatus;
    use crate::store::Store;

    use super::*;

    // No NullMemoryStore stub here: unlike SessionProcessor, LiveExtractor takes
    // Arc<Mutex<Memory>> directly (no MemoryStore param) — a stub would be
    // dead code and fail Task 4's zero-clippy-warning gate.

    fn tool_use(name: &str, input: serde_json::Value) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::ToolUse { id: "tu_1".into(), name: name.into(), input }],
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 30, output_tokens: 8 },
        }
    }

    fn end_turn(text: &str) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::Text { text: text.into() }],
            stop_reason: StopReason::EndTurn,
            usage: Usage { input_tokens: 10, output_tokens: 2 },
        }
    }

    /// Recording session with `transcript`, plus a shared store and memory.
    /// `min_new_chars` is dropped to 1 so short test transcripts trigger.
    fn extractor_with(
        responses: Vec<CompletionResponse>,
        transcript: &str,
    ) -> (LiveExtractor, Arc<Mutex<Store>>, Arc<Mutex<Memory>>, String) {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        if !transcript.is_empty() {
            store.append_transcript(&session.id, transcript).unwrap();
        }
        let sid = session.id;
        let store = Arc::new(Mutex::new(store));
        let memory = Arc::new(Mutex::new(Memory::default()));
        let mut extractor = LiveExtractor::new(
            Arc::new(MockProvider::new(responses)),
            store.clone(),
            memory.clone(),
            &sid,
        );
        extractor.min_new_chars = 1;
        (extractor, store, memory, sid)
    }

    #[tokio::test]
    async fn extracts_items_and_advances_cursor() {
        let (mut extractor, store, _mem, sid) = extractor_with(
            vec![
                tool_use("add_item", serde_json::json!({"kind": "todo", "text": "order lumber"})),
                end_turn("captured"),
            ],
            "we need to order lumber for the deck",
        );
        assert_eq!(extractor.cursor(), 0);
        let outcome = extractor.maybe_extract().await.unwrap();
        assert_eq!(
            outcome,
            LiveExtractOutcome::Extracted {
                items_added: 1,
                usage: Usage { input_tokens: 40, output_tokens: 10 },
            }
        );
        // cursor advanced to the transcript length in chars
        assert_eq!(extractor.cursor(), "we need to order lumber for the deck".chars().count());

        let store = store.lock().unwrap();
        let items = store.list_items_for_session(&sid).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "todo");
        // R9: the live pass is billed to the session under its own purpose
        let usage = store.list_llm_usage_for_session(&sid).unwrap();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].purpose, "live_extraction");
        assert_eq!(usage[0].input_tokens, 40);
    }

    #[tokio::test]
    async fn skips_when_too_little_new_transcript() {
        // default min_new_chars (120) with a 5-char transcript → no call
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        store.append_transcript(&session.id, "hi ok").unwrap();
        let sid = session.id;
        let store = Arc::new(Mutex::new(store));
        let provider = Arc::new(MockProvider::new(vec![]));
        let mut extractor = LiveExtractor::new(
            provider.clone(),
            store.clone(),
            Arc::new(Mutex::new(Memory::default())),
            &sid,
        );
        let outcome = extractor.maybe_extract().await.unwrap();
        assert_eq!(outcome, LiveExtractOutcome::Skipped);
        assert!(provider.requests().is_empty(), "no LLM call when below the floor");
        assert_eq!(extractor.cursor(), 0);
    }

    #[tokio::test]
    async fn skips_when_no_new_transcript_since_last_pass() {
        let (mut extractor, _store, _mem, _sid) = extractor_with(
            vec![
                tool_use("add_item", serde_json::json!({"kind": "todo", "text": "x"})),
                end_turn("ok"),
            ],
            "order lumber for the deck today",
        );
        assert!(matches!(extractor.maybe_extract().await.unwrap(), LiveExtractOutcome::Extracted { .. }));
        // no new transcript appended → second pass skips (and would panic on an
        // exhausted mock if it tried to call the provider)
        assert_eq!(extractor.maybe_extract().await.unwrap(), LiveExtractOutcome::Skipped);
    }

    #[tokio::test]
    async fn skips_when_not_recording() {
        // end the session first: process()'s domain, not the extractor's
        let (mut extractor, store, _mem, sid) =
            extractor_with(vec![], "a long enough transcript to clear the floor");
        store.lock().unwrap().end_and_record_session(&sid).unwrap();
        let outcome = extractor.maybe_extract().await.unwrap();
        assert_eq!(outcome, LiveExtractOutcome::Skipped, "not recording → no live pass");
        assert_eq!(extractor.cursor(), 0);
    }

    #[tokio::test]
    async fn memory_reaches_the_live_system_prompt() {
        let provider = Arc::new(MockProvider::new(vec![end_turn("nothing new")]));
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        store.append_transcript(&session.id, "talk about the french drain regrade").unwrap();
        let sid = session.id;
        let mut memory = Memory::default();
        memory.remember_from("vocabulary", "french drain", 1, FactSource::Stated, None);
        let mut extractor = LiveExtractor::new(
            provider.clone(),
            Arc::new(Mutex::new(store)),
            Arc::new(Mutex::new(memory)),
            &sid,
        );
        extractor.min_new_chars = 1;
        extractor.maybe_extract().await.unwrap();
        let reqs = provider.requests();
        assert!(reqs[0].system.contains("french drain"));
        // and the new transcript reached the user message
        assert!(matches!(
            &reqs[0].messages[0].content[0],
            ContentBlock::Text { text } if text.contains("french drain regrade")
        ));
    }
}
```

- [ ] **Step 2: Run to see failure**

Run: `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo test -p murmur-core live`
Expected: compile FAIL.

- [ ] **Step 3: Implement** (in `src/pipeline/live.rs`, above the tests)

```rust
//! Live in-session extraction (spec Rev 2 §2): while a session is *recording*,
//! cheap incremental agent passes lift clearly-stated items onto a live board
//! as they're spoken. End-of-session `process()` (Plan 04) stays the source of
//! truth — it tombstones every live item (`Store::clear_session_outputs`) and
//! re-creates the authoritative set. The live board therefore *swaps* when
//! processing lands; the UI re-queries `list_items_for_session` on the session's
//! status change (Recording → Processed).
//!
//! Serialization by construction: `maybe_extract` runs ONLY while the session is
//! `Recording`; `SessionProcessor::process` accepts only
//! `AwaitingProcessing | Failed`. The two are temporally disjoint — the status
//! gate is the boundary. A pass that finds the session no longer recording skips
//! silently (handles the stop-button race).
//!
//! Failure posture: a failed pass never disrupts recording. Non-zero usage is
//! logged (R9), the cursor is NOT advanced, and the next tick retries the same
//! window. Items a failed pass already wrote stay on the board and are
//! de-duplicated by the "already captured" list on the retry.

use std::sync::{Arc, Mutex};

use harness::{
    Agent, AgentConfig, ContextAssembler, ContextSection, LlmProvider, Memory, Message,
    ToolRegistry, Usage,
};

use crate::domain::SessionStatus;
use crate::error::CoreError;
use crate::store::Store;

use super::prompts;
use super::tools::AddItemTool;

/// Result of one live pass. The app shell re-queries the board regardless; this
/// tells it whether a pass ran and what it cost.
#[derive(Clone, Debug, PartialEq)]
pub enum LiveExtractOutcome {
    /// A pass ran. `items_added` is the net change in this session's live item
    /// count — a refresh hint, approximate under concurrent manual edits, not an
    /// authority. Cursor advanced.
    Extracted { items_added: usize, usage: Usage },
    /// No LLM call: too little new transcript since the last pass, or the session
    /// is no longer recording (stop-button race).
    Skipped,
    /// The pass failed and was swallowed to protect recording. Non-zero usage is
    /// logged; the cursor is unchanged so the next tick retries.
    Failed { usage: Usage },
}

/// Drives incremental extraction for ONE recording session. One instance per
/// session, ticked by the app shell (the cadence — every N seconds / on pause —
/// is app-shell policy). `&mut self` makes sequential-only calls a compile-time
/// guarantee: the in-memory cursor is never raced.
pub struct LiveExtractor {
    /// A *cheaper* provider than the end-of-session processor (Rev 2 §2: live
    /// passes optimize for cost; routing is the separate-provider seam).
    provider: Arc<dyn LlmProvider>,
    store: Arc<Mutex<Store>>,
    memory: Arc<Mutex<Memory>>,
    session_id: String,
    /// Chars of transcript covered by a *successful* pass (in-memory: a crash
    /// just re-extracts from 0 or waits for end-of-session truth — no migration).
    cursor: usize,
    /// Floor on new transcript before a pass is worth making. The *cadence* is
    /// app-shell policy; this only guards against passes on a few new chars.
    pub min_new_chars: usize,
    /// Budget for the new-transcript window (chars/4 ≈ tokens).
    pub transcript_window_tokens: usize,
    /// Budget for the already-captured dedup list (newest-first; oldest cut).
    pub already_captured_budget_tokens: usize,
    pub max_turns: usize,
    pub max_tokens: u32,
}

impl LiveExtractor {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        store: Arc<Mutex<Store>>,
        memory: Arc<Mutex<Memory>>,
        session_id: &str,
    ) -> Self {
        LiveExtractor {
            provider,
            store,
            memory,
            session_id: session_id.to_string(),
            cursor: 0,
            min_new_chars: 120,
            transcript_window_tokens: 2_000,
            already_captured_budget_tokens: 400,
            max_turns: 8,
            max_tokens: 1_024,
        }
    }

    /// Chars of transcript covered by the last successful pass.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn locked(&self) -> Result<std::sync::MutexGuard<'_, Store>, CoreError> {
        self.store
            .lock()
            .map_err(|_| CoreError::InvalidState("store lock poisoned".into()))
    }

    /// Runs one incremental extraction pass if warranted. Never surfaces an LLM
    /// error — recording must not be disrupted (see module docs). `Err` is
    /// reserved for genuine store faults (lock poisoned, session vanished).
    pub async fn maybe_extract(&mut self) -> Result<LiveExtractOutcome, CoreError> {
        // Gate + snapshot under a scoped store guard (never held across an await,
        // never overlapping the memory guard).
        let (window, already_captured, items_before, seen_chars) = {
            let store = self.locked()?;
            let session = store.get_session(&self.session_id)?;
            if session.status != SessionStatus::Recording {
                return Ok(LiveExtractOutcome::Skipped);
            }
            let total_chars = session.transcript.chars().count();
            if total_chars.saturating_sub(self.cursor) < self.min_new_chars {
                return Ok(LiveExtractOutcome::Skipped);
            }
            let window: String = session.transcript.chars().skip(self.cursor).collect();
            let items = store.list_items_for_session(&self.session_id)?;
            (window, prompts::format_already_captured(&items), items.len(), total_chars)
        };

        // Memory guard in its own scope — no overlap with the store guard above.
        let memory_prompt = self
            .memory
            .lock()
            .map_err(|_| CoreError::InvalidState("memory lock poisoned".into()))?
            .to_prompt();

        let assembled = ContextAssembler::assemble(&[
            ContextSection {
                title: "already captured".into(),
                content: already_captured,
                budget_tokens: self.already_captured_budget_tokens,
            },
            ContextSection {
                title: "new transcript".into(),
                content: window,
                budget_tokens: self.transcript_window_tokens,
            },
        ]);

        let mut registry = ToolRegistry::new();
        registry.register(AddItemTool::new(self.store.clone(), &self.session_id));
        let agent = Agent::new(
            self.provider.clone(),
            registry,
            AgentConfig {
                system_prompt: prompts::live_extraction_system_prompt(&memory_prompt),
                max_turns: self.max_turns,
                max_tokens: self.max_tokens,
            },
        );

        match agent.run(vec![Message::user_text(assembled.text)]).await {
            Ok(outcome) => {
                let items_after = {
                    let store = self.locked()?;
                    // Cost first (R9), then read the new count.
                    store.record_llm_usage(
                        Some(&self.session_id),
                        "live_extraction",
                        &outcome.usage,
                    )?;
                    store.list_items_for_session(&self.session_id)?.len()
                };
                // Advance only on success — a failed pass re-reads this window.
                self.cursor = seen_chars;
                Ok(LiveExtractOutcome::Extracted {
                    items_added: items_after.saturating_sub(items_before),
                    usage: outcome.usage,
                })
            }
            Err(run_err) => {
                // Swallow. Log only real spend: a turn-1 provider error burned no
                // tokens, so a zero row would be noise (coordinator precedent). A
                // store failure here must not mask the swallow — best-effort.
                if run_err.usage != Usage::default() {
                    if let Ok(store) = self.locked() {
                        let _ = store.record_llm_usage(
                            Some(&self.session_id),
                            "live_extraction",
                            &run_err.usage,
                        );
                    }
                }
                Ok(LiveExtractOutcome::Failed { usage: run_err.usage })
            }
        }
    }
}
```

`src/pipeline/mod.rs` — add below `pub mod tools;` (keep `pub(crate) mod prompts;`):
```rust
pub mod live;
```

`src/lib.rs` — extend the pipeline re-exports:
```rust
pub use pipeline::live::{LiveExtractOutcome, LiveExtractor};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo test -p murmur-core`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): LiveExtractor — incremental in-session passes onto the live board"
```

---

### Task 3: Failure posture, dedup, and retry-after-partial

**Files:**
- Modify: `src/pipeline/live.rs` (tests only)

- [ ] **Step 1: Write the failing tests** (add to the tests module in `src/pipeline/live.rs`)

```rust
    #[tokio::test]
    async fn provider_error_is_swallowed_and_cursor_held() {
        // empty script → first complete() errors: RunError carries zero usage
        let (mut extractor, store, _mem, sid) =
            extractor_with(vec![], "order lumber for the deck today");
        let outcome = extractor.maybe_extract().await.unwrap();
        assert_eq!(outcome, LiveExtractOutcome::Failed { usage: Usage::default() });
        // cursor NOT advanced → next tick retries the same window
        assert_eq!(extractor.cursor(), 0);
        let store = store.lock().unwrap();
        // no tokens burned → no usage row (a zero row would be noise)
        assert!(store.list_llm_usage_for_session(&sid).unwrap().is_empty());
        assert!(store.list_items_for_session(&sid).unwrap().is_empty());
    }

    #[tokio::test]
    async fn mid_run_failure_logs_partial_usage_and_holds_cursor() {
        // one tool_use response, then the script is exhausted → the agent loops
        // for a second turn and the provider errors. RunError carries the first
        // turn's usage AND the add_item already wrote one live item.
        let (mut extractor, store, _mem, sid) = extractor_with(
            vec![tool_use("add_item", serde_json::json!({"kind": "todo", "text": "order lumber"}))],
            "order lumber for the deck today",
        );
        let outcome = extractor.maybe_extract().await.unwrap();
        assert_eq!(
            outcome,
            LiveExtractOutcome::Failed { usage: Usage { input_tokens: 30, output_tokens: 8 } }
        );
        assert_eq!(extractor.cursor(), 0, "cursor held so the window retries");

        let store = store.lock().unwrap();
        // the item the failing pass already wrote stays on the board (R7)
        assert_eq!(store.list_items_for_session(&sid).unwrap().len(), 1);
        // partial spend is logged (R9)
        let usage = store.list_llm_usage_for_session(&sid).unwrap();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].purpose, "live_extraction");
        assert_eq!(usage[0].input_tokens, 30);
    }

    #[tokio::test]
    async fn already_captured_list_is_in_the_user_message() {
        let provider = Arc::new(MockProvider::new(vec![end_turn("noted")]));
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        store.append_transcript(&session.id, "still need to order the lumber today").unwrap();
        store.add_item(&session.id, "todo", "order lumber").unwrap();
        let sid = session.id;
        let mut extractor = LiveExtractor::new(
            provider.clone(),
            Arc::new(Mutex::new(store)),
            Arc::new(Mutex::new(Memory::default())),
            &sid,
        );
        extractor.min_new_chars = 1;
        extractor.maybe_extract().await.unwrap();
        let reqs = provider.requests();
        assert!(matches!(
            &reqs[0].messages[0].content[0],
            ContentBlock::Text { text }
                if text.contains("already captured") && text.contains("order lumber")
        ));
    }
```

- [ ] **Step 2: Run to see failure**

Run: `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo test -p murmur-core live`
Expected: PASS immediately (Task 2's implementation already satisfies these — this task is pure test-hardening of the failure/dedup contract). If any assertion fails, fix the implementation, not the test.

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "test(core): live pass failure is swallowed, cursor held, dedup list wired"
```

---

### Task 4: Swap-contract integration test + exports + docs + verification

**Files:**
- Create: `crates/murmur-core/tests/live_extraction_e2e.rs`
- Modify: `README.md`

- [ ] **Step 1: Write the integration test** (exercises only public re-exports)

```rust
//! Live board → end-of-session swap (spec Rev 2 §2): live extraction populates
//! the board during recording; `process()` tombstones those items and
//! re-creates the authoritative set. Only public `murmur_core::` API is used.

use std::sync::{Arc, Mutex};

use harness::{
    CompletionResponse, ContentBlock, HarnessError, Memory, MemoryStore, MockProvider, StopReason,
    Usage,
};
use murmur_core::{LiveExtractOutcome, LiveExtractor, SessionProcessor, SessionStatus, Store};

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
        usage: Usage { input_tokens: 30, output_tokens: 8 },
    }
}

fn end_turn(text: &str) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::Text { text: text.into() }],
        stop_reason: StopReason::EndTurn,
        usage: Usage { input_tokens: 10, output_tokens: 2 },
    }
}

#[tokio::test]
async fn live_board_is_swapped_by_end_of_session_processing() {
    let store = Store::open_in_memory("marcos-phone").unwrap();
    let session = store.start_session(None).unwrap();
    store
        .append_transcript(&session.id, "order twelve two-by-tens for the deck framing today")
        .unwrap();
    let sid = session.id;
    let store = Arc::new(Mutex::new(store));
    let memory = Arc::new(Mutex::new(Memory::default()));

    // Live pass: a CHEAP provider extracts one item onto the board.
    let mut extractor = LiveExtractor::new(
        Arc::new(MockProvider::new(vec![
            tool_use("add_item", serde_json::json!({"kind": "todo", "text": "order twelve 2x10s"})),
            end_turn("captured"),
        ])),
        store.clone(),
        memory.clone(),
        &sid,
    );
    extractor.min_new_chars = 1;
    let outcome = extractor.maybe_extract().await.unwrap();
    assert!(matches!(outcome, LiveExtractOutcome::Extracted { items_added: 1, .. }));

    // The live board shows the item; capture its id to prove the swap.
    let live_ids: Vec<String> = {
        let s = store.lock().unwrap();
        s.list_items_for_session(&sid).unwrap().into_iter().map(|i| i.id).collect()
    };
    assert_eq!(live_ids.len(), 1);

    // End recording → queue → process with the (stronger) end-of-session provider.
    store.lock().unwrap().end_and_record_session(&sid).unwrap();
    let processor = SessionProcessor::new(
        Arc::new(MockProvider::new(vec![
            tool_use("add_item", serde_json::json!({"kind": "todo", "text": "order 12 2x10 joists"})),
            tool_use("add_item", serde_json::json!({"kind": "safety", "text": "verify ledger attachment"})),
            end_turn("done"),
            tool_use("write_summary", serde_json::json!({"summary": "Deck framing: lumber ordered."})),
        ])),
        store.clone(),
        memory.clone(),
        Arc::new(NullMemoryStore),
    );
    let processed = processor.process(&sid).await.unwrap();
    assert_eq!(processed.session.status, SessionStatus::Processed);

    // Contract: live items are tombstoned and REPLACED by the authoritative set.
    let s = store.lock().unwrap();
    let after = s.list_items_for_session(&sid).unwrap();
    assert_eq!(after.len(), 2, "authoritative pass re-created the board");
    for item in &after {
        assert!(!live_ids.contains(&item.id), "live item ids must not survive the swap");
    }
    // Both passes are billed to the session under distinct purposes (R9).
    let purposes: Vec<String> = s
        .list_llm_usage_for_session(&sid)
        .unwrap()
        .into_iter()
        .map(|u| u.purpose)
        .collect();
    assert!(purposes.contains(&"live_extraction".to_string()));
    assert!(purposes.contains(&"processing".to_string()));
}
```

- [ ] **Step 2: Run it**

Run: `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo test -p murmur-core --test live_extraction_e2e`
Expected: PASS. If a re-export is missing, add it to `lib.rs` rather than reaching into modules.

- [ ] **Step 3: Update README**

Replace the plan-series lines:
```markdown
Done: 01 foundation, 02 memory + reflection + context assembler, 03 domain + storage, 04 processing pipeline + reflection coordinator, 05 live extraction.
Next: 06.
```

- [ ] **Step 4: Full verification**

Run: `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo test` → all pass.
Run: `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo clippy --all-targets` → zero warnings (fix mechanically, no API changes, no `#[allow]`; STOP and report if a fix would change behavior).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "test(core): live-board swap e2e; docs: plan 05 done"
```

---

## Deferred (named, for later plans)

1. **Offline-degradable live board (spec Rev 2 §2).** The extractor already degrades gracefully — a failed pass is swallowed and end-of-session `process()` catches everything. Explicit "queue live passes while offline and replay" is unnecessary: live extraction is a pure enhancement and `process()` is the truth. No work owed; noted so a reviewer doesn't mistake the absence for a gap.
2. **Cursor lookback / boundary items.** A reference that spans the cursor boundary (subject named before the cursor, verb after) can be missed live. `process()` catches it. If field testing shows the live board feels laggy on such items, revisit with a small fixed lookback overlap — not before evidence.
3. **Cadence policy in the app shell (Plan 07 FFI + platform shells).** The "when to tick" decision (every N seconds of new transcript / on pause detection / on silence) is app-shell policy. Core exposes `maybe_extract` + `min_new_chars`; the shell owns the timer. FFI wiring exposes `LiveExtractor` across UniFFI (likely needs an `&self` wrapper over the `&mut self` method — an actor or a `Mutex<usize>` cursor at the boundary).
4. **Model routing config.** `LiveExtractor` and `SessionProcessor` each take their own `Arc<dyn LlmProvider>` (the cheap-vs-strong seam). A routing/config type (model ids per purpose: `live_extraction`, `processing`, `reflection`) is app-shell wiring (FFI plan).
5. **Live board animations / arrival hints.** `LiveExtractOutcome::Extracted { items_added }` is the refresh hint. Per-item arrival diffs (the generative-UI layout-op vocabulary, spec §5) are a renderer concern, deferred to the UI plans.
6. **Spend cap interaction with live passes (R9 second half).** Live passes add rows to `llm_usage`; `usage_totals()` already sums them. A hard cap that *pauses live extraction* when the session nears the cap belongs to the app layer — core just reports.

## Self-Review Notes

- **Spec coverage:** Rev 2 §2 live extraction ✓ (Task 2: `LiveExtractor::maybe_extract`, add-item-only, cheaper provider seam); offline-degradable ✓ (failure swallowed, Task 3 + Deferred 1); end-of-session is truth / swap contract ✓ (Task 4 integration test proves tombstone-and-replace via `clear_session_outputs`); R6 doubly ✓ (Task 1 live prompt "Bias hard toward fewer items", partial-transcript skip rule); R7 ✓ (live items are real `items` rows, inspectable, and a failed pass's items persist); R9 ✓ (Task 2 + Task 3 log `"live_extraction"` usage on success and on partial failure, session-tagged, zero-usage row suppressed). Status-gate serialization boundary ✓ (Task 2 `skips_when_not_recording`; `process()` already rejects non-`AwaitingProcessing/Failed`).
- **Type consistency:** every referenced item exists in the read source — `AddItemTool::new(Arc<Mutex<Store>>, &str)` (tools.rs), `Store::{get_session, list_items_for_session, add_item, record_llm_usage, list_llm_usage_for_session, end_and_record_session, clear_session_outputs}` (sessions.rs/items.rs/usage.rs), `Agent::new(Arc<dyn LlmProvider>, ToolRegistry, AgentConfig)` and `Agent::run(Vec<Message>)` returning `Result<TurnOutcome, RunError>` with `RunError { source, usage }` (agent.rs), `ContextAssembler::assemble(&[ContextSection])→AssembledContext{ text, .. }` (context.rs), `Memory::to_prompt()` + `remember_from` (used in mod.rs/coordinator tests), `MockProvider::{new, requests}` (mock.rs), `SessionProcessor::{new, process}` + `ProcessOutcome{ session, usage }` (mod.rs), `SessionStatus::Recording` (domain.rs), `CapturedItem{ id, kind, text, .. }` (domain.rs). `LiveExtractOutcome` derives `Clone, Debug, PartialEq` so tests can `assert_eq!` it. No new store method, no migration — verified `clear_session_outputs` and `list_items_for_session` already deliver the swap.
- **Judgment calls for reviewers:** (a) `&mut self` over interior mutability — the cursor is per-session serial state; the borrow checker enforces the sequential-tick contract for free (FFI wraps later). (b) same `items` table over an ephemeral surface — the tombstone cascade + board query already exist; a separate surface duplicates them for no product gain. (c) in-memory cursor — the board is disposable (process() rebuilds it), so persistence buys nothing worth a migration. (d) add-item-only — smaller tool surface = cheaper, safer live calls; reports/contacts/memory are end-of-session concerns. (e) failure returns `Ok(Failed{..})` not `Err` — "no error surfaced to the recording flow"; `Err` means a store fault only. (f) dedup by "already captured" list rather than diffing — one list in the prompt, newest-first so truncation drops the least-relevant; the retry-after-partial case (Task 3) proves it also covers items a crashed pass already wrote.
- **Test-count checkpoints:** T1 +4 (prompts), T2 +5 (live core), T3 +3 (failure/dedup: provider-error swallow, mid-run partial usage, already-captured list in the user message), T4 +1 e2e ≈ **13 new**. Counts are expectations, not gates. Existing suite is 159; expect ~172 after.
- **Constraints surfaced for Plan 06/07:** (1) `LiveExtractor::maybe_extract` is `&mut self` — the FFI boundary (Plan 07) cannot expose it directly through a `&self` UniFFI object; it needs an actor/`Mutex` wrapper (noted Deferred 3). (2) The cadence trigger is deliberately NOT in core — the platform shells own the timer, so Plan 07 must wire a tick loop tied to STT segment arrival / pause detection. (3) Live and end-of-session passes take separate providers — Plan 07's config must route at least three purposes (`live_extraction` cheap, `processing` strong, `reflection` cheap). (4) Nothing in Plan 05 touches STT (Plan 06's domain); the extractor consumes whatever `append_transcript` has persisted, so STT and live extraction compose without coupling.
```