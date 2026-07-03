# Murmur Rust Core — Plan 04: Processing Pipeline + Reflection Coordinator

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire harness to murmur-core: the end-of-session processing pipeline (agent pass with vocational tools → extracted items/contacts/report → summary → processed/failed), LLM cost logging (R9 day one), the lightweight session projection, and the reflection coordinator (snapshot → reflect → swap → record).

**Architecture:** All in `crates/murmur-core` at `/Users/claude/murmur-rmp`. `Store` moves behind `Arc<Mutex<Store>>` for tool sharing (std Mutex, never held across await — the Plan 01 reviewer decision: tools capture handles via Arc, no context param on `Tool::execute`). `SessionProcessor` runs a two-phase pass per session: (1) extraction — `harness::Agent` with four tools (`add_item`, `upsert_contact`, `write_report`, plus harness's `update_memory` session-tagged); (2) summary — one-shot forced `write_summary` tool call (the Plan 02 reflection-engine pattern). Reprocessing is idempotent: prior extracted outputs are tombstoned first, so a Failed retry can't duplicate todos. `ReflectionCoordinator` implements the Plan-02-deferred contract: pre-reflection snapshot save, `activity_for_reflection` → `ReflectionEngine::reflect` → swap-and-persist → `record_reflection`; never called during an active session (documented, app-enforced). Model routing stays wiring: processor and coordinator each take their own `Arc<dyn LlmProvider>` (cheap model for reflection/summary, stronger for extraction — the app decides).

**Tech Stack:** existing deps only (harness path dep, rusqlite, serde/serde_json, thiserror; tokio + harness's MockProvider in dev-deps). NEW dev-dependency on murmur-core: `tokio` (workspace, for async tests). All tests hermetic — MockProvider only, no network.

**Spec:** vision spec §4 (murmur-core registers vocational tools), §6 (process on session end, offline queue), §7 (learning system), R6 (under-extraction bias), R7 (inspectable outcomes), R9 (cost per session measured and logged from day one). Plan 03 opus-review contract: SessionSummary projection; dual-call session-end; queue pull via `list_sessions_by_status`; coordinator sequence with pre-reflection snapshot.

---

## File Structure

```
crates/murmur-core/
  Cargo.toml               # MODIFY: add tokio dev-dependency
  src/
    error.rs               # MODIFY: CoreError::Agent(#[from] HarnessError)
    store/migrations.rs    # MODIFY (pre-ship v1 edit): llm_usage table
    store/usage.rs         # NEW: record/list/total LLM usage
    store/mod.rs           # MODIFY: mod usage;
    store/sessions.rs      # MODIFY: SessionSummary projection, end_and_record_session, clear_session_outputs
    domain.rs              # MODIFY: SessionSummary, LlmUsageRow
    pipeline/
      mod.rs               # NEW: SessionProcessor, ProcessOutcome, process(), process_pending()
      tools.rs             # NEW: AddItemTool, UpsertContactTool, WriteReportTool
      prompts.rs           # NEW: extraction system prompt, summary prompt + tool spec
    coordinator.rs         # NEW: ReflectionCoordinator
    lib.rs                 # MODIFY: wire modules
```

Run cargo via the dev shell or `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo <cmd>`. The v1 migration is still pre-ship (no remote, no released app) — editing it is authorized; append-only discipline starts at first ship.

---

### Task 1: Usage logging + Agent error variant

**Files:**
- Modify: `src/error.rs`, `src/store/migrations.rs`, `src/store/mod.rs`, `src/domain.rs`, `src/lib.rs`
- Create: `src/store/usage.rs`

- [ ] **Step 1: Write the failing tests** (bottom of `src/store/usage.rs`)

```rust
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use harness::Usage;

    use crate::store::Store;

    fn store() -> Store {
        Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000))
    }

    #[test]
    fn record_and_list_for_session() {
        let s = store();
        let session = s.start_session(None).unwrap();
        s.record_llm_usage(
            Some(&session.id),
            "processing",
            &Usage { input_tokens: 900, output_tokens: 120 },
        )
        .unwrap();
        let rows = s.list_llm_usage_for_session(&session.id).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].purpose, "processing");
        assert_eq!(rows[0].input_tokens, 900);
        assert_eq!(rows[0].output_tokens, 120);
        assert_eq!(rows[0].created_at, 1000);
    }

    #[test]
    fn sessionless_usage_is_allowed() {
        let s = store();
        s.record_llm_usage(None, "reflection", &Usage { input_tokens: 300, output_tokens: 80 })
            .unwrap();
        assert_eq!(s.usage_totals().unwrap(), (300, 80));
    }

    #[test]
    fn totals_sum_across_rows() {
        let s = store();
        let session = s.start_session(None).unwrap();
        s.record_llm_usage(Some(&session.id), "processing", &Usage { input_tokens: 10, output_tokens: 1 })
            .unwrap();
        s.record_llm_usage(None, "reflection", &Usage { input_tokens: 5, output_tokens: 2 })
            .unwrap();
        assert_eq!(s.usage_totals().unwrap(), (15, 3));
        assert_eq!(s.list_llm_usage_for_session(&session.id).unwrap().len(), 1);
    }

    #[test]
    fn unknown_session_is_rejected() {
        let s = store();
        let err = s.record_llm_usage(Some("nope"), "processing", &Usage::default());
        assert!(err.is_err(), "FK to sessions must hold");
    }
}
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p murmur-core usage`
Expected: compile FAIL.

- [ ] **Step 3: Implement**

`src/error.rs` — add variant:
```rust
    #[error("agent error: {0}")]
    Agent(#[from] harness::HarnessError),
```

`src/store/migrations.rs` — inside the v1 SQL, after the `reflection_state` table (pre-ship edit, authorized):
```sql
    -- append-only cost log (R9: cost per session measured from day one).
    -- No tombstone: rows are never deleted, only summed.
    CREATE TABLE llm_usage (
        id            TEXT PRIMARY KEY,
        session_id    TEXT REFERENCES sessions(id),
        purpose       TEXT NOT NULL,
        input_tokens  INTEGER NOT NULL,
        output_tokens INTEGER NOT NULL,
        created_at    INTEGER NOT NULL,
        device_id     TEXT NOT NULL
    );
    CREATE INDEX idx_llm_usage_session ON llm_usage(session_id);
```

`src/domain.rs` — add:
```rust
/// One LLM call's cost record (R9). Append-only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LlmUsageRow {
    pub id: String,
    pub session_id: Option<String>,
    /// What the tokens bought: "processing", "summary", "reflection", …
    pub purpose: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub created_at: u64,
    pub device_id: String,
}
```

`src/store/usage.rs` (above tests):
```rust
use harness::Usage;
use rusqlite::Row;

use crate::domain::LlmUsageRow;
use crate::error::CoreError;
use crate::ids::new_id;
use crate::store::Store;

fn usage_from_row(row: &Row) -> Result<LlmUsageRow, CoreError> {
    Ok(LlmUsageRow {
        id: row.get("id").map_err(CoreError::Sqlite)?,
        session_id: row.get("session_id").map_err(CoreError::Sqlite)?,
        purpose: row.get("purpose").map_err(CoreError::Sqlite)?,
        input_tokens: row.get::<_, i64>("input_tokens").map_err(CoreError::Sqlite)? as u64,
        output_tokens: row.get::<_, i64>("output_tokens").map_err(CoreError::Sqlite)? as u64,
        created_at: row.get::<_, i64>("created_at").map_err(CoreError::Sqlite)? as u64,
        device_id: row.get("device_id").map_err(CoreError::Sqlite)?,
    })
}

impl Store {
    /// Logs one LLM call's token cost (R9). `session_id` is None for
    /// session-independent work (reflection).
    pub fn record_llm_usage(
        &self,
        session_id: Option<&str>,
        purpose: &str,
        usage: &Usage,
    ) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO llm_usage (id, session_id, purpose, input_tokens, output_tokens, created_at, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                new_id(),
                session_id,
                purpose,
                usage.input_tokens as i64,
                usage.output_tokens as i64,
                self.now() as i64,
                self.device_id,
            ],
        )?;
        Ok(())
    }

    pub fn list_llm_usage_for_session(&self, session_id: &str) -> Result<Vec<LlmUsageRow>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, purpose, input_tokens, output_tokens, created_at, device_id
             FROM llm_usage WHERE session_id = ?1 ORDER BY id ASC",
        )?;
        let mut rows = stmt.query([session_id])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(usage_from_row(row)?);
        }
        Ok(out)
    }

    /// (total input tokens, total output tokens) across all recorded calls —
    /// the spend meter's raw feed.
    pub fn usage_totals(&self) -> Result<(u64, u64), CoreError> {
        let (i, o): (i64, i64) = self.conn.query_row(
            "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0) FROM llm_usage",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((i as u64, o as u64))
    }
}
```

`src/store/mod.rs` — add `mod usage;` alongside the other submodules.
`src/lib.rs` — add `LlmUsageRow` to the domain re-export list.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p murmur-core`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): llm usage cost log and Agent error variant"
```

---

### Task 2: SessionSummary projection, dual-call helper, idempotent-output clearing

**Files:**
- Modify: `src/domain.rs`, `src/store/sessions.rs`, `src/lib.rs`

- [ ] **Step 1: Write the failing tests** (add to the tests module in `src/store/sessions.rs`)

```rust
    #[test]
    fn summaries_are_light_and_reverse_chron() {
        let s = store().with_clock(Arc::new(|| 100));
        let a = s.start_session(None).unwrap();
        s.append_transcript(&a.id, "0123456789").unwrap();
        let s = s.with_clock(Arc::new(|| 200));
        let b = s.start_session(None).unwrap();

        let summaries = s.list_session_summaries().unwrap();
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].id, b.id);
        assert_eq!(summaries[1].id, a.id);
        assert_eq!(summaries[1].transcript_chars, 10);
        assert!(summaries[1].summary.is_none());
    }

    #[test]
    fn summaries_by_status_filter() {
        let s = store();
        let a = s.start_session(None).unwrap();
        s.end_session(&a.id).unwrap();
        let _recording = s.start_session(None).unwrap();
        let queued = s.list_session_summaries_by_status(SessionStatus::AwaitingProcessing).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].id, a.id);
    }

    #[test]
    fn end_and_record_session_does_both() {
        let s = store();
        let session = s.start_session(None).unwrap();
        let ended = s.end_and_record_session(&session.id).unwrap();
        assert_eq!(ended.status, SessionStatus::AwaitingProcessing);
        assert_eq!(s.reflection_signals().unwrap().sessions_since_reflection, 1);
    }

    #[test]
    fn clear_session_outputs_tombstones_live_children() {
        let s = store();
        let session = s.start_session(None).unwrap();
        s.add_item(&session.id, "todo", "one").unwrap();
        s.add_artifact(&session.id, "report", "t", "b").unwrap();
        let cleared = s.clear_session_outputs(&session.id).unwrap();
        assert_eq!(cleared, 2);
        assert!(s.list_items_for_session(&session.id).unwrap().is_empty());
        assert!(s.list_artifacts_for_session(&session.id).unwrap().is_empty());
        // idempotent: nothing left to clear
        assert_eq!(s.clear_session_outputs(&session.id).unwrap(), 0);
        // missing session errors
        assert!(matches!(
            s.clear_session_outputs("nope"),
            Err(CoreError::NotFound { entity: "session", .. })
        ));
    }
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p murmur-core sessions`
Expected: compile FAIL.

- [ ] **Step 3: Implement**

`src/domain.rs` — add:
```rust
/// Transcript-free projection for lists and queue polling (Plan 03 review:
/// full `Session` structs carry 50-100KB transcripts; lists must not).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub job_id: Option<String>,
    pub status: SessionStatus,
    pub summary: Option<String>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub transcript_chars: u64,
}
```

`src/store/sessions.rs` — add:
```rust
const SUMMARY_COLS: &str =
    "id, job_id, status, summary, started_at, ended_at, length(transcript) AS transcript_chars";

fn summary_from_row(row: &Row) -> Result<SessionSummary, CoreError> {
    let status_raw: String = row.get("status").map_err(CoreError::Sqlite)?;
    Ok(SessionSummary {
        id: row.get("id").map_err(CoreError::Sqlite)?,
        job_id: row.get("job_id").map_err(CoreError::Sqlite)?,
        status: SessionStatus::parse(&status_raw)?,
        summary: row.get("summary").map_err(CoreError::Sqlite)?,
        started_at: row.get::<_, i64>("started_at").map_err(CoreError::Sqlite)? as u64,
        ended_at: row
            .get::<_, Option<i64>>("ended_at")
            .map_err(CoreError::Sqlite)?
            .map(|v| v as u64),
        transcript_chars: row.get::<_, i64>("transcript_chars").map_err(CoreError::Sqlite)? as u64,
    })
}
```

And in `impl Store` (share the row-collection shape with the existing list methods):
```rust
    /// Session library / UI listing without transcripts. Reverse-chron.
    pub fn list_session_summaries(&self) -> Result<Vec<SessionSummary>, CoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SUMMARY_COLS} FROM sessions WHERE deleted_at IS NULL
             ORDER BY started_at DESC, id DESC"
        ))?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(summary_from_row(row)?);
        }
        Ok(out)
    }

    /// Queue polling without transcripts (processing pull, zombie sweep).
    pub fn list_session_summaries_by_status(
        &self,
        status: SessionStatus,
    ) -> Result<Vec<SessionSummary>, CoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SUMMARY_COLS} FROM sessions WHERE status = ?1 AND deleted_at IS NULL
             ORDER BY started_at DESC, id DESC"
        ))?;
        let mut rows = stmt.query([status.as_str()])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(summary_from_row(row)?);
        }
        Ok(out)
    }

    /// The session-end call (Plan 03 review: dual-call contract). Ends the
    /// recording AND records the session for reflection cadence in one place
    /// so callers can't forget the bookkeeping half.
    pub fn end_and_record_session(&self, id: &str) -> Result<Session, CoreError> {
        let session = self.end_session(id)?;
        self.record_session_completed()?;
        Ok(session)
    }

    /// Tombstones all live items and artifacts of a session (one transaction).
    /// The processing pipeline calls this before (re)processing so a Failed
    /// retry can't duplicate extracted outputs. Returns rows tombstoned.
    pub fn clear_session_outputs(&self, session_id: &str) -> Result<usize, CoreError> {
        self.get_session(session_id)?;
        let now = self.now() as i64;
        let tx = self.conn.unchecked_transaction()?;
        let items = tx.execute(
            "UPDATE items SET deleted_at = ?1, updated_at = ?1
             WHERE session_id = ?2 AND deleted_at IS NULL",
            rusqlite::params![now, session_id],
        )?;
        let artifacts = tx.execute(
            "UPDATE artifacts SET deleted_at = ?1, updated_at = ?1
             WHERE session_id = ?2 AND deleted_at IS NULL",
            rusqlite::params![now, session_id],
        )?;
        tx.commit()?;
        Ok(items + artifacts)
    }
```

`src/lib.rs` — add `SessionSummary` to the domain re-exports.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p murmur-core`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): session summary projection, end-and-record helper, idempotent output clearing"
```

---

### Task 3: Vocational tools

**Files:**
- Create: `src/pipeline/mod.rs` (module shell), `src/pipeline/tools.rs`
- Modify: `src/lib.rs`, `crates/murmur-core/Cargo.toml`

- [ ] **Step 1: Add tokio dev-dependency**

`crates/murmur-core/Cargo.toml`:
```toml
[dev-dependencies]
tokio = { workspace = true }
```

- [ ] **Step 2: Write the failing tests** (bottom of `src/pipeline/tools.rs`)

```rust
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use harness::{HarnessError, Tool};

    use crate::store::Store;

    fn shared_store_with_session() -> (Arc<Mutex<Store>>, String) {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        (Arc::new(Mutex::new(store)), session.id)
    }

    #[tokio::test]
    async fn add_item_writes_through_store() {
        let (store, sid) = shared_store_with_session();
        let tool = AddItemTool::new(store.clone(), &sid);
        let out = tool
            .execute(serde_json::json!({"kind": "todo", "text": "order lumber"}))
            .await
            .unwrap();
        assert_eq!(out, "added todo: order lumber");
        let items = store.lock().unwrap().list_items_for_session(&sid).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "todo");
    }

    #[tokio::test]
    async fn add_item_rejects_bad_input() {
        let (store, sid) = shared_store_with_session();
        let tool = AddItemTool::new(store, &sid);
        let err = tool.execute(serde_json::json!({"kind": "todo"})).await.unwrap_err();
        assert!(matches!(err, HarnessError::Tool { .. }));
    }

    #[tokio::test]
    async fn upsert_contact_writes_through_store() {
        let (store, _sid) = shared_store_with_session();
        let tool = UpsertContactTool::new(store.clone());
        let out = tool
            .execute(serde_json::json!({"name": "Dev", "trade": "framer"}))
            .await
            .unwrap();
        assert_eq!(out, "contact saved: Dev");
        let contacts = store.lock().unwrap().list_contacts().unwrap();
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].trade.as_deref(), Some("framer"));
    }

    #[tokio::test]
    async fn write_report_creates_artifact() {
        let (store, sid) = shared_store_with_session();
        let tool = WriteReportTool::new(store.clone(), &sid);
        let out = tool
            .execute(serde_json::json!({"title": "Johnson walk", "body": "## Summary\nDeck."}))
            .await
            .unwrap();
        assert_eq!(out, "report written: Johnson walk");
        let artifacts = store.lock().unwrap().list_artifacts_for_session(&sid).unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].kind, "report");
    }

    #[tokio::test]
    async fn store_errors_surface_as_tool_errors() {
        let store = Arc::new(Mutex::new(Store::open_in_memory("device-a").unwrap()));
        let tool = AddItemTool::new(store, "no-such-session");
        let err = tool
            .execute(serde_json::json!({"kind": "todo", "text": "x"}))
            .await
            .unwrap_err();
        assert!(matches!(err, HarnessError::Tool { .. }));
    }
}
```

- [ ] **Step 3: Run to see failure**

Run: `cargo test -p murmur-core tools`
Expected: compile FAIL.

- [ ] **Step 4: Implement**

`src/pipeline/mod.rs` (shell for now):
```rust
pub mod tools;
```

`src/pipeline/tools.rs` (above tests):
```rust
//! Vocational tools (spec §4): thin adapters from the harness Tool trait onto
//! the Store's writer API. Tools capture their handles (Arc) — no context
//! parameter on execute (Plan 01 review decision). The std Mutex is never
//! held across an await.

use std::sync::{Arc, Mutex};

use harness::{HarnessError, Tool};

use crate::store::Store;

fn tool_err(name: &str, message: impl Into<String>) -> HarnessError {
    HarnessError::Tool { name: name.into(), message: message.into() }
}

fn lock<'a>(
    store: &'a Arc<Mutex<Store>>,
    tool: &str,
) -> Result<std::sync::MutexGuard<'a, Store>, HarnessError> {
    store.lock().map_err(|_| tool_err(tool, "store lock poisoned"))
}

fn req_str<'a>(input: &'a serde_json::Value, key: &str, tool: &str) -> Result<&'a str, HarnessError> {
    input[key].as_str().ok_or_else(|| tool_err(tool, format!("missing '{key}'")))
}

pub struct AddItemTool {
    store: Arc<Mutex<Store>>,
    session_id: String,
}

impl AddItemTool {
    pub fn new(store: Arc<Mutex<Store>>, session_id: &str) -> Self {
        AddItemTool { store, session_id: session_id.to_string() }
    }
}

#[async_trait::async_trait]
impl Tool for AddItemTool {
    fn name(&self) -> &str {
        "add_item"
    }

    fn description(&self) -> &str {
        "Record one clearly-stated item from the session. Only extract what was actually said — fewer, confident items beat many guessed ones. Never invent assignees, prices, or dates."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "kind": { "type": "string", "enum": ["todo", "decision", "note", "safety", "part", "price"] },
                "text": { "type": "string", "description": "one short item, in the speaker's own terms" }
            },
            "required": ["kind", "text"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<String, HarnessError> {
        let kind = req_str(&input, "kind", "add_item")?;
        let text = req_str(&input, "text", "add_item")?;
        lock(&self.store, "add_item")?
            .add_item(&self.session_id, kind, text)
            .map_err(|e| tool_err("add_item", e.to_string()))?;
        Ok(format!("added {kind}: {text}"))
    }
}

pub struct UpsertContactTool {
    store: Arc<Mutex<Store>>,
}

impl UpsertContactTool {
    pub fn new(store: Arc<Mutex<Store>>) -> Self {
        UpsertContactTool { store }
    }
}

#[async_trait::async_trait]
impl Tool for UpsertContactTool {
    fn name(&self) -> &str {
        "upsert_contact"
    }

    fn description(&self) -> &str {
        "Save or update a person mentioned in the session (sub, client, supplier). Match is by exact name; omit fields you don't know rather than guessing."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "trade": { "type": "string" },
                "phone": { "type": "string" },
                "notes": { "type": "string" }
            },
            "required": ["name"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<String, HarnessError> {
        let name = req_str(&input, "name", "upsert_contact")?;
        let trade = input["trade"].as_str();
        let phone = input["phone"].as_str();
        let notes = input["notes"].as_str();
        lock(&self.store, "upsert_contact")?
            .upsert_contact(name, trade, phone, notes)
            .map_err(|e| tool_err("upsert_contact", e.to_string()))?;
        Ok(format!("contact saved: {name}"))
    }
}

pub struct WriteReportTool {
    store: Arc<Mutex<Store>>,
    session_id: String,
}

impl WriteReportTool {
    pub fn new(store: Arc<Mutex<Store>>, session_id: &str) -> Self {
        WriteReportTool { store, session_id: session_id.to_string() }
    }
}

#[async_trait::async_trait]
impl Tool for WriteReportTool {
    fn name(&self) -> &str {
        "write_report"
    }

    fn description(&self) -> &str {
        "Write the session report (markdown). Call at most once, and only when the session has enough substance to report on."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string" },
                "body": { "type": "string", "description": "markdown" }
            },
            "required": ["title", "body"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<String, HarnessError> {
        let title = req_str(&input, "title", "write_report")?;
        let body = req_str(&input, "body", "write_report")?;
        lock(&self.store, "write_report")?
            .add_artifact(&self.session_id, "report", title, body)
            .map_err(|e| tool_err("write_report", e.to_string()))?;
        Ok(format!("report written: {title}"))
    }
}
```

`src/lib.rs` — add:
```rust
pub mod pipeline;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p murmur-core`

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(core): vocational tools bridging harness agent to the store"
```

---

### Task 4: Prompts + summary pass

**Files:**
- Create: `src/pipeline/prompts.rs`
- Modify: `src/pipeline/mod.rs`

- [ ] **Step 1: Write the failing tests** (bottom of `src/pipeline/prompts.rs`)

```rust
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use harness::{CompletionResponse, ContentBlock, MockProvider, StopReason, Usage};

    use super::*;

    #[test]
    fn extraction_prompt_carries_the_rules() {
        let p = extraction_system_prompt("## vocabulary\n- french drain\n");
        assert!(p.contains("Only extract what was clearly said"));
        assert!(p.contains("at most once"), "report budget");
        assert!(p.contains("french drain"), "memory is injected");
        assert!(p.contains("update_memory"));
    }

    #[test]
    fn extraction_prompt_without_memory_omits_the_block() {
        let p = extraction_system_prompt("");
        assert!(!p.contains("What you know about this user"));
    }

    #[tokio::test]
    async fn summarize_forces_the_tool_and_returns_text() {
        let provider = Arc::new(MockProvider::new(vec![CompletionResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "write_summary".into(),
                input: serde_json::json!({"summary": "Walked the deck; two todos."}),
            }],
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 40, output_tokens: 12 },
        }]));
        let (summary, usage) = summarize(provider.clone(), "transcript text", 512).await.unwrap();
        assert_eq!(summary, "Walked the deck; two todos.");
        assert_eq!(usage, Usage { input_tokens: 40, output_tokens: 12 });
        let reqs = provider.requests();
        assert_eq!(reqs[0].tool_choice.as_deref(), Some("write_summary"));
        assert!(reqs[0].max_tokens >= 1);
    }

    #[tokio::test]
    async fn summarize_without_tool_call_errors() {
        let provider = Arc::new(MockProvider::new(vec![CompletionResponse {
            content: vec![ContentBlock::Text { text: "no tool".into() }],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
        }]));
        let err = summarize(provider, "t", 512).await.unwrap_err();
        assert!(matches!(err, harness::HarnessError::Provider(msg) if msg.contains("write_summary")));
    }
}
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p murmur-core prompts`
Expected: compile FAIL.

- [ ] **Step 3: Implement** (above tests in `src/pipeline/prompts.rs`)

```rust
//! Prompts for the processing pipeline. Product rules live here:
//! R6 (under-extraction bias) and R7 (real outcomes) are prompt-enforced;
//! the tools themselves stay mechanical.

use std::sync::Arc;

use harness::{
    CompletionRequest, ContentBlock, HarnessError, LlmProvider, Message, ToolSpec, Usage,
};

const WRITE_SUMMARY: &str = "write_summary";

/// System prompt for the extraction pass. `memory_prompt` is
/// `Memory::to_prompt()` output ("" when empty).
pub(crate) fn extraction_system_prompt(memory_prompt: &str) -> String {
    let memory_block = if memory_prompt.trim().is_empty() {
        String::new()
    } else {
        format!("\n\nWhat you know about this user:\n{memory_prompt}")
    };
    format!(
        "You process one transcribed field-work session (site walk, inspection, \
         client meeting) for a tradesperson. Extract structured records with the \
         tools, then reply with one short confirmation line.\n\
         Rules:\n\
         - Only extract what was clearly said. Fewer, confident items beat many \
         guessed ones — one invented assignee or price costs more trust than three \
         missed todos. When unsure, skip it.\n\
         - Use add_item for todos, decisions, notes, safety issues, parts, prices.\n\
         - Use upsert_contact for people mentioned with a role (sub, client, supplier).\n\
         - Call write_report at most once, and only if the session has enough \
         substance for a report worth sharing.\n\
         - Use update_memory only for durable facts about the user, their people, \
         projects, or vocabulary — never for session content.\n\
         - Transcripts are speech-to-text: expect misrecognized jargon and names; \
         prefer terms from what you know about the user.{memory_block}"
    )
}

fn summary_tool_spec() -> ToolSpec {
    ToolSpec {
        name: WRITE_SUMMARY.into(),
        description: "Record the session summary for the library list.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "summary": { "type": "string", "description": "1-2 plain sentences: what happened, key outcomes" }
            },
            "required": ["summary"]
        }),
    }
}

/// One-shot forced summary call (the Plan 02 reflection-engine pattern).
pub(crate) async fn summarize(
    provider: Arc<dyn LlmProvider>,
    transcript_excerpt: &str,
    max_tokens: u32,
) -> Result<(String, Usage), HarnessError> {
    let response = provider
        .complete(CompletionRequest {
            system: "Summarize one transcribed field-work session in 1-2 plain sentences \
                     for a session list. Lead with what happened; include key outcomes."
                .into(),
            messages: vec![Message::user_text(format!("Transcript:\n{transcript_excerpt}"))],
            tools: vec![summary_tool_spec()],
            max_tokens,
            tool_choice: Some(WRITE_SUMMARY.into()),
        })
        .await?;

    let summary = response
        .content
        .iter()
        .find_map(|b| match b {
            ContentBlock::ToolUse { name, input, .. } if name == WRITE_SUMMARY => {
                input.get("summary").and_then(|s| s.as_str()).map(str::to_string)
            }
            _ => None,
        })
        .ok_or_else(|| {
            HarnessError::Provider("summary response missing write_summary call".into())
        })?;
    Ok((summary, response.usage))
}
```

`src/pipeline/mod.rs` — add `pub(crate) mod prompts;` (keep `pub mod tools;`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p murmur-core`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): extraction prompt with R6 rules and forced summary pass"
```

---

### Task 5: SessionProcessor

**Files:**
- Modify: `src/pipeline/mod.rs`, `src/lib.rs`

- [ ] **Step 1: Write the failing tests** (bottom of `src/pipeline/mod.rs`)

```rust
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
        tool_use("write_summary", serde_json::json!({"summary": text}))
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
        // usage: 100+20, 100+20, 50+10 agent + 100+20 summary
        assert_eq!(outcome.usage, Usage { input_tokens: 350, output_tokens: 70 });

        let store = store.lock().unwrap();
        assert_eq!(store.list_items_for_session(&sid).unwrap().len(), 1);
        assert_eq!(store.list_contacts().unwrap().len(), 1);
        let usage_rows = store.list_llm_usage_for_session(&sid).unwrap();
        assert_eq!(usage_rows.len(), 1);
        assert_eq!(usage_rows[0].purpose, "processing");
        assert_eq!(usage_rows[0].input_tokens, 350);
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
        assert_eq!(usage_rows[0].input_tokens, 50, "agent pass tokens counted");
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
}
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p murmur-core pipeline`
Expected: compile FAIL.

- [ ] **Step 3: Implement** (in `src/pipeline/mod.rs`, above tests)

```rust
//! End-of-session processing (spec §6): transcript in, structured records +
//! summary out. Reprocessing is idempotent — prior outputs are tombstoned
//! first, so a Failed retry can't duplicate todos.

pub mod tools;

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

pub struct ProcessOutcome {
    pub session: Session,
    pub usage: Usage,
}

pub struct SessionProcessor {
    provider: Arc<dyn LlmProvider>,
    store: Arc<Mutex<Store>>,
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
            summary_max_tokens: 512,
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
    pub async fn process(&self, session_id: &str) -> Result<ProcessOutcome, CoreError> {
        // Phase 0: validate, clear prior outputs, snapshot inputs.
        let (transcript, memory_prompt) = {
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
            store.clear_session_outputs(session_id)?;
            let memory_prompt = self
                .memory
                .lock()
                .map_err(|_| CoreError::InvalidState("memory lock poisoned".into()))?
                .to_prompt();
            (session.transcript, memory_prompt)
        };

        let assembled = ContextAssembler::assemble(&[ContextSection {
            title: "transcript".into(),
            content: transcript,
            budget_tokens: self.transcript_budget_tokens,
        }]);

        // Phase 1+2: extraction agent pass, then forced summary.
        let mut usage = Usage::default();
        let result = self.run_llm_phases(session_id, &assembled.text, &memory_prompt, &mut usage).await;

        // Exit: persist outcome + cost, success or not.
        let store = self.locked()?;
        match result {
            Ok(summary) => {
                let session = store.mark_session_processed(session_id, &summary)?;
                store.record_llm_usage(Some(session_id), "processing", &usage)?;
                Ok(ProcessOutcome { session, usage })
            }
            Err(e) => {
                store.mark_session_failed(session_id)?;
                store.record_llm_usage(Some(session_id), "processing", &usage)?;
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
    ) -> Result<String, harness::HarnessError> {
        let mut registry = ToolRegistry::new();
        registry.register(AddItemTool::new(self.store.clone(), session_id));
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
        let outcome = agent
            .run(vec![Message::user_text(format!(
                "Process this session.\n\n{assembled_transcript}"
            ))])
            .await?;
        usage.add(&outcome.usage);

        let (summary, summary_usage) = prompts::summarize(
            self.provider.clone(),
            assembled_transcript,
            self.summary_max_tokens,
        )
        .await?;
        usage.add(&summary_usage);
        Ok(summary)
    }
}
```

`src/lib.rs` — extend pipeline export:
```rust
pub use pipeline::{ProcessOutcome, SessionProcessor};
pub use pipeline::tools::{AddItemTool, UpsertContactTool, WriteReportTool};
```

Note: `usage` asserts in the end-to-end test assume the agent loop echoes each scripted response's usage once — that matches the harness contract (usage accumulated per provider call).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p murmur-core`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): SessionProcessor — extraction pass, forced summary, idempotent retry, cost logging"
```

---

### Task 6: Queue runner

**Files:**
- Modify: `src/pipeline/mod.rs`

- [ ] **Step 1: Write the failing test** (add to pipeline tests)

```rust
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
```

(Note: the test reads `processor.store` — keep the `store` field `pub(crate)` or add a test-only accessor; prefer `pub(crate)`.)

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p murmur-core process_pending`
Expected: compile FAIL.

- [ ] **Step 3: Implement** (add to `impl SessionProcessor`; change `store` field to `pub(crate)`)

```rust
    /// Drains the awaiting_processing queue (spec §6: offline sessions queue
    /// and process on reconnect). One session at a time — failures mark that
    /// session Failed and the drain continues. Failed sessions are NOT
    /// auto-retried here; retry is an explicit `process()` call (user-visible
    /// retry affordance, R7).
    pub async fn process_pending(&self) -> Result<Vec<(String, Result<ProcessOutcome, CoreError>)>, CoreError> {
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p murmur-core`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): queue drain over awaiting sessions with per-session failure isolation"
```

---

### Task 7: ReflectionCoordinator

**Files:**
- Create: `src/coordinator.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write the failing tests** (bottom of `src/coordinator.rs`)

```rust
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use harness::{
        CompletionResponse, ContentBlock, HarnessError, Memory, MemoryStore, MockProvider,
        StopReason, Usage,
    };

    use crate::store::Store;

    use super::*;

    /// Records every save so tests can assert the snapshot-then-swap order.
    struct SpyMemoryStore {
        saved: Mutex<Vec<Memory>>,
    }
    impl SpyMemoryStore {
        fn new() -> Arc<Self> {
            Arc::new(SpyMemoryStore { saved: Mutex::new(Vec::new()) })
        }
    }
    impl MemoryStore for SpyMemoryStore {
        fn load(&self) -> Result<Memory, HarnessError> {
            Ok(Memory::default())
        }
        fn save(&self, m: &Memory) -> Result<(), HarnessError> {
            self.saved.lock().unwrap().push(m.clone());
            Ok(())
        }
    }

    fn write_memory_response(sections: serde_json::Value) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "write_memory".into(),
                input: serde_json::json!({"sections": sections}),
            }],
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 200, output_tokens: 40 },
        }
    }

    fn store_with_ended_session() -> Store {
        let s = Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000));
        let session = s.start_session(None).unwrap();
        s.append_transcript(&session.id, "walked the deck with Dev").unwrap();
        s.end_and_record_session(&session.id).unwrap();
        s
    }

    fn coordinator_with(
        responses: Vec<CompletionResponse>,
        store: Store,
    ) -> (ReflectionCoordinator, Arc<Mutex<Memory>>, Arc<SpyMemoryStore>, Arc<Mutex<Store>>) {
        let memory = Arc::new(Mutex::new(Memory::default()));
        let memory_store = SpyMemoryStore::new();
        let store = Arc::new(Mutex::new(store));
        let coordinator = ReflectionCoordinator::new(
            Arc::new(MockProvider::new(responses)),
            store.clone(),
            memory.clone(),
            memory_store.clone(),
        );
        (coordinator, memory, memory_store, store)
    }

    #[tokio::test]
    async fn reflects_when_policy_says_so() {
        let (coordinator, memory, memory_store, store) = coordinator_with(
            vec![write_memory_response(serde_json::json!({"people": ["Dev — framer"]}))],
            store_with_ended_session(),
        );
        let churn = coordinator.maybe_reflect().await.unwrap();
        assert!(churn.is_some());

        // memory swapped
        assert_eq!(memory.lock().unwrap().section_texts("people"), vec!["Dev — framer"]);
        // snapshot-then-swap: first save is the PRE-reflection memory (empty),
        // second is the new one
        let saves = memory_store.saved.lock().unwrap();
        assert_eq!(saves.len(), 2);
        assert!(saves[0].sections.is_empty());
        assert_eq!(saves[1].section_texts("people"), vec!["Dev — framer"]);

        // signals recorded + cost logged
        let store = store.lock().unwrap();
        let signals = store.reflection_signals().unwrap();
        assert_eq!(signals.completed_reflections, 1);
        assert_eq!(signals.sessions_since_reflection, 0);
        assert_eq!(store.usage_totals().unwrap(), (200, 40));
    }

    #[tokio::test]
    async fn skips_when_policy_says_no() {
        // fresh store: zero sessions since reflection -> policy false
        let store = Store::open_in_memory("device-a").unwrap();
        let (coordinator, _memory, memory_store, _store) = coordinator_with(vec![], store);
        let churn = coordinator.maybe_reflect().await.unwrap();
        assert!(churn.is_none());
        assert!(memory_store.saved.lock().unwrap().is_empty(), "no saves when skipped");
    }

    #[tokio::test]
    async fn skips_when_no_activity() {
        // a completed-session counter without any ended session content
        // (e.g. all sessions tombstoned since): policy says yes, activity is empty
        let s = Store::open_in_memory("device-a").unwrap().with_clock(Arc::new(|| 1000));
        let session = s.start_session(None).unwrap();
        s.end_and_record_session(&session.id).unwrap(); // empty transcript -> blank activity entry skipped
        let (coordinator, _memory, memory_store, _store) = coordinator_with(vec![], s);
        let churn = coordinator.maybe_reflect().await.unwrap();
        assert!(churn.is_none());
        assert!(memory_store.saved.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn engine_failure_leaves_memory_and_signals_untouched() {
        // engine errors (no write_memory in response)
        let bad = CompletionResponse {
            content: vec![ContentBlock::Text { text: "refused".into() }],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
        };
        let (coordinator, memory, memory_store, store) =
            coordinator_with(vec![bad], store_with_ended_session());
        let err = coordinator.maybe_reflect().await.unwrap_err();
        assert!(matches!(err, CoreError::Agent(_)));
        assert!(memory.lock().unwrap().sections.is_empty(), "memory not swapped");
        // only the pre-reflection snapshot save happened
        assert_eq!(memory_store.saved.lock().unwrap().len(), 1);
        let signals = store.lock().unwrap().reflection_signals().unwrap();
        assert_eq!(signals.completed_reflections, 0, "failed reflection is not recorded");
    }
}
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p murmur-core coordinator`
Expected: compile FAIL.

- [ ] **Step 3: Implement** (above tests in `src/coordinator.rs`)

```rust
//! Reflection coordinator (spec §7, Rev 3 §1; the Plan 02/03 deferred
//! contract). Call `maybe_reflect` when there is guaranteed compute and NO
//! active session — the engine swaps the whole memory, so an interleaved
//! in-session update would be silently discarded (see
//! `ReflectionEngine::reflect`).
//!
//! Sequence: policy gate -> activity gate -> PRE-reflection snapshot save ->
//! engine reflect -> swap + persist -> record signals + cost.

use std::sync::{Arc, Mutex};

use harness::{
    Clock, LlmProvider, Memory, MemoryStore, ReflectionEngine, ReflectionPolicy,
};

use crate::error::CoreError;
use crate::store::Store;

fn system_clock() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub struct ReflectionCoordinator {
    engine: ReflectionEngine,
    pub policy: ReflectionPolicy,
    store: Arc<Mutex<Store>>,
    memory: Arc<Mutex<Memory>>,
    memory_store: Arc<dyn MemoryStore>,
    clock: Clock,
    /// Most-recent sessions fed to one reflection.
    pub max_activity_sessions: usize,
}

impl ReflectionCoordinator {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        store: Arc<Mutex<Store>>,
        memory: Arc<Mutex<Memory>>,
        memory_store: Arc<dyn MemoryStore>,
    ) -> Self {
        ReflectionCoordinator {
            engine: ReflectionEngine::new(provider),
            policy: ReflectionPolicy::default(),
            store,
            memory,
            memory_store,
            clock: Arc::new(system_clock),
            max_activity_sessions: 8,
        }
    }

    /// Replaces the clock (tests inject deterministic time).
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    fn locked_store(&self) -> Result<std::sync::MutexGuard<'_, Store>, CoreError> {
        self.store
            .lock()
            .map_err(|_| CoreError::InvalidState("store lock poisoned".into()))
    }

    /// Runs a reflection if the cadence policy and activity feed warrant one.
    /// Returns `Some(churn)` when a reflection ran, `None` when skipped.
    /// On engine failure: memory and signals are untouched (the pre-reflection
    /// snapshot save has already rotated, which is harmless), error returned.
    pub async fn maybe_reflect(&self) -> Result<Option<f32>, CoreError> {
        let (current_memory, activity) = {
            let store = self.locked_store()?;
            let signals = store.reflection_signals()?;
            if !self.policy.should_reflect(&signals) {
                return Ok(None);
            }
            let activity = store.activity_for_reflection(self.max_activity_sessions)?;
            if activity.is_empty() {
                return Ok(None);
            }
            let memory = self
                .memory
                .lock()
                .map_err(|_| CoreError::InvalidState("memory lock poisoned".into()))?
                .clone();
            (memory, activity)
        };

        // Pre-reflection snapshot: saving the CURRENT memory rotates it into
        // the store's snapshot slots, guaranteeing a rollback point that this
        // reflection cannot erode (Plan 02 final-review note).
        self.memory_store.save(&current_memory).map_err(CoreError::Agent)?;

        let outcome = self
            .engine
            .reflect(&current_memory, &activity, (self.clock)())
            .await
            .map_err(CoreError::Agent)?;

        {
            let mut memory = self
                .memory
                .lock()
                .map_err(|_| CoreError::InvalidState("memory lock poisoned".into()))?;
            *memory = outcome.memory.clone();
        }
        self.memory_store.save(&outcome.memory).map_err(CoreError::Agent)?;

        let store = self.locked_store()?;
        store.record_reflection(outcome.churn)?;
        store.record_llm_usage(None, "reflection", &outcome.usage)?;
        Ok(Some(outcome.churn))
    }
}
```

`src/lib.rs` — add:
```rust
pub mod coordinator;
pub use coordinator::ReflectionCoordinator;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p murmur-core`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): reflection coordinator — snapshot, reflect, swap, record"
```

---

### Task 8: E2E scenario test + docs + verification

**Files:**
- Create: `crates/murmur-core/tests/pipeline_e2e.rs`
- Modify: `README.md`

- [ ] **Step 1: Write the E2E test** (integration test — exercises only public API)

```rust
//! Transcript-in -> records/summary/reflection-out, over scripted LLM
//! responses (spec §10: E2E integration tests in Rust).

use std::sync::{Arc, Mutex};

use harness::{
    CompletionResponse, ContentBlock, HarnessError, Memory, MemoryStore, MockProvider,
    StopReason, Usage,
};
use murmur_core::{
    NewJob, ReflectionCoordinator, SessionProcessor, SessionStatus, Store,
};

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
        usage: Usage { input_tokens: 100, output_tokens: 25 },
    }
}

fn end_turn(text: &str) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::Text { text: text.into() }],
        stop_reason: StopReason::EndTurn,
        usage: Usage { input_tokens: 60, output_tokens: 10 },
    }
}

#[tokio::test]
async fn site_walk_end_to_end() {
    // A day in the life: job, session, walk, process, reflect.
    let store = Store::open_in_memory("marcos-phone").unwrap();
    let job = store
        .create_job(NewJob { name: "Johnson remodel".into(), ..Default::default() })
        .unwrap();
    let session = store.start_session(Some(&job.id)).unwrap();
    store
        .append_transcript(
            &session.id,
            "okay deck framing is soft near the ledger, Dev needs to sister two joists. \
             order twelve two-by-tens. tell the client the railing decision can wait.",
        )
        .unwrap();
    store.end_and_record_session(&session.id).unwrap();

    let store = Arc::new(Mutex::new(store));
    let memory = Arc::new(Mutex::new(Memory::default()));
    let memory_store: Arc<dyn MemoryStore> = Arc::new(NullMemoryStore);

    // Processing: two items, a contact, a report, then summary.
    let processor = SessionProcessor::new(
        Arc::new(MockProvider::new(vec![
            tool_use("add_item", serde_json::json!({"kind": "todo", "text": "order twelve 2x10s"})),
            tool_use("add_item", serde_json::json!({"kind": "safety", "text": "deck framing soft near ledger"})),
            tool_use("upsert_contact", serde_json::json!({"name": "Dev", "trade": "framer"})),
            tool_use("write_report", serde_json::json!({"title": "Johnson walk", "body": "## Deck\nSister two joists."})),
            end_turn("done"),
            tool_use("write_summary", serde_json::json!({"summary": "Deck walk: framing fix planned, lumber ordered."})),
        ])),
        store.clone(),
        memory.clone(),
        memory_store.clone(),
    );
    let results = processor.process_pending().await.unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].1.is_ok());

    {
        let s = store.lock().unwrap();
        let processed = s.get_session(&session.id).unwrap();
        assert_eq!(processed.status, SessionStatus::Processed);
        assert_eq!(s.list_items_for_session(&session.id).unwrap().len(), 2);
        assert_eq!(s.list_artifacts_for_session(&session.id).unwrap().len(), 1);
        assert_eq!(s.list_contacts().unwrap().len(), 1);
        assert_eq!(s.list_open_todos().unwrap().len(), 1);
        let (input, output) = s.usage_totals().unwrap();
        assert!(input > 0 && output > 0, "R9: cost logged");
        // the session library sees the summary without the transcript
        let summaries = s.list_session_summaries().unwrap();
        assert_eq!(summaries[0].summary.as_deref(), Some("Deck walk: framing fix planned, lumber ordered."));
    }

    // Reflection: warmup cadence -> reflect on the session's summary.
    let coordinator = ReflectionCoordinator::new(
        Arc::new(MockProvider::new(vec![tool_use(
            "write_memory",
            serde_json::json!({"sections": {"people": ["Dev — framer"], "vocabulary": ["sister joists"]}}),
        )])),
        store.clone(),
        memory.clone(),
        memory_store,
    );
    let churn = coordinator.maybe_reflect().await.unwrap();
    assert!(churn.is_some());
    assert_eq!(memory.lock().unwrap().section_texts("people"), vec!["Dev — framer"]);
    let signals = store.lock().unwrap().reflection_signals().unwrap();
    assert_eq!(signals.completed_reflections, 1);
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p murmur-core --test pipeline_e2e`
Expected: PASS (fix faithfully if the public API surface differs — this test may only use `murmur_core::` re-exports; add missing re-exports to lib.rs rather than reaching into modules).

- [ ] **Step 3: Update README**

Plan-series lines:
```markdown
Done: 01 foundation, 02 memory + reflection + context assembler, 03 domain + storage, 04 processing pipeline + reflection coordinator.
Next: 05 live extraction.
```

- [ ] **Step 4: Full verification**

Run: `cargo test` → all pass. `cargo clippy --all-targets` → zero warnings (fix mechanically, no API changes, no allow-attributes; STOP and report if behavior would change).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "test(core): site-walk e2e scenario; docs: plan 04 done"
```

---

## Deferred (named, for later plans)

1. **Live extraction (Plan 05)** — incremental agent passes during recording feed the live board; end-of-session `process()` stays the source of truth.
2. **Model routing config** — processor and coordinator already take separate providers; a routing/config type (model ids per purpose) is app-shell wiring (FFI plan).
3. **Spend cap enforcement (R9 second half)** — `usage_totals()` is the feed; the cap check + UI belong to the app layer.
4. **NFC normalization** — before memory writes and search, when keyboard input joins STT.
5. **Contact merge across sync** — upsert dedups by name locally, by id across sync (Plan 03 opus note).
6. **Processing checkpoints for very-long sessions** (spec §6) — revisit with live extraction; the transcript budget truncation covers the pathological case until then.

## Self-Review Notes

- **Spec coverage:** §4 vocational tools ✓ (Task 3: add_item/upsert_contact/write_report + harness update_memory registered in Task 5), §6 process-on-end + offline queue ✓ (Tasks 5–6), §7 coordinator contract ✓ (Task 7: snapshot→reflect→swap→record, no-overlap doc), R6 ✓ (prompt + tool descriptions), R7 ✓ (tool results are real store outcomes via Plan 01 agent loop; explicit-retry note on process_pending), R9 ✓ (Task 1 + cost logged on failure, tested). Plan 03 opus contract: SessionSummary ✓ (Task 2), dual-call helper ✓ (Task 2), queue pull ✓ (Task 6 uses summaries — no transcript loading).
- **Type consistency:** `CoreError::Agent(#[from] HarnessError)` (Task 1) consumed by pipeline/coordinator (`.map_err(CoreError::Agent)` where the `?`-from conversion isn't available on harness results inside non-From contexts — both compile paths shown); `clear_session_outputs` (Task 2) called in Task 5 Phase 0; `list_session_summaries_by_status` (Task 2) consumed by Task 6; tools constructed with `Arc<Mutex<Store>>` matching `SessionProcessor.store`; `UpdateMemoryTool::new(...).for_session(...)` matches harness's shipped builder; `MockProvider::new(Vec<CompletionResponse>)` + `.requests()` and `Agent::new(provider, registry, AgentConfig)` verified against harness source before writing.
- **Judgment calls for reviewers:** two-phase (extraction agent + forced summary) over single-pass — deterministic summary extraction beats parsing the agent's final prose; idempotent reprocess by tombstoning prior outputs — simplest correct retry story, tombstones preserve the audit trail (R7); `process_pending` doesn't auto-retry Failed — retries stay user-visible; coordinator returns `Option<f32>` churn — the app's only decision input is "did it run"; std Mutex over tokio Mutex — no lock is ever held across an await, verified by construction (locks are scoped blocks around store calls).
- **Test-count checkpoints:** T1 +4, T2 +4, T3 +5, T4 +4, T5 +5, T6 +1, T7 +4, T8 +1 e2e ≈ 28 new. Counts are expectations, not gates.
