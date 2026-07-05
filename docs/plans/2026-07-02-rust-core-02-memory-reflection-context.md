# Murmur Rust Core — Plan 02: Memory, Reflection, Context Assembler

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the harness its learning machinery: a capped, decaying, sectioned memory with file persistence and an in-session update tool; an LLM-driven reflection engine with a signal-driven cadence policy; and a token-budgeted context assembler.

**Architecture:** All in `crates/harness` at `/Users/claude/murmur-rmp` (no app-specific logic — sections are consumer-named strings). Memory is a `BTreeMap<section, Vec<MemoryEntry>>` with a word cap enforced by dropping oldest-touched entries; staleness pruning is explicit (`prune_stale`). Reflection is a full-replace pass (spec: "replace, not append"): the engine sends current memory + recent activity, **forces** a `write_memory` tool call (new `tool_choice` field on `CompletionRequest`), rebuilds the Memory preserving `last_touched` for retained texts, clamps to cap, and reports a churn score. `ReflectionPolicy` turns churn/corrections/session counts into a should-reflect decision (every session during warmup, exponential backoff on low churn, corrections snap it back — spec Rev 3). The context assembler truncates named sections to per-section token budgets using a documented chars/4 approximation. Platform scheduling (BGProcessingTask etc.) is NOT in this plan — the app layer calls `should_reflect`/`reflect` when it has compute.

**Tech Stack:** Existing deps only (serde, serde_json, thiserror, async-trait, tokio/wiremock in dev). No new crates. Timestamps are unix seconds (`u64`); tests inject clocks.

**Spec:** `docs/superpowers/specs/2026-07-01-murmur-rebuild-vision-design.md` §7 (learning system), §4 (context assembler), Rev 3 amendments 1–3. Vocabulary's second consumer (STT biasing) reads `Memory::section_texts("vocabulary")` in Plan 05 — the accessor is built here.

---

## File Structure

```
crates/harness/src/
  llm.rs                 # MODIFY: add tool_choice to CompletionRequest
  error.rs               # MODIFY: add Storage variant
  providers/anthropic.rs # MODIFY: serialize tool_choice when Some
  mock.rs                # MODIFY (tests only): tool_choice: None in constructions
  agent.rs               # MODIFY: pass tool_choice: None in the loop
  memory/
    mod.rs               # Memory, MemoryEntry, cap/prune/prompt/accessors
    store.rs             # MemoryStore trait + FileMemoryStore (atomic write)
    tool.rs              # UpdateMemoryTool (remember/forget)
  reflection/
    mod.rs               # ReflectionEngine, ReflectionOutcome, churn
    policy.rs            # ReflectionPolicy, ReflectionSignals
  context.rs             # approx_tokens, ContextSection, ContextAssembler
  lib.rs                 # MODIFY: wire new modules
```

Run cargo on this host via the repo dev shell (`direnv`/`nix develop`) or `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo <cmd>`.

---

### Task 1: `tool_choice` plumbing + `Storage` error variant

**Files:**
- Modify: `crates/harness/src/llm.rs`, `crates/harness/src/error.rs`, `crates/harness/src/providers/anthropic.rs`, `crates/harness/src/mock.rs` (test constructions), `crates/harness/src/agent.rs` (loop request)

- [ ] **Step 1: Write the failing test** (add to `providers/anthropic.rs` tests module)

```rust
    #[tokio::test]
    async fn forced_tool_choice_is_serialized_and_absent_when_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [{"type": "text", "text": "ok"}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 1, "output_tokens": 1}
            })))
            .expect(2)
            .mount(&server)
            .await;

        let provider = AnthropicProvider::new("sk-test", "claude-haiku-4-5-20251001")
            .with_base_url(server.uri());

        let mut req = request();
        req.tool_choice = Some("echo".into());
        provider.complete(req).await.unwrap();

        provider.complete(request()).await.unwrap();

        let received = server.received_requests().await.unwrap();
        let body0: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
        assert_eq!(body0["tool_choice"]["type"], "tool");
        assert_eq!(body0["tool_choice"]["name"], "echo");
        let body1: serde_json::Value = serde_json::from_slice(&received[1].body).unwrap();
        assert!(body1.get("tool_choice").is_none());
    }
```

- [ ] **Step 2: Run tests to see the compile failure**

Run: `cargo test -p harness`
Expected: compile FAIL — `CompletionRequest` has no field `tool_choice`.

- [ ] **Step 3: Implement**

`crates/harness/src/error.rs` — add variant after `Provider`:
```rust
    #[error("storage error: {0}")]
    Storage(String),
```

`crates/harness/src/llm.rs` — add field to `CompletionRequest` (after `max_tokens`):
```rust
    /// Force the model to call this tool by name (None = model decides).
    pub tool_choice: Option<String>,
```

Update EVERY existing `CompletionRequest { ... }` construction to include `tool_choice: None`:
- `mock.rs` tests: 2 sites
- `agent.rs` `run()` loop: 1 site (the agent never forces a tool)
- `providers/anthropic.rs` tests `request()` helper: 1 site

`crates/harness/src/providers/anthropic.rs` — in `complete`, after building `body`, insert conditionally (body must stay identical when `None`):
```rust
        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": req.max_tokens,
            "system": req.system,
            "messages": req.messages,
            "tools": req.tools,
        });
        if let Some(name) = &req.tool_choice {
            body["tool_choice"] = serde_json::json!({"type": "tool", "name": name});
        }
```
(change `let body` to `let mut body`; the rest of the function is unchanged)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harness`
Expected: all pass (19 unit + previous, plus the new one → 20 total across targets with the e2e).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(harness): forced tool_choice on requests; Storage error variant"
```

---

### Task 2: Memory core types

**Files:**
- Create: `crates/harness/src/memory/mod.rs`
- Modify: `crates/harness/src/lib.rs`

- [ ] **Step 1: Write the failing tests** (bottom of `memory/mod.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn mem_with(section: &str, texts: &[(&str, u64)]) -> Memory {
        let mut m = Memory::default();
        for (t, at) in texts {
            m.remember(section, t, *at);
        }
        m
    }

    #[test]
    fn remember_adds_and_touches_existing() {
        let mut m = Memory::default();
        m.remember("people", "Dev — framer", 100);
        m.remember("people", "Dev — framer", 200); // same text: touch, don't duplicate
        assert_eq!(m.sections["people"].len(), 1);
        assert_eq!(m.sections["people"][0].last_touched, 200);
    }

    #[test]
    fn forget_removes_and_reports() {
        let mut m = mem_with("people", &[("Dev — framer", 100)]);
        assert!(m.forget("people", "Dev — framer"));
        assert!(!m.forget("people", "Dev — framer"));
        assert!(m.sections.get("people").is_none(), "empty sections are dropped");
    }

    #[test]
    fn word_count_counts_entry_words() {
        let m = mem_with("vocabulary", &[("bark mulch", 1), ("french drain", 1)]);
        assert_eq!(m.word_count(), 4);
    }

    #[test]
    fn to_prompt_renders_sections_in_order() {
        let mut m = mem_with("people", &[("Dev — framer", 1)]);
        m.remember("jobs", "Johnson remodel — active", 1);
        assert_eq!(
            m.to_prompt(),
            "## jobs\n- Johnson remodel — active\n\n## people\n- Dev — framer\n"
        );
    }

    #[test]
    fn to_prompt_empty_memory_is_empty_string() {
        assert_eq!(Memory::default().to_prompt(), "");
    }

    #[test]
    fn section_texts_accessor() {
        let m = mem_with("vocabulary", &[("skid steer", 1), ("french drain", 1)]);
        assert_eq!(m.section_texts("vocabulary"), vec!["french drain", "skid steer"]);
        assert!(m.section_texts("nope").is_empty());
    }
}
```

Note on `section_texts` ordering: entries are stored in insertion order within a section; the test above inserts "skid steer" then "french drain" and expects insertion order back — so expected is `vec!["skid steer", "french drain"]`. Use THIS corrected assertion:
```rust
        assert_eq!(m.section_texts("vocabulary"), vec!["skid steer", "french drain"]);
```

- [ ] **Step 2: Run tests to see the compile failure**

Run: `cargo test -p harness memory`
Expected: compile FAIL — module/type not found (after adding `pub mod memory;` to lib.rs, see Step 3).

- [ ] **Step 3: Implement** (above tests in `memory/mod.rs`)

```rust
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Default word cap for a whole memory (spec §7: reflection compresses, never accumulates).
pub const DEFAULT_WORD_CAP: usize = 500;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub text: String,
    /// Unix seconds when this entry was last added, confirmed, or re-mentioned.
    pub last_touched: u64,
}

/// Sectioned agent memory. Section names are consumer-defined strings
/// (e.g. "vocabulary", "people", "projects", "preferences").
/// The "vocabulary" section is read by the STT biasing layer in Plan 05
/// via [`Memory::section_texts`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Memory {
    pub sections: BTreeMap<String, Vec<MemoryEntry>>,
}

impl Memory {
    /// Adds `text` to `section`, or refreshes `last_touched` if the exact text exists.
    pub fn remember(&mut self, section: &str, text: &str, now: u64) {
        let entries = self.sections.entry(section.to_string()).or_default();
        match entries.iter_mut().find(|e| e.text == text) {
            Some(e) => e.last_touched = now,
            None => entries.push(MemoryEntry { text: text.to_string(), last_touched: now }),
        }
    }

    /// Removes the exact `text` from `section`. Returns whether anything was removed.
    /// Sections left empty are dropped.
    pub fn forget(&mut self, section: &str, text: &str) -> bool {
        let Some(entries) = self.sections.get_mut(section) else {
            return false;
        };
        let before = entries.len();
        entries.retain(|e| e.text != text);
        let removed = entries.len() < before;
        if entries.is_empty() {
            self.sections.remove(section);
        }
        removed
    }

    /// Total whitespace-separated words across all entry texts.
    pub fn word_count(&self) -> usize {
        self.sections
            .values()
            .flatten()
            .map(|e| e.text.split_whitespace().count())
            .sum()
    }

    /// Entry texts of one section, in insertion order. Empty if the section is absent.
    pub fn section_texts(&self, section: &str) -> Vec<&str> {
        self.sections
            .get(section)
            .map(|es| es.iter().map(|e| e.text.as_str()).collect())
            .unwrap_or_default()
    }

    /// Markdown rendering for prompt injection: `## section` headers, `- ` entries,
    /// sections in BTreeMap (alphabetical) order. Empty memory renders as "".
    pub fn to_prompt(&self) -> String {
        let mut out = String::new();
        for (name, entries) in &self.sections {
            if entries.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str("## ");
            out.push_str(name);
            out.push('\n');
            for e in entries {
                out.push_str("- ");
                out.push_str(&e.text);
                out.push('\n');
            }
        }
        out
    }
}
```

`crates/harness/src/lib.rs` — add:
```rust
pub mod memory;
pub use memory::{Memory, MemoryEntry, DEFAULT_WORD_CAP};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harness`
Expected: all pass (26 unit).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(harness): sectioned Memory with remember/forget/prompt rendering"
```

---

### Task 3: Staleness pruning + cap clamping

**Files:**
- Modify: `crates/harness/src/memory/mod.rs`

- [ ] **Step 1: Write the failing tests** (add to `memory/mod.rs` tests)

```rust
    #[test]
    fn prune_stale_drops_old_entries_and_empty_sections() {
        let mut m = Memory::default();
        m.remember("people", "old contact", 100);
        m.remember("people", "fresh contact", 900);
        m.remember("jobs", "ancient job", 50);
        let removed = m.prune_stale(1000, 500); // older than 500s ago goes
        assert_eq!(removed, 2);
        assert_eq!(m.section_texts("people"), vec!["fresh contact"]);
        assert!(m.sections.get("jobs").is_none());
    }

    #[test]
    fn clamp_to_cap_drops_oldest_first() {
        let mut m = Memory::default();
        m.remember("a", "one two three", 100); // 3 words, oldest
        m.remember("b", "four five", 200); // 2 words
        m.remember("c", "six seven eight nine", 300); // 4 words, newest
        let removed = m.clamp_to_cap(6);
        assert_eq!(removed, 2, "drops the two oldest entries to get under cap");
        assert_eq!(m.word_count(), 4);
        assert!(m.sections.get("a").is_none());
        assert!(m.sections.get("b").is_none());
        assert_eq!(m.section_texts("c"), vec!["six seven eight nine"]);
    }

    #[test]
    fn clamp_to_cap_noop_when_within() {
        let mut m = Memory::default();
        m.remember("a", "one two", 100);
        assert_eq!(m.clamp_to_cap(10), 0);
        assert_eq!(m.word_count(), 2);
    }
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p harness memory`
Expected: compile FAIL — `prune_stale`/`clamp_to_cap` not found.

- [ ] **Step 3: Implement** (add methods to `impl Memory`)

```rust
    /// Removes entries whose `last_touched` is older than `max_age_secs` before `now`
    /// (spec Rev 3: forgetting is a feature). Returns how many entries were removed.
    pub fn prune_stale(&mut self, now: u64, max_age_secs: u64) -> usize {
        let cutoff = now.saturating_sub(max_age_secs);
        let mut removed = 0;
        self.sections.retain(|_, entries| {
            let before = entries.len();
            entries.retain(|e| e.last_touched >= cutoff);
            removed += before - entries.len();
            !entries.is_empty()
        });
        removed
    }

    /// Drops oldest-touched entries (ties: section-name order) until `word_count() <= cap`.
    /// Returns how many entries were removed.
    pub fn clamp_to_cap(&mut self, cap: usize) -> usize {
        let mut removed = 0;
        while self.word_count() > cap {
            let oldest = self
                .sections
                .iter()
                .flat_map(|(name, entries)| {
                    entries.iter().map(move |e| (e.last_touched, name.clone(), e.text.clone()))
                })
                .min_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            let Some((_, section, text)) = oldest else { break };
            self.forget(&section, &text);
            removed += 1;
        }
        removed
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harness`
Expected: all pass (29 unit).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(harness): memory staleness pruning and oldest-first cap clamping"
```

---

### Task 4: FileMemoryStore

**Files:**
- Create: `crates/harness/src/memory/store.rs`
- Modify: `crates/harness/src/memory/mod.rs` (add `pub mod store;` at top), `crates/harness/src/lib.rs`

- [ ] **Step 1: Write the failing tests** (bottom of `memory/store.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("harness-memstore-{tag}-{nanos}.json"))
    }

    #[test]
    fn load_missing_file_returns_default() {
        let store = FileMemoryStore::new(temp_path("missing"));
        assert_eq!(store.load().unwrap(), Memory::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = temp_path("roundtrip");
        let store = FileMemoryStore::new(path.clone());
        let mut m = Memory::default();
        m.remember("vocabulary", "french drain", 42);
        store.save(&m).unwrap();
        assert_eq!(store.load().unwrap(), m);
        assert!(!path.with_extension("tmp").exists(), "temp file cleaned up by rename");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn corrupt_file_is_a_storage_error() {
        let path = temp_path("corrupt");
        std::fs::write(&path, "not json").unwrap();
        let store = FileMemoryStore::new(path.clone());
        assert!(matches!(store.load(), Err(HarnessError::Storage(_))));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn save_creates_parent_dirs() {
        let dir = temp_path("nested");
        let path = dir.join("memory.json");
        let store = FileMemoryStore::new(path.clone());
        store.save(&Memory::default()).unwrap();
        assert!(path.exists());
        std::fs::remove_dir_all(dir).ok();
    }
}
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p harness store`
Expected: compile FAIL.

- [ ] **Step 3: Implement** (above tests in `memory/store.rs`)

```rust
use std::path::PathBuf;

use crate::error::HarnessError;
use crate::memory::Memory;

/// Persistence seam for [`Memory`]. File-backed in production; swap for tests.
pub trait MemoryStore: Send + Sync {
    fn load(&self) -> Result<Memory, HarnessError>;
    fn save(&self, memory: &Memory) -> Result<(), HarnessError>;
}

/// JSON file store with atomic writes (write to `.tmp`, then rename).
pub struct FileMemoryStore {
    path: PathBuf,
}

impl FileMemoryStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        FileMemoryStore { path: path.into() }
    }
}

impl MemoryStore for FileMemoryStore {
    fn load(&self) -> Result<Memory, HarnessError> {
        if !self.path.exists() {
            return Ok(Memory::default());
        }
        let raw = std::fs::read_to_string(&self.path)
            .map_err(|e| HarnessError::Storage(format!("read {}: {e}", self.path.display())))?;
        serde_json::from_str(&raw)
            .map_err(|e| HarnessError::Storage(format!("parse {}: {e}", self.path.display())))
    }

    fn save(&self, memory: &Memory) -> Result<(), HarnessError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| HarnessError::Storage(format!("mkdir {}: {e}", parent.display())))?;
        }
        let json = serde_json::to_string_pretty(memory)
            .map_err(|e| HarnessError::Storage(format!("serialize memory: {e}")))?;
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, json)
            .map_err(|e| HarnessError::Storage(format!("write {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| HarnessError::Storage(format!("rename to {}: {e}", self.path.display())))
    }
}
```

`memory/mod.rs` — first line: `pub mod store;`
`lib.rs` — extend the memory re-export:
```rust
pub use memory::store::{FileMemoryStore, MemoryStore};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harness`
Expected: all pass (33 unit).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(harness): file-backed MemoryStore with atomic writes"
```

---

### Task 5: UpdateMemoryTool

**Files:**
- Create: `crates/harness/src/memory/tool.rs`
- Modify: `crates/harness/src/memory/mod.rs` (`pub mod tool;`), `crates/harness/src/lib.rs`

- [ ] **Step 1: Write the failing tests** (bottom of `memory/tool.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Memory;
    use crate::tool::Tool;
    use std::sync::Mutex as StdMutex;

    /// In-memory store that records saves.
    struct SpyStore {
        saved: StdMutex<Vec<Memory>>,
    }

    impl SpyStore {
        fn new() -> Arc<Self> {
            Arc::new(SpyStore { saved: StdMutex::new(Vec::new()) })
        }
    }

    impl MemoryStore for SpyStore {
        fn load(&self) -> Result<Memory, HarnessError> {
            Ok(Memory::default())
        }
        fn save(&self, memory: &Memory) -> Result<(), HarnessError> {
            self.saved.lock().unwrap().push(memory.clone());
            Ok(())
        }
    }

    fn tool_with(store: Arc<SpyStore>) -> (UpdateMemoryTool, Arc<Mutex<Memory>>) {
        let memory = Arc::new(Mutex::new(Memory::default()));
        let tool = UpdateMemoryTool::with_clock(memory.clone(), store, Arc::new(|| 777));
        (tool, memory)
    }

    #[tokio::test]
    async fn remember_mutates_and_persists() {
        let store = SpyStore::new();
        let (tool, memory) = tool_with(store.clone());
        let out = tool
            .execute(serde_json::json!({"op": "remember", "section": "people", "text": "Dev — framer"}))
            .await
            .unwrap();
        assert_eq!(out, "remembered in people: Dev — framer");
        let m = memory.lock().unwrap();
        assert_eq!(m.sections["people"][0].last_touched, 777);
        assert_eq!(store.saved.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn forget_removes_or_errors() {
        let store = SpyStore::new();
        let (tool, _memory) = tool_with(store.clone());
        tool.execute(serde_json::json!({"op": "remember", "section": "people", "text": "Dave"}))
            .await
            .unwrap();
        let out = tool
            .execute(serde_json::json!({"op": "forget", "section": "people", "text": "Dave"}))
            .await
            .unwrap();
        assert_eq!(out, "forgot from people: Dave");
        let err = tool
            .execute(serde_json::json!({"op": "forget", "section": "people", "text": "Dave"}))
            .await
            .unwrap_err();
        assert!(matches!(err, HarnessError::Tool { .. }));
    }

    #[tokio::test]
    async fn bad_input_is_a_tool_error() {
        let store = SpyStore::new();
        let (tool, _memory) = tool_with(store);
        let err = tool
            .execute(serde_json::json!({"op": "explode", "section": "x", "text": "y"}))
            .await
            .unwrap_err();
        assert!(matches!(err, HarnessError::Tool { .. }));
        let err = tool.execute(serde_json::json!({"op": "remember"})).await.unwrap_err();
        assert!(matches!(err, HarnessError::Tool { .. }));
    }
}
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p harness memory::tool`
Expected: compile FAIL.

- [ ] **Step 3: Implement** (above tests in `memory/tool.rs`)

```rust
use std::sync::{Arc, Mutex};

use crate::error::HarnessError;
use crate::memory::store::MemoryStore;
use crate::memory::Memory;
use crate::tool::Tool;

/// Injectable clock (unix seconds) so tests are deterministic.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

fn system_clock() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// In-session memory updates (spec §7): the agent remembers corrections and new
/// facts as they happen. Every mutation is persisted immediately via the store.
pub struct UpdateMemoryTool {
    memory: Arc<Mutex<Memory>>,
    store: Arc<dyn MemoryStore>,
    clock: Clock,
}

impl UpdateMemoryTool {
    pub fn new(memory: Arc<Mutex<Memory>>, store: Arc<dyn MemoryStore>) -> Self {
        Self::with_clock(memory, store, Arc::new(system_clock))
    }

    pub fn with_clock(
        memory: Arc<Mutex<Memory>>,
        store: Arc<dyn MemoryStore>,
        clock: Clock,
    ) -> Self {
        UpdateMemoryTool { memory, store, clock }
    }

    fn err(message: impl Into<String>) -> HarnessError {
        HarnessError::Tool { name: "update_memory".into(), message: message.into() }
    }
}

#[async_trait::async_trait]
impl Tool for UpdateMemoryTool {
    fn name(&self) -> &str {
        "update_memory"
    }

    fn description(&self) -> &str {
        "Remember or forget one fact about the user, their people, projects, vocabulary, or preferences. Use for corrections and durable facts, not session content."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "op": { "type": "string", "enum": ["remember", "forget"] },
                "section": { "type": "string", "description": "e.g. vocabulary, people, projects, preferences" },
                "text": { "type": "string", "description": "one short fact" }
            },
            "required": ["op", "section", "text"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<String, HarnessError> {
        let op = input["op"].as_str().ok_or_else(|| Self::err("missing 'op'"))?;
        let section = input["section"].as_str().ok_or_else(|| Self::err("missing 'section'"))?;
        let text = input["text"].as_str().ok_or_else(|| Self::err("missing 'text'"))?;

        let snapshot = {
            let mut mem = self.memory.lock().map_err(|_| Self::err("memory lock poisoned"))?;
            match op {
                "remember" => mem.remember(section, text, (self.clock)()),
                "forget" => {
                    if !mem.forget(section, text) {
                        return Err(Self::err(format!("no entry in {section} matching: {text}")));
                    }
                }
                other => return Err(Self::err(format!("unknown op: {other}"))),
            }
            mem.clone()
        };
        self.store.save(&snapshot)?;

        Ok(match op {
            "remember" => format!("remembered in {section}: {text}"),
            _ => format!("forgot from {section}: {text}"),
        })
    }
}
```

`memory/mod.rs` — after `pub mod store;`: `pub mod tool;`
`lib.rs` — add:
```rust
pub use memory::tool::UpdateMemoryTool;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harness`
Expected: all pass (36 unit).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(harness): update_memory tool with persistence and injectable clock"
```

---

### Task 6: Context assembler

**Files:**
- Create: `crates/harness/src/context.rs`
- Modify: `crates/harness/src/lib.rs`

- [ ] **Step 1: Write the failing tests** (bottom of `context.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approx_tokens_is_chars_over_four_rounded_up() {
        assert_eq!(approx_tokens(""), 0);
        assert_eq!(approx_tokens("abcd"), 1);
        assert_eq!(approx_tokens("abcde"), 2);
    }

    #[test]
    fn within_budget_sections_pass_through() {
        let out = ContextAssembler::assemble(&[
            ContextSection { title: "memory".into(), content: "knows stuff".into(), budget_tokens: 100 },
            ContextSection { title: "recent".into(), content: "items".into(), budget_tokens: 100 },
        ]);
        assert_eq!(out.text, "## memory\nknows stuff\n\n## recent\nitems");
        assert!(out.truncated_sections.is_empty());
        assert_eq!(out.approx_tokens, approx_tokens(&out.text));
    }

    #[test]
    fn over_budget_section_is_truncated_with_marker() {
        let long = "word ".repeat(100); // 500 chars
        let out = ContextAssembler::assemble(&[ContextSection {
            title: "transcript".into(),
            content: long,
            budget_tokens: 10, // 40 chars
        }]);
        assert_eq!(out.truncated_sections, vec!["transcript".to_string()]);
        assert!(out.text.contains("…[truncated]"));
        // content portion respects the budget: 40 chars + marker
        let body = out.text.strip_prefix("## transcript\n").unwrap();
        let content_part = body.strip_suffix("\n…[truncated]").unwrap();
        assert_eq!(content_part.chars().count(), 40);
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        let content = "é".repeat(50);
        let out = ContextAssembler::assemble(&[ContextSection {
            title: "t".into(),
            content,
            budget_tokens: 5, // 20 chars
        }]);
        assert!(out.text.contains(&"é".repeat(20)));
        assert_eq!(out.truncated_sections.len(), 1);
    }

    #[test]
    fn empty_sections_are_skipped() {
        let out = ContextAssembler::assemble(&[
            ContextSection { title: "empty".into(), content: String::new(), budget_tokens: 10 },
            ContextSection { title: "full".into(), content: "hi".into(), budget_tokens: 10 },
        ]);
        assert_eq!(out.text, "## full\nhi");
    }
}
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p harness context`
Expected: compile FAIL.

- [ ] **Step 3: Implement** (above tests in `context.rs`)

```rust
//! Token-budgeted context assembly (spec §4: token budget is a first-class
//! constraint). Uses a documented ~4-chars-per-token approximation — good
//! enough for budget enforcement; exact counts come from provider usage.

/// Approximate token count: ceil(chars / 4).
pub fn approx_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// One named, budgeted block of prompt context.
pub struct ContextSection {
    pub title: String,
    pub content: String,
    pub budget_tokens: usize,
}

pub struct AssembledContext {
    pub text: String,
    pub approx_tokens: usize,
    /// Titles of sections that had to be cut to fit their budget.
    pub truncated_sections: Vec<String>,
}

pub struct ContextAssembler;

impl ContextAssembler {
    /// Renders sections as `## title\ncontent`, truncating each to its own
    /// budget (by chars = tokens * 4) with a `…[truncated]` marker.
    /// Empty sections are skipped entirely.
    pub fn assemble(sections: &[ContextSection]) -> AssembledContext {
        let mut parts = Vec::new();
        let mut truncated_sections = Vec::new();

        for section in sections {
            if section.content.is_empty() {
                continue;
            }
            let budget_chars = section.budget_tokens * 4;
            let content = if section.content.chars().count() > budget_chars {
                truncated_sections.push(section.title.clone());
                let cut: String = section.content.chars().take(budget_chars).collect();
                format!("{cut}\n…[truncated]")
            } else {
                section.content.clone()
            };
            parts.push(format!("## {}\n{}", section.title, content));
        }

        let text = parts.join("\n\n");
        AssembledContext { approx_tokens: approx_tokens(&text), text, truncated_sections }
    }
}
```

`lib.rs` — add:
```rust
pub mod context;
pub use context::{approx_tokens, AssembledContext, ContextAssembler, ContextSection};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harness`
Expected: all pass (41 unit).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(harness): token-budgeted context assembler with truncation markers"
```

---

### Task 7: ReflectionPolicy

**Files:**
- Create: `crates/harness/src/reflection/policy.rs`, `crates/harness/src/reflection/mod.rs` (module shell)
- Modify: `crates/harness/src/lib.rs`

- [ ] **Step 1: Write the failing tests** (bottom of `reflection/policy.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn signals(sessions: u32, corrections: u32, completed: u32, churn: &[f32]) -> ReflectionSignals {
        ReflectionSignals {
            sessions_since_reflection: sessions,
            corrections_since_reflection: corrections,
            completed_reflections: completed,
            recent_churn: churn.to_vec(),
        }
    }

    #[test]
    fn no_new_sessions_means_no_reflection() {
        let p = ReflectionPolicy::default();
        assert!(!p.should_reflect(&signals(0, 0, 0, &[])));
        assert!(!p.should_reflect(&signals(0, 3, 10, &[0.0])), "even corrections wait for a session");
    }

    #[test]
    fn warmup_reflects_every_session() {
        let p = ReflectionPolicy::default(); // warmup_reflections = 5
        assert!(p.should_reflect(&signals(1, 0, 0, &[])));
        assert!(p.should_reflect(&signals(1, 0, 4, &[0.0, 0.0])));
    }

    #[test]
    fn corrections_snap_cadence_back() {
        let p = ReflectionPolicy::default();
        // post-warmup, dead-flat churn, but a correction happened
        assert!(p.should_reflect(&signals(1, 1, 20, &[0.0, 0.0, 0.0])));
    }

    #[test]
    fn low_churn_backs_off_exponentially() {
        let p = ReflectionPolicy::default(); // threshold 0.1, max 16
        // one low-churn reflection → interval 2
        assert!(!p.should_reflect(&signals(1, 0, 6, &[0.05])));
        assert!(p.should_reflect(&signals(2, 0, 6, &[0.05])));
        // three trailing low-churn → interval 8
        assert!(!p.should_reflect(&signals(7, 0, 9, &[0.05, 0.05, 0.05])));
        assert!(p.should_reflect(&signals(8, 0, 9, &[0.05, 0.05, 0.05])));
    }

    #[test]
    fn high_churn_keeps_every_session_cadence() {
        let p = ReflectionPolicy::default();
        assert!(p.should_reflect(&signals(1, 0, 10, &[0.5])));
        // trailing high churn resets the backoff even after earlier low ones
        assert!(p.should_reflect(&signals(1, 0, 10, &[0.05, 0.05, 0.5])));
    }

    #[test]
    fn interval_is_capped_at_max() {
        let p = ReflectionPolicy::default(); // max_interval_sessions = 16
        let flat = [0.0f32; 10];
        assert!(!p.should_reflect(&signals(15, 0, 30, &flat)));
        assert!(p.should_reflect(&signals(16, 0, 30, &flat)));
    }
}
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p harness policy`
Expected: compile FAIL.

- [ ] **Step 3: Implement**

`reflection/mod.rs` (shell for now; engine arrives in Task 8):
```rust
pub mod policy;
```

`reflection/policy.rs` (above tests):
```rust
//! Signal-driven reflection cadence (spec Rev 3): every session during warmup,
//! exponential backoff while reflections keep changing little, and any user
//! correction snaps cadence back to the next session end.
//!
//! The app layer owns persistence of these counters and calls
//! [`ReflectionPolicy::should_reflect`] whenever it has guaranteed compute
//! (session end / app open). Platform schedulers are out of scope here.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReflectionSignals {
    /// Sessions completed since the last reflection ran.
    pub sessions_since_reflection: u32,
    /// User corrections (edits of agent output) since the last reflection.
    pub corrections_since_reflection: u32,
    /// Total reflections ever completed (drives warmup).
    pub completed_reflections: u32,
    /// Churn scores of recent reflections, oldest→newest (see ReflectionOutcome::churn).
    pub recent_churn: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct ReflectionPolicy {
    /// Reflect after every session until this many reflections have run.
    pub warmup_reflections: u32,
    /// Churn below this counts as "nothing new learned".
    pub low_churn_threshold: f32,
    /// Ceiling for the backoff interval.
    pub max_interval_sessions: u32,
}

impl Default for ReflectionPolicy {
    fn default() -> Self {
        ReflectionPolicy {
            warmup_reflections: 5,
            low_churn_threshold: 0.1,
            max_interval_sessions: 16,
        }
    }
}

impl ReflectionPolicy {
    /// Sessions to wait before the next reflection: 1 during warmup or while
    /// churn stays high; doubles per consecutive trailing low-churn reflection.
    pub fn required_interval(&self, completed_reflections: u32, recent_churn: &[f32]) -> u32 {
        if completed_reflections < self.warmup_reflections {
            return 1;
        }
        let trailing_low = recent_churn
            .iter()
            .rev()
            .take_while(|c| **c < self.low_churn_threshold)
            .count() as u32;
        (1u32 << trailing_low.min(6)).min(self.max_interval_sessions)
    }

    pub fn should_reflect(&self, s: &ReflectionSignals) -> bool {
        if s.sessions_since_reflection == 0 {
            return false;
        }
        if s.corrections_since_reflection > 0 {
            return true;
        }
        s.sessions_since_reflection
            >= self.required_interval(s.completed_reflections, &s.recent_churn)
    }
}
```

`lib.rs` — add:
```rust
pub mod reflection;
pub use reflection::policy::{ReflectionPolicy, ReflectionSignals};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harness`
Expected: all pass (47 unit).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(harness): signal-driven reflection cadence policy"
```

---

### Task 8: ReflectionEngine

**Files:**
- Create: `crates/harness/src/reflection/engine.rs`
- Modify: `crates/harness/src/reflection/mod.rs`, `crates/harness/src/lib.rs`

- [ ] **Step 1: Write the failing tests** (bottom of `reflection/engine.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::*;
    use crate::mock::MockProvider;
    use std::sync::Arc;

    fn write_memory_response(sections: serde_json::Value) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "write_memory".into(),
                input: serde_json::json!({ "sections": sections }),
            }],
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 100, output_tokens: 50 },
        }
    }

    fn current_memory() -> Memory {
        let mut m = Memory::default();
        m.remember("people", "Dev — framer", 111);
        m.remember("people", "Dave — plumber", 222);
        m
    }

    #[tokio::test]
    async fn rebuilds_memory_preserving_last_touched_for_kept_texts() {
        let provider = Arc::new(MockProvider::new(vec![write_memory_response(
            serde_json::json!({ "people": ["Dev — framer", "Sara — electrician"] }),
        )]));
        let engine = ReflectionEngine::new(provider.clone());

        let out = engine
            .reflect(&current_memory(), &["walked the Johnson site".into()], 999)
            .await
            .unwrap();

        let people = &out.memory.sections["people"];
        assert_eq!(people.len(), 2);
        assert_eq!(people[0], MemoryEntry { text: "Dev — framer".into(), last_touched: 111 });
        assert_eq!(people[1], MemoryEntry { text: "Sara — electrician".into(), last_touched: 999 });
        assert_eq!(out.usage, Usage { input_tokens: 100, output_tokens: 50 });

        // request shape: forced tool, memory + activity present
        let reqs = provider.requests();
        assert_eq!(reqs[0].tool_choice.as_deref(), Some("write_memory"));
        let ContentBlock::Text { text } = &reqs[0].messages[0].content[0] else {
            panic!("expected text block")
        };
        assert!(text.contains("Dev — framer"));
        assert!(text.contains("walked the Johnson site"));
    }

    #[tokio::test]
    async fn churn_measures_added_plus_removed() {
        // old: {Dev, Dave}; new: {Dev, Sara} → added 1, removed 1, sizes 2+2 → churn 0.5
        let provider = Arc::new(MockProvider::new(vec![write_memory_response(
            serde_json::json!({ "people": ["Dev — framer", "Sara — electrician"] }),
        )]));
        let engine = ReflectionEngine::new(provider);
        let out = engine.reflect(&current_memory(), &[], 999).await.unwrap();
        assert!((out.churn - 0.5).abs() < 1e-6);
    }

    #[tokio::test]
    async fn identical_rewrite_has_zero_churn() {
        let provider = Arc::new(MockProvider::new(vec![write_memory_response(
            serde_json::json!({ "people": ["Dev — framer", "Dave — plumber"] }),
        )]));
        let engine = ReflectionEngine::new(provider);
        let out = engine.reflect(&current_memory(), &[], 999).await.unwrap();
        assert_eq!(out.churn, 0.0);
    }

    #[tokio::test]
    async fn result_is_clamped_to_word_cap() {
        let provider = Arc::new(MockProvider::new(vec![write_memory_response(
            serde_json::json!({ "notes": ["one two three", "four five six seven"] }),
        )]));
        let mut engine = ReflectionEngine::new(provider);
        engine.word_cap = 4;
        let out = engine.reflect(&Memory::default(), &[], 999).await.unwrap();
        assert!(out.memory.word_count() <= 4);
    }

    #[tokio::test]
    async fn missing_tool_call_is_an_error() {
        let provider = Arc::new(MockProvider::new(vec![CompletionResponse {
            content: vec![ContentBlock::Text { text: "I decline".into() }],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
        }]));
        let engine = ReflectionEngine::new(provider);
        let err = engine.reflect(&Memory::default(), &[], 999).await.unwrap_err();
        assert!(matches!(err, HarnessError::Provider(msg) if msg.contains("write_memory")));
    }
}
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p harness engine`
Expected: compile FAIL.

- [ ] **Step 3: Implement** (above tests in `reflection/engine.rs`)

```rust
//! LLM-driven reflection (spec §7): reads current memory + recent activity,
//! REPLACES the memory (compress, don't accumulate), preserving last_touched
//! for texts that survive. Returns a churn score the cadence policy consumes.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::error::HarnessError;
use crate::llm::{
    CompletionRequest, ContentBlock, LlmProvider, Message, ToolSpec, Usage,
};
use crate::memory::{Memory, DEFAULT_WORD_CAP};

const WRITE_MEMORY: &str = "write_memory";

pub struct ReflectionOutcome {
    pub memory: Memory,
    /// (added + removed) / (old_count + new_count); 0.0 when both are empty.
    pub churn: f32,
    pub usage: Usage,
}

pub struct ReflectionEngine {
    provider: Arc<dyn LlmProvider>,
    pub word_cap: usize,
    pub max_tokens: u32,
}

impl ReflectionEngine {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        ReflectionEngine { provider, word_cap: DEFAULT_WORD_CAP, max_tokens: 2048 }
    }

    fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: WRITE_MEMORY.into(),
            description: "Write the complete updated memory. This REPLACES all sections."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "sections": {
                        "type": "object",
                        "description": "section name -> list of short fact strings",
                        "additionalProperties": { "type": "array", "items": { "type": "string" } }
                    }
                },
                "required": ["sections"]
            }),
        }
    }

    fn system_prompt(&self) -> String {
        format!(
            "You maintain a compact long-term memory about one user for a field-work \
             assistant. Rewrite the ENTIRE memory: keep what stays true, integrate what \
             the recent activity shows, drop what is stale or disproven. Prefer fewer, \
             sharper facts. Hard limit: {} words total. Typical sections: vocabulary, \
             people, projects, preferences. Call {} exactly once with the full result.",
            self.word_cap, WRITE_MEMORY
        )
    }

    pub async fn reflect(
        &self,
        current: &Memory,
        activity: &[String],
        now: u64,
    ) -> Result<ReflectionOutcome, HarnessError> {
        let activity_block = if activity.is_empty() {
            "(none)".to_string()
        } else {
            activity
                .iter()
                .enumerate()
                .map(|(i, a)| format!("{}. {a}", i + 1))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let memory_block = if current.sections.is_empty() {
            "(empty)".to_string()
        } else {
            current.to_prompt()
        };
        let user = format!(
            "Current memory:\n{memory_block}\n\nRecent activity since last reflection:\n{activity_block}"
        );

        let response = self
            .provider
            .complete(CompletionRequest {
                system: self.system_prompt(),
                messages: vec![Message::user_text(user)],
                tools: vec![self.tool_spec()],
                max_tokens: self.max_tokens,
                tool_choice: Some(WRITE_MEMORY.into()),
            })
            .await?;

        let sections = response
            .content
            .iter()
            .find_map(|b| match b {
                ContentBlock::ToolUse { name, input, .. } if name == WRITE_MEMORY => {
                    input.get("sections").and_then(|s| s.as_object()).cloned()
                }
                _ => None,
            })
            .ok_or_else(|| {
                HarnessError::Provider("reflection response missing write_memory call".into())
            })?;

        let mut memory = Memory::default();
        for (section, texts) in &sections {
            let Some(texts) = texts.as_array() else { continue };
            for text in texts.iter().filter_map(|t| t.as_str()) {
                let last_touched = current
                    .sections
                    .get(section)
                    .and_then(|es| es.iter().find(|e| e.text == text))
                    .map(|e| e.last_touched)
                    .unwrap_or(now);
                memory.remember(section, text, last_touched);
            }
        }
        memory.clamp_to_cap(self.word_cap);

        let churn = churn_between(current, &memory);
        Ok(ReflectionOutcome { memory, churn, usage: response.usage })
    }
}

/// (added + removed) / (old_count + new_count), 0.0 when both sides are empty.
fn churn_between(old: &Memory, new: &Memory) -> f32 {
    let keys = |m: &Memory| -> BTreeSet<(String, String)> {
        m.sections
            .iter()
            .flat_map(|(s, es)| es.iter().map(move |e| (s.clone(), e.text.clone())))
            .collect()
    };
    let old_keys = keys(old);
    let new_keys = keys(new);
    let denominator = old_keys.len() + new_keys.len();
    if denominator == 0 {
        return 0.0;
    }
    let added = new_keys.difference(&old_keys).count();
    let removed = old_keys.difference(&new_keys).count();
    (added + removed) as f32 / denominator as f32
}
```

`reflection/mod.rs` — replace with:
```rust
pub mod engine;
pub mod policy;
```

`lib.rs` — extend reflection exports:
```rust
pub use reflection::engine::{ReflectionEngine, ReflectionOutcome};
```

Also add `MemoryEntry` to the memory re-export in lib.rs if not already present (Task 2 exported it).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harness`
Expected: all pass (52 unit + 1 integration).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(harness): LLM reflection engine with churn scoring and cap clamping"
```

---

### Task 9: Docs + full verification

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update README plan-series line**

Replace the line `Next: 02 memory + reflection + context assembler.` with:
```markdown
Done: 01 foundation, 02 memory + reflection + context assembler.
Next: 03 murmur-core domain + storage.
```

- [ ] **Step 2: Full verification**

Run: `cargo test` → all pass. Run: `cargo clippy --all-targets` → zero warnings. Fix any clippy findings mechanically (do not change public API shapes); if a fix would change behavior, STOP and report.

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "docs: plan series status; clippy clean for plan 02"
```

---

## Rev 2 amendments (frontier research, 2026-07-02)

Source: `docs/research/2026-07-02-agent-memory-frontier.md`. Adopted: snapshots (SSGM dual-track rollback), per-fact provenance, importance-aware forgetting, reflection prompt hardening. The controller merges these replacements into task text at dispatch time; where a block below conflicts with the original task, **this section wins**.

### A. Replaces Task 2's `MemoryEntry` and `remember` (and their tests)

```rust
/// Where a fact came from — drives eviction priority and debuggability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FactSource {
    /// The agent's own deduction. First to be evicted.
    Inferred,
    /// The user said it outright.
    Stated,
    /// The user corrected the agent. Never auto-pruned; last to be evicted.
    Corrected,
}

impl FactSource {
    pub fn rank(self) -> u8 {
        match self {
            FactSource::Inferred => 0,
            FactSource::Stated => 1,
            FactSource::Corrected => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub text: String,
    /// Unix seconds when this entry was last added, confirmed, or re-mentioned.
    pub last_touched: u64,
    pub source: FactSource,
    /// Session id this fact came from, if known.
    pub session: Option<String>,
}
```

`remember` keeps its simple signature (defaults: `Inferred`, no session) and delegates to the full version. On an existing exact text: refresh `last_touched`; upgrade `source`/`session` only if the new source ranks higher (never downgrade):

```rust
    pub fn remember(&mut self, section: &str, text: &str, now: u64) {
        self.remember_from(section, text, now, FactSource::Inferred, None);
    }

    pub fn remember_from(
        &mut self,
        section: &str,
        text: &str,
        now: u64,
        source: FactSource,
        session: Option<String>,
    ) {
        let entries = self.sections.entry(section.to_string()).or_default();
        match entries.iter_mut().find(|e| e.text == text) {
            Some(e) => {
                e.last_touched = now;
                if source.rank() > e.source.rank() {
                    e.source = source;
                    e.session = session;
                }
            }
            None => entries.push(MemoryEntry {
                text: text.to_string(),
                last_touched: now,
                source,
                session,
            }),
        }
    }
```

Task 2 test updates: in `remember_adds_and_touches_existing` also assert `m.sections["people"][0].source == FactSource::Inferred`; add:

```rust
    #[test]
    fn source_upgrades_but_never_downgrades() {
        let mut m = Memory::default();
        m.remember_from("people", "Dev — framer", 100, FactSource::Corrected, Some("s3".into()));
        m.remember("people", "Dev — framer", 200); // inferred touch
        let e = &m.sections["people"][0];
        assert_eq!(e.last_touched, 200);
        assert_eq!(e.source, FactSource::Corrected);
        assert_eq!(e.session.as_deref(), Some("s3"));
        m.remember_from("people", "Dev — framer", 300, FactSource::Stated, Some("s9".into()));
        assert_eq!(m.sections["people"][0].source, FactSource::Corrected, "no downgrade");
    }
```

lib.rs also re-exports `FactSource`.

### B. Replaces Task 3's eviction/pruning semantics (and adds tests)

`prune_stale` never removes `Corrected` entries. `clamp_to_cap` evicts by ascending `(source.rank(), last_touched, section name)` — inferred-oldest first, corrected last:

```rust
    pub fn prune_stale(&mut self, now: u64, max_age_secs: u64) -> usize {
        let cutoff = now.saturating_sub(max_age_secs);
        let mut removed = 0;
        self.sections.retain(|_, entries| {
            let before = entries.len();
            entries.retain(|e| e.source == FactSource::Corrected || e.last_touched >= cutoff);
            removed += before - entries.len();
            !entries.is_empty()
        });
        removed
    }

    pub fn clamp_to_cap(&mut self, cap: usize) -> usize {
        let mut removed = 0;
        while self.word_count() > cap {
            let next = self
                .sections
                .iter()
                .flat_map(|(name, entries)| {
                    entries
                        .iter()
                        .map(move |e| ((e.source.rank(), e.last_touched, name.clone()), e.text.clone()))
                })
                .min_by(|a, b| a.0.cmp(&b.0));
            let Some(((_, _, section), text)) = next else { break };
            self.forget(&section, &text);
            removed += 1;
        }
        removed
    }
```

Additional Task 3 tests:

```rust
    #[test]
    fn corrected_facts_survive_pruning_and_evict_last() {
        let mut m = Memory::default();
        m.remember_from("people", "Dev not Dave", 10, FactSource::Corrected, None);
        m.remember("people", "likes early starts", 999);
        assert_eq!(m.prune_stale(1000, 100), 1, "only the inferred fact prunes");
        assert_eq!(m.section_texts("people"), vec!["Dev not Dave"]);

        m.remember_from("a", "one two three four", 500, FactSource::Stated, None);
        // cap forces eviction: inferred gone already; stated (rank 1) goes before corrected (rank 2)
        m.clamp_to_cap(3);
        assert_eq!(m.section_texts("people"), vec!["Dev not Dave"]);
        assert!(m.sections.get("a").is_none());
    }
```

(The original `clamp_to_cap_drops_oldest_first` test still passes: all-Inferred entries fall back to oldest-first.)

### C. Adds snapshot rotation to Task 4's FileMemoryStore (and a test)

Every `save` rotates up to 3 prior versions (`.1` newest … `.3` oldest) before the atomic rename; `snapshots()` returns whichever parse cleanly, newest first:

```rust
    fn rotate_snapshots(&self) {
        for i in (1..3usize).rev() {
            let from = self.path.with_extension(i.to_string());
            let to = self.path.with_extension((i + 1).to_string());
            if from.exists() {
                let _ = std::fs::rename(&from, &to);
            }
        }
        if self.path.exists() {
            let _ = std::fs::copy(&self.path, self.path.with_extension("1"));
        }
    }

    /// Prior memory versions, newest first (up to 3). Rollback path if a
    /// reflection rewrite dropped something important (research rec #8).
    pub fn snapshots(&self) -> Vec<Memory> {
        (1..=3usize)
            .filter_map(|i| std::fs::read_to_string(self.path.with_extension(i.to_string())).ok())
            .filter_map(|raw| serde_json::from_str(&raw).ok())
            .collect()
    }
```

Call `self.rotate_snapshots();` as the first line of `save` (before parent-dir creation). Add test:

```rust
    #[test]
    fn save_rotates_up_to_three_snapshots() {
        let path = temp_path("snapshots");
        let store = FileMemoryStore::new(path.clone());
        for i in 0..5u64 {
            let mut m = Memory::default();
            m.remember("v", &format!("version {i}"), i);
            store.save(&m).unwrap();
        }
        let snaps = store.snapshots();
        assert_eq!(snaps.len(), 3);
        assert_eq!(snaps[0].section_texts("v"), vec!["version 3"]);
        assert_eq!(snaps[2].section_texts("v"), vec!["version 1"]);
        for ext in ["1", "2", "3"] {
            std::fs::remove_file(path.with_extension(ext)).ok();
        }
        std::fs::remove_file(path).ok();
    }
```

### D. Task 5 tool gains optional `source` and a session tag

`UpdateMemoryTool` gets a `session: Option<String>` field (`new`/`with_clock` set it to `None`; add builder `pub fn for_session(mut self, id: impl Into<String>) -> Self`). Input schema `properties` gains:

```json
"source": { "type": "string", "enum": ["stated", "inferred", "corrected"], "description": "how you know this; default inferred" }
```

(`required` unchanged.) In `execute`, parse `source` (default `Inferred`; unknown value → tool error) and call `mem.remember_from(section, text, (self.clock)(), source, self.session.clone())`. Add test:

```rust
    #[tokio::test]
    async fn source_and_session_are_recorded() {
        let store = SpyStore::new();
        let memory = Arc::new(Mutex::new(Memory::default()));
        let tool = UpdateMemoryTool::with_clock(memory.clone(), store, Arc::new(|| 5))
            .for_session("s42");
        tool.execute(serde_json::json!({"op": "remember", "section": "people", "text": "Dev", "source": "corrected"}))
            .await
            .unwrap();
        let m = memory.lock().unwrap();
        assert_eq!(m.sections["people"][0].source, FactSource::Corrected);
        assert_eq!(m.sections["people"][0].session.as_deref(), Some("s42"));
    }
```

(Import `FactSource` in the tool module.)

### E. Task 8 engine: preserve full provenance; hardened prompt

Where the engine rebuilds entries, surviving exact-text facts keep their whole prior entry (source, session, last_touched), and new facts are `Inferred`/no-session:

```rust
                let prior = current
                    .sections
                    .get(section)
                    .and_then(|es| es.iter().find(|e| e.text == text))
                    .cloned();
                match prior {
                    Some(e) => memory.remember_from(section, text, e.last_touched, e.source, e.session),
                    None => memory.remember_from(section, text, now, FactSource::Inferred, None),
                }
```

System prompt gains, after the word-limit sentence:

```
Keep facts that survive VERBATIM, character for character — do not paraphrase them. \
When recent activity contradicts an existing fact, drop the stale fact and write the \
corrected one; never merge the two into a blended claim. Facts marked as user \
corrections outrank everything else — do not drop or alter them.
```

The `reflect` user message renders corrected facts with a marker so the model can honor that rule — in the memory block, append ` [corrected]` after entries whose `source == FactSource::Corrected` (do this in `reflect` when building `memory_block`, not in `Memory::to_prompt`, which stays clean for general context use). Task 8 test update: `rebuilds_memory_preserving_last_touched_for_kept_texts` asserts the surviving entry equals the FULL prior entry (source/session preserved); churn/clamp/missing-tool tests unchanged. Churn remains logged-not-trusted: the verbatim rule exists precisely so paraphrase noise doesn't fake churn (research rec #6).

### F. Spec-level note (no code)

Vocabulary curation (≤100 short, phonetically-confusable, domain-specific terms — iOS `contextualStrings` limit) is enforced where vocabulary is consumed and written: reflection prompt guidance lands in Plan 03's murmur-core prompts, STT-side enforcement in Plan 05.

## Self-Review Notes

- **Spec coverage:** §7 memory file ✓ (Tasks 2–4), update_memory tool ✓ (Task 5), reflection replace-not-append ✓ (Task 8), 500-word cap ✓ (Tasks 2/3/8), Rev 3 adaptive cadence + corrections snap-back ✓ (Task 7), Rev 3 forgetting (staleness prune + cap + quieter-not-chattier is prompt-level, noted) ✓ (Task 3), §4 context assembler with budgets ✓ (Task 6), vocabulary→STT accessor seam ✓ (`section_texts`, Task 2). Memory-transparency UI and platform scheduling are app-layer (Plans 03+/06) — out of scope here by design.
- **Type consistency:** `CompletionRequest.tool_choice: Option<String>` added Task 1, consumed Task 8; `Memory::remember/forget/clamp_to_cap/section_texts` defined Tasks 2–3, used Tasks 5 and 8; `MemoryStore` trait defined Task 4, consumed Task 5 (`Arc<dyn MemoryStore>`); `HarnessError::Storage` added Task 1, used Task 4; churn formula in Task 8 matches the doc comment and the 0.5/0.0 tests.
- **Test-count checkpoints** (19 unit at start): T1 +1=20, T2 +6=26, T3 +3=29, T4 +4=33, T5 +3=36, T6 +5=41, T7 +6=47, T8 +5=52. Counts are expectations, not gates — if a count differs but all tests pass and coverage matches the task, proceed.
- **Known judgment calls for reviewers:** exact-string entry matching (no fuzzy dedup — YAGNI until real data shows the need); churn counts entry add/remove only (a reworded fact = 1 added + 1 removed, which correctly reads as churn); `Clock` as `Arc<dyn Fn>` rather than a trait (smallest thing that makes time injectable).
