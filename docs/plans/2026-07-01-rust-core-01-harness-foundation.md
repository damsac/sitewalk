# Murmur Rust Core — Plan 01: Harness Foundation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create the fresh Murmur rebuild repo and the reusable `harness` crate: provider-agnostic LLM types, a mock provider, a tool registry, an agent loop with per-turn usage accounting, and a real Anthropic (BYOK) provider — all test-first.

**Architecture:** Rust workspace at `~/murmur-rmp` (fresh repo, `main` branch). `crates/harness` holds zero app-specific logic (spec §4): consumers register tools and prompts. Message/content types mirror the Anthropic wire shape (text / tool_use / tool_result blocks; tool results travel in user-role messages) but are provider-agnostic — each provider adapts. The agent loop is: call provider → execute any tool_use blocks → feed tool_result blocks back → repeat until a text-only response or max-turns. Tool failures become `is_error` tool results the model sees (spec R7 resilience), never a crashed loop. Token usage is accumulated on every turn (spec R9: cost measured from day one).

**Tech Stack:** Rust 2021, tokio (tests + client), serde/serde_json, thiserror, async-trait, reqwest (rustls-tls), wiremock (dev), plan sequence lives in this doc series (01–06).

**Spec:** `docs/superpowers/specs/2026-07-01-murmur-rebuild-vision-design.md` (Murmur repo). This plan covers spec §4 "harness crate" items: agent loop, provider-agnostic LLM client, tool registry. Memory/reflection, context assembler, and layout protocol come in Plans 02 and 06.

---

## File Structure

```
~/murmur-rmp/
  Cargo.toml                      # workspace
  rust-toolchain.toml
  .gitignore
  README.md
  crates/harness/
    Cargo.toml
    src/
      lib.rs                      # module wiring + re-exports
      error.rs                    # HarnessError
      llm.rs                      # Role, ContentBlock, Message, CompletionRequest/Response, Usage, LlmProvider trait
      mock.rs                     # MockProvider (scripted responses, records requests) — also used by downstream crates' tests
      tool.rs                     # Tool trait, ToolRegistry
      agent.rs                    # Agent, AgentConfig, TurnOutcome — the loop
      providers/
        mod.rs
        anthropic.rs              # AnthropicProvider (BYOK)
```

One responsibility per file; `mock.rs` ships in the library (not `#[cfg(test)]`) because Plans 02–04 test against it.

---

### Task 1: Repo + workspace scaffold

**Files:**
- Create: `~/murmur-rmp/Cargo.toml`, `~/murmur-rmp/rust-toolchain.toml`, `~/murmur-rmp/.gitignore`, `~/murmur-rmp/README.md`, `~/murmur-rmp/crates/harness/Cargo.toml`, `~/murmur-rmp/crates/harness/src/lib.rs`

- [ ] **Step 1: Initialize the repo**

```bash
mkdir -p ~/murmur-rmp/crates/harness/src
cd ~/murmur-rmp
git init -b main
```

- [ ] **Step 2: Write workspace files**

`Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/harness"]

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
async-trait = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
wiremock = "0.6"
```

`rust-toolchain.toml`:
```toml
[toolchain]
channel = "stable"
```

`.gitignore`:
```
/target
**/*.rs.bk
.DS_Store
```

`README.md`:
```markdown
# Murmur (rebuild)

AI meeting notes for blue-collar field work. Rust core workspace.

- `crates/harness` — reusable agent harness (no app-specific logic)

Vision spec + plan series live in the Murmur meta repo under `docs/superpowers/`.
```

`crates/harness/Cargo.toml`:
```toml
[package]
name = "harness"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
async-trait = { workspace = true }
reqwest = { workspace = true }

[dev-dependencies]
tokio = { workspace = true }
wiremock = { workspace = true }
```

`crates/harness/src/lib.rs`:
```rust
pub mod error;
```
(placeholder module comes in Task 2 — create `src/error.rs` with `// filled in Task 2` empty content is NOT allowed; instead make lib.rs empty for now:)

Correction — keep Task 1 compiling on its own. `crates/harness/src/lib.rs`:
```rust
// modules are added task by task
```

- [ ] **Step 3: Verify it builds**

Run: `cd ~/murmur-rmp && cargo check`
Expected: `Finished` with no errors.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "chore: workspace scaffold with harness crate"
```

---

### Task 2: Error type + LLM message types

**Files:**
- Create: `crates/harness/src/error.rs`, `crates/harness/src/llm.rs`
- Modify: `crates/harness/src/lib.rs`

- [ ] **Step 1: Write the failing test** (bottom of `crates/harness/src/llm.rs`, written together with the types below — TDD here means the test drives the serde shape; write the test first in the file, watch it fail to compile, then fill types)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_block_serializes_to_anthropic_wire_shape() {
        let block = ContentBlock::ToolUse {
            id: "tu_1".into(),
            name: "create_item".into(),
            input: serde_json::json!({"title": "mulch beds"}),
        };
        let v = serde_json::to_value(&block).unwrap();
        assert_eq!(v["type"], "tool_use");
        assert_eq!(v["name"], "create_item");
        let back: ContentBlock = serde_json::from_value(v).unwrap();
        assert_eq!(back, block);
    }

    #[test]
    fn tool_result_round_trips() {
        let block = ContentBlock::ToolResult {
            tool_use_id: "tu_1".into(),
            content: "ok".into(),
            is_error: false,
        };
        let v = serde_json::to_value(&block).unwrap();
        assert_eq!(v["type"], "tool_result");
        let back: ContentBlock = serde_json::from_value(v).unwrap();
        assert_eq!(back, block);
    }

    #[test]
    fn usage_adds() {
        let mut u = Usage { input_tokens: 10, output_tokens: 5 };
        u.add(&Usage { input_tokens: 3, output_tokens: 7 });
        assert_eq!(u, Usage { input_tokens: 13, output_tokens: 12 });
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p harness`
Expected: compile FAIL (`ContentBlock` not found).

- [ ] **Step 3: Write the types**

`crates/harness/src/error.rs`:
```rust
#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    #[error("provider error: {0}")]
    Provider(String),
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("tool '{name}' failed: {message}")]
    Tool { name: String, message: String },
    #[error("agent exceeded max turns ({0})")]
    MaxTurns(usize),
}
```

`crates/harness/src/llm.rs` (above the tests):
```rust
use serde::{Deserialize, Serialize};

use crate::error::HarnessError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn user_text(text: impl Into<String>) -> Self {
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompletionRequest {
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, other: &Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompletionResponse {
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    pub usage: Usage,
}

#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, HarnessError>;
}
```

`crates/harness/src/lib.rs`:
```rust
pub mod error;
pub mod llm;

pub use error::HarnessError;
pub use llm::{
    CompletionRequest, CompletionResponse, ContentBlock, LlmProvider, Message, Role, StopReason,
    ToolSpec, Usage,
};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harness`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(harness): LLM message types, provider trait, error type"
```

---

### Task 3: MockProvider

**Files:**
- Create: `crates/harness/src/mock.rs`
- Modify: `crates/harness/src/lib.rs`

Ships in the library (not cfg(test)) so downstream crates (Plans 02–04) can script agent behavior in their tests.

- [ ] **Step 1: Write the failing test** (bottom of `crates/harness/src/mock.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::*;

    fn text_response(s: &str) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::Text { text: s.into() }],
            stop_reason: StopReason::EndTurn,
            usage: Usage { input_tokens: 1, output_tokens: 1 },
        }
    }

    #[tokio::test]
    async fn returns_scripted_responses_in_order_and_records_requests() {
        let mock = MockProvider::new(vec![text_response("one"), text_response("two")]);
        let req = CompletionRequest {
            system: "sys".into(),
            messages: vec![Message::user_text("hi")],
            tools: vec![],
            max_tokens: 100,
        };
        let r1 = mock.complete(req.clone()).await.unwrap();
        let r2 = mock.complete(req.clone()).await.unwrap();
        assert_eq!(r1.content, vec![ContentBlock::Text { text: "one".into() }]);
        assert_eq!(r2.content, vec![ContentBlock::Text { text: "two".into() }]);
        assert_eq!(mock.requests().len(), 2);
        assert_eq!(mock.requests()[0].system, "sys");
    }

    #[tokio::test]
    async fn errors_when_script_is_exhausted() {
        let mock = MockProvider::new(vec![]);
        let req = CompletionRequest {
            system: String::new(),
            messages: vec![],
            tools: vec![],
            max_tokens: 1,
        };
        let err = mock.complete(req).await.unwrap_err();
        assert!(matches!(err, crate::HarnessError::Provider(_)));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p harness mock`
Expected: compile FAIL (`MockProvider` not found).

- [ ] **Step 3: Implement**

`crates/harness/src/mock.rs` (above tests):
```rust
use std::collections::VecDeque;
use std::sync::Mutex;

use crate::error::HarnessError;
use crate::llm::{CompletionRequest, CompletionResponse, LlmProvider};

pub struct MockProvider {
    responses: Mutex<VecDeque<CompletionResponse>>,
    requests: Mutex<Vec<CompletionRequest>>,
}

impl MockProvider {
    pub fn new(responses: Vec<CompletionResponse>) -> Self {
        MockProvider {
            responses: Mutex::new(responses.into()),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub fn requests(&self) -> Vec<CompletionRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl LlmProvider for MockProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, HarnessError> {
        self.requests.lock().unwrap().push(req);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| HarnessError::Provider("mock script exhausted".into()))
    }
}
```

`crates/harness/src/lib.rs` — add:
```rust
pub mod mock;
pub use mock::MockProvider;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harness`
Expected: all pass (5 total).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(harness): MockProvider for scripted agent tests"
```

---

### Task 4: Tool trait + ToolRegistry

**Files:**
- Create: `crates/harness/src/tool.rs`
- Modify: `crates/harness/src/lib.rs`

- [ ] **Step 1: Write the failing test** (bottom of `crates/harness/src/tool.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    #[async_trait::async_trait]
    impl Tool for Echo {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "echoes the input back"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            })
        }
        async fn execute(&self, input: serde_json::Value) -> Result<String, HarnessError> {
            Ok(input["text"].as_str().unwrap_or_default().to_string())
        }
    }

    #[tokio::test]
    async fn dispatches_by_name() {
        let mut reg = ToolRegistry::new();
        reg.register(Echo);
        let out = reg
            .execute("echo", serde_json::json!({"text": "hi"}))
            .await
            .unwrap();
        assert_eq!(out, "hi");
    }

    #[tokio::test]
    async fn unknown_tool_is_an_error() {
        let reg = ToolRegistry::new();
        let err = reg.execute("nope", serde_json::json!({})).await.unwrap_err();
        assert!(matches!(err, HarnessError::UnknownTool(n) if n == "nope"));
    }

    #[test]
    fn specs_lists_registered_tools() {
        let mut reg = ToolRegistry::new();
        reg.register(Echo);
        let specs = reg.specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "echo");
        assert_eq!(specs[0].description, "echoes the input back");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p harness tool`
Expected: compile FAIL (`ToolRegistry` not found).

- [ ] **Step 3: Implement**

`crates/harness/src/tool.rs` (above tests):
```rust
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::HarnessError;
use crate::llm::ToolSpec;

#[async_trait::async_trait]
pub trait Tool: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> serde_json::Value;
    async fn execute(&self, input: serde_json::Value) -> Result<String, HarnessError>;
}

#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: impl Tool) {
        self.tools.insert(tool.name().to_string(), Arc::new(tool));
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools
            .values()
            .map(|t| ToolSpec {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
            })
            .collect()
    }

    pub async fn execute(
        &self,
        name: &str,
        input: serde_json::Value,
    ) -> Result<String, HarnessError> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| HarnessError::UnknownTool(name.to_string()))?;
        tool.execute(input).await
    }
}
```

`crates/harness/src/lib.rs` — add:
```rust
pub mod tool;
pub use tool::{Tool, ToolRegistry};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harness`
Expected: all pass (8 total).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(harness): Tool trait and ToolRegistry"
```

---

### Task 5: Agent loop

**Files:**
- Create: `crates/harness/src/agent.rs`
- Modify: `crates/harness/src/lib.rs`

Loop contract (spec §4, R7): call provider → if response has tool_use blocks, execute each via the registry, append assistant message + user message of tool_results, repeat. Tool failure (including unknown tool) becomes an `is_error: true` tool_result the model sees; only provider errors and max-turns abort. Usage accumulates across turns.

- [ ] **Step 1: Write the failing tests** (bottom of `crates/harness/src/agent.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::*;
    use crate::mock::MockProvider;
    use crate::tool::{Tool, ToolRegistry};
    use std::sync::{Arc, Mutex};

    fn usage1() -> Usage {
        Usage { input_tokens: 10, output_tokens: 20 }
    }

    fn text_end(s: &str) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::Text { text: s.into() }],
            stop_reason: StopReason::EndTurn,
            usage: usage1(),
        }
    }

    fn tool_call(name: &str, input: serde_json::Value) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: name.into(),
                input,
            }],
            stop_reason: StopReason::ToolUse,
            usage: usage1(),
        }
    }

    struct Recorder {
        calls: Arc<Mutex<Vec<serde_json::Value>>>,
        reply: Result<String, String>,
    }

    #[async_trait::async_trait]
    impl Tool for Recorder {
        fn name(&self) -> &str {
            "recorder"
        }
        fn description(&self) -> &str {
            "records calls"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, input: serde_json::Value) -> Result<String, HarnessError> {
            self.calls.lock().unwrap().push(input);
            self.reply.clone().map_err(|m| HarnessError::Tool {
                name: "recorder".into(),
                message: m,
            })
        }
    }

    fn agent_with(
        responses: Vec<CompletionResponse>,
        tools: ToolRegistry,
    ) -> (Agent, Arc<MockProvider>) {
        let provider = Arc::new(MockProvider::new(responses));
        let agent = Agent::new(
            provider.clone(),
            tools,
            AgentConfig {
                system_prompt: "you are a field agent".into(),
                max_turns: 5,
                max_tokens: 1000,
            },
        );
        (agent, provider)
    }

    #[tokio::test]
    async fn text_only_response_ends_the_loop() {
        let (agent, provider) = agent_with(vec![text_end("done")], ToolRegistry::new());
        let out = agent.run(vec![Message::user_text("hi")]).await.unwrap();
        assert_eq!(out.text, "done");
        assert_eq!(out.usage, usage1());
        let reqs = provider.requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].system, "you are a field agent");
    }

    #[tokio::test]
    async fn tool_call_executes_and_result_feeds_back() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut reg = ToolRegistry::new();
        reg.register(Recorder { calls: calls.clone(), reply: Ok("saved".into()) });

        let (agent, provider) = agent_with(
            vec![
                tool_call("recorder", serde_json::json!({"x": 1})),
                text_end("all done"),
            ],
            reg,
        );
        let out = agent.run(vec![Message::user_text("go")]).await.unwrap();

        assert_eq!(out.text, "all done");
        assert_eq!(calls.lock().unwrap().as_slice(), &[serde_json::json!({"x": 1})]);
        // usage accumulated over two provider calls
        assert_eq!(out.usage, Usage { input_tokens: 20, output_tokens: 40 });

        // second request must carry assistant tool_use then user tool_result
        let reqs = provider.requests();
        assert_eq!(reqs.len(), 2);
        let second = &reqs[1];
        let assistant = &second.messages[second.messages.len() - 2];
        let user = &second.messages[second.messages.len() - 1];
        assert_eq!(assistant.role, Role::Assistant);
        assert!(matches!(&assistant.content[0], ContentBlock::ToolUse { name, .. } if name == "recorder"));
        assert_eq!(
            user.content[0],
            ContentBlock::ToolResult {
                tool_use_id: "tu_1".into(),
                content: "saved".into(),
                is_error: false,
            }
        );
    }

    #[tokio::test]
    async fn failing_tool_becomes_error_result_not_abort() {
        let mut reg = ToolRegistry::new();
        reg.register(Recorder {
            calls: Arc::new(Mutex::new(Vec::new())),
            reply: Err("disk full".into()),
        });
        let (agent, provider) = agent_with(
            vec![tool_call("recorder", serde_json::json!({})), text_end("recovered")],
            reg,
        );
        let out = agent.run(vec![Message::user_text("go")]).await.unwrap();
        assert_eq!(out.text, "recovered");
        let reqs = provider.requests();
        let user = reqs[1].messages.last().unwrap();
        assert!(matches!(
            &user.content[0],
            ContentBlock::ToolResult { is_error: true, content, .. } if content.contains("disk full")
        ));
    }

    #[tokio::test]
    async fn unknown_tool_becomes_error_result() {
        let (agent, provider) = agent_with(
            vec![tool_call("ghost", serde_json::json!({})), text_end("ok")],
            ToolRegistry::new(),
        );
        let out = agent.run(vec![Message::user_text("go")]).await.unwrap();
        assert_eq!(out.text, "ok");
        let reqs = provider.requests();
        let user = reqs[1].messages.last().unwrap();
        assert!(matches!(
            &user.content[0],
            ContentBlock::ToolResult { is_error: true, .. }
        ));
    }

    #[tokio::test]
    async fn max_turns_aborts() {
        let mut reg = ToolRegistry::new();
        reg.register(Recorder {
            calls: Arc::new(Mutex::new(Vec::new())),
            reply: Ok("again".into()),
        });
        // always answers with a tool call; max_turns = 5 → 5 responses then error
        let responses = (0..5)
            .map(|_| tool_call("recorder", serde_json::json!({})))
            .collect();
        let (agent, _provider) = agent_with(responses, reg);
        let err = agent.run(vec![Message::user_text("go")]).await.unwrap_err();
        assert!(matches!(err, HarnessError::MaxTurns(5)));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p harness agent`
Expected: compile FAIL (`Agent` not found).

- [ ] **Step 3: Implement**

`crates/harness/src/agent.rs` (above tests):
```rust
use std::sync::Arc;

use crate::error::HarnessError;
use crate::llm::{
    CompletionRequest, ContentBlock, LlmProvider, Message, Role, ToolSpec, Usage,
};
use crate::tool::ToolRegistry;

#[derive(Clone, Debug)]
pub struct AgentConfig {
    pub system_prompt: String,
    pub max_turns: usize,
    pub max_tokens: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TurnOutcome {
    /// Concatenated text of the final (tool-free) assistant response.
    pub text: String,
    /// Full transcript including tool_use/tool_result messages appended during the run.
    pub messages: Vec<Message>,
    /// Token usage accumulated across every provider call in this run.
    pub usage: Usage,
}

pub struct Agent {
    provider: Arc<dyn LlmProvider>,
    tools: ToolRegistry,
    config: AgentConfig,
}

impl Agent {
    pub fn new(provider: Arc<dyn LlmProvider>, tools: ToolRegistry, config: AgentConfig) -> Self {
        Agent { provider, tools, config }
    }

    fn tool_specs(&self) -> Vec<ToolSpec> {
        self.tools.specs()
    }

    pub async fn run(&self, mut messages: Vec<Message>) -> Result<TurnOutcome, HarnessError> {
        let mut usage = Usage::default();

        for _ in 0..self.config.max_turns {
            let response = self
                .provider
                .complete(CompletionRequest {
                    system: self.config.system_prompt.clone(),
                    messages: messages.clone(),
                    tools: self.tool_specs(),
                    max_tokens: self.config.max_tokens,
                })
                .await?;
            usage.add(&response.usage);

            let tool_uses: Vec<(String, String, serde_json::Value)> = response
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolUse { id, name, input } => {
                        Some((id.clone(), name.clone(), input.clone()))
                    }
                    _ => None,
                })
                .collect();

            if tool_uses.is_empty() {
                let text = response
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                messages.push(Message { role: Role::Assistant, content: response.content });
                return Ok(TurnOutcome { text, messages, usage });
            }

            messages.push(Message { role: Role::Assistant, content: response.content.clone() });

            let mut results = Vec::with_capacity(tool_uses.len());
            for (id, name, input) in tool_uses {
                let block = match self.tools.execute(&name, input).await {
                    Ok(content) => ContentBlock::ToolResult {
                        tool_use_id: id,
                        content,
                        is_error: false,
                    },
                    Err(e) => ContentBlock::ToolResult {
                        tool_use_id: id,
                        content: e.to_string(),
                        is_error: true,
                    },
                };
                results.push(block);
            }
            messages.push(Message { role: Role::User, content: results });
        }

        Err(HarnessError::MaxTurns(self.config.max_turns))
    }
}
```

`crates/harness/src/lib.rs` — add:
```rust
pub mod agent;
pub use agent::{Agent, AgentConfig, TurnOutcome};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harness`
Expected: all pass (13 total).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(harness): agent loop with tool dispatch, error-resilient results, usage accounting"
```

---

### Task 6: Anthropic provider (BYOK)

**Files:**
- Create: `crates/harness/src/providers/mod.rs`, `crates/harness/src/providers/anthropic.rs`
- Modify: `crates/harness/src/lib.rs`

Speaks the Messages API. Base URL injectable so wiremock can stand in. No streaming yet (Plan 04 revisits for live extraction). API key arrives from the app's keychain layer later — the provider just takes a string.

- [ ] **Step 1: Write the failing tests** (bottom of `crates/harness/src/providers/anthropic.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn request() -> CompletionRequest {
        CompletionRequest {
            system: "sys".into(),
            messages: vec![Message::user_text("hello")],
            tools: vec![ToolSpec {
                name: "echo".into(),
                description: "d".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }],
            max_tokens: 256,
        }
    }

    #[tokio::test]
    async fn sends_correct_request_and_parses_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "sk-test"))
            .and(header("anthropic-version", "2023-06-01"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [
                    {"type": "text", "text": "hi there"},
                    {"type": "tool_use", "id": "tu_9", "name": "echo", "input": {"text": "x"}}
                ],
                "stop_reason": "tool_use",
                "usage": {"input_tokens": 42, "output_tokens": 7}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let provider = AnthropicProvider::new("sk-test", "claude-haiku-4-5-20251001")
            .with_base_url(server.uri());
        let resp = provider.complete(request()).await.unwrap();

        assert_eq!(resp.stop_reason, StopReason::ToolUse);
        assert_eq!(resp.usage, Usage { input_tokens: 42, output_tokens: 7 });
        assert_eq!(resp.content.len(), 2);
        assert!(matches!(&resp.content[1], ContentBlock::ToolUse { name, .. } if name == "echo"));

        // verify body shape
        let received = &server.received_requests().await.unwrap()[0];
        let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
        assert_eq!(body["model"], "claude-haiku-4-5-20251001");
        assert_eq!(body["system"], "sys");
        assert_eq!(body["max_tokens"], 256);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["tools"][0]["name"], "echo");
    }

    #[tokio::test]
    async fn api_error_maps_to_provider_error_with_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "type": "error",
                "error": {"type": "authentication_error", "message": "invalid x-api-key"}
            })))
            .mount(&server)
            .await;

        let provider =
            AnthropicProvider::new("bad-key", "claude-haiku-4-5-20251001").with_base_url(server.uri());
        let err = provider.complete(request()).await.unwrap_err();
        match err {
            crate::HarnessError::Provider(msg) => {
                assert!(msg.contains("401"));
                assert!(msg.contains("invalid x-api-key"));
            }
            other => panic!("wrong error: {other:?}"),
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p harness anthropic`
Expected: compile FAIL (`AnthropicProvider` not found).

- [ ] **Step 3: Implement**

`crates/harness/src/providers/mod.rs`:
```rust
pub mod anthropic;
pub use anthropic::AnthropicProvider;
```

`crates/harness/src/providers/anthropic.rs` (above tests):
```rust
use serde::Deserialize;

use crate::error::HarnessError;
use crate::llm::{
    CompletionRequest, CompletionResponse, ContentBlock, LlmProvider, StopReason, Usage,
};

pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        AnthropicProvider {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://api.anthropic.com".into(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

#[derive(Deserialize)]
struct ApiResponse {
    content: Vec<ContentBlock>,
    stop_reason: StopReason,
    usage: Usage,
}

#[async_trait::async_trait]
impl LlmProvider for AnthropicProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, HarnessError> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": req.max_tokens,
            "system": req.system,
            "messages": req.messages,
            "tools": req.tools,
        });

        let resp = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(|e| HarnessError::Provider(e.to_string()))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| HarnessError::Provider(e.to_string()))?;

        if !status.is_success() {
            return Err(HarnessError::Provider(format!("HTTP {status}: {text}")));
        }

        let parsed: ApiResponse = serde_json::from_str(&text)
            .map_err(|e| HarnessError::Provider(format!("bad response body: {e}: {text}")))?;

        Ok(CompletionResponse {
            content: parsed.content,
            stop_reason: parsed.stop_reason,
            usage: parsed.usage,
        })
    }
}
```

`crates/harness/src/lib.rs` — add:
```rust
pub mod providers;
pub use providers::AnthropicProvider;
```

Note: when `tools` is empty, sending `"tools": []` is accepted by the API; no need to omit the field.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harness`
Expected: all pass (15 total).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(harness): Anthropic Messages API provider (BYOK)"
```

---

### Task 7: End-to-end harness test + docs

**Files:**
- Create: `crates/harness/tests/agent_anthropic_e2e.rs`
- Modify: `README.md`

One integration test wiring `Agent` + `AnthropicProvider` + wiremock across a full tool round-trip — proves the pieces compose across crate boundaries the way downstream plans will use them.

- [ ] **Step 1: Write the test**

`crates/harness/tests/agent_anthropic_e2e.rs`:
```rust
use std::sync::Arc;

use harness::{
    Agent, AgentConfig, AnthropicProvider, ContentBlock, HarnessError, Message, Tool,
    ToolRegistry,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Respond, ResponseTemplate};

struct SaveItem;

#[async_trait::async_trait]
impl Tool for SaveItem {
    fn name(&self) -> &str {
        "save_item"
    }
    fn description(&self) -> &str {
        "saves a captured item"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {"title": {"type": "string"}},
            "required": ["title"]
        })
    }
    async fn execute(&self, input: serde_json::Value) -> Result<String, HarnessError> {
        Ok(format!("saved: {}", input["title"].as_str().unwrap_or("?")))
    }
}

struct Script;

impl Respond for Script {
    fn respond(&self, req: &wiremock::Request) -> ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        let has_tool_result = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
            .any(|b| b["type"] == "tool_result");

        if has_tool_result {
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [{"type": "text", "text": "logged the mulch"}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 30, "output_tokens": 10}
            }))
        } else {
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [{"type": "tool_use", "id": "tu_1", "name": "save_item",
                             "input": {"title": "bark mulch — front beds"}}],
                "stop_reason": "tool_use",
                "usage": {"input_tokens": 20, "output_tokens": 15}
            }))
        }
    }
}

#[tokio::test]
async fn full_round_trip_through_real_provider_wire_format() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(Script)
        .expect(2)
        .mount(&server)
        .await;

    let provider =
        AnthropicProvider::new("sk-test", "claude-haiku-4-5-20251001").with_base_url(server.uri());
    let mut tools = ToolRegistry::new();
    tools.register(SaveItem);

    let agent = Agent::new(
        Arc::new(provider),
        tools,
        AgentConfig {
            system_prompt: "extract items from field transcripts".into(),
            max_turns: 4,
            max_tokens: 512,
        },
    );

    let out = agent
        .run(vec![Message::user_text("front beds need mulch, call it three yards")])
        .await
        .unwrap();

    assert_eq!(out.text, "logged the mulch");
    assert_eq!(out.usage.input_tokens, 50);
    assert_eq!(out.usage.output_tokens, 25);
    assert!(out
        .messages
        .iter()
        .any(|m| m.content.iter().any(|b| matches!(b, ContentBlock::ToolResult { content, .. } if content == "saved: bark mulch — front beds"))));
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p harness --test agent_anthropic_e2e`
Expected: PASS (if it fails, fix the harness — this test asserts the composition contract, don't weaken it).

- [ ] **Step 3: Update README**

Append to `README.md`:
```markdown

## Testing

`cargo test` — all tests are hermetic (MockProvider or wiremock); no network, no API keys.

## Plan series

Implementation plans 01–06 live in the Murmur meta repo at `docs/superpowers/plans/2026-07-01-rust-core-*.md`.
Next: 02 memory + reflection + context assembler.
```

- [ ] **Step 4: Full suite + commit**

Run: `cargo test`
Expected: all pass.

```bash
git add -A && git commit -m "test(harness): e2e agent+provider round-trip over wire format; README"
```

---

## Self-Review Notes

- **Spec coverage (this plan's slice):** agent loop ✓ (Task 5), provider-agnostic client ✓ (Tasks 2, 6), tool registry ✓ (Task 4), usage/cost accounting from day one (R9 groundwork) ✓ (Tasks 5, 7), tool-failure resilience (R7) ✓ (Task 5 tests). Memory/reflection, context assembler, layout protocol, streaming: deliberately Plans 02/04/06.
- **Type consistency:** `ToolRegistry::execute(name, input) -> Result<String, HarnessError>` used identically in Tasks 4 and 5; `Usage::add` defined Task 2, used Task 5; `with_base_url` defined Task 6, used Tasks 6–7.
- **No placeholders:** every step has full code or exact commands. Task 1 Step 2 includes an inline correction (lib.rs starts empty) — the corrected line is the one to use.
