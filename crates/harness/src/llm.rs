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
    /// Any block type this crate doesn't know yet (e.g. thinking, server_tool_use).
    /// Lenient on purpose. Unknown blocks are dropped before messages are re-sent
    /// to a provider (see Agent::run) — we can't faithfully round-trip them.
    #[serde(other)]
    Unknown,
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

/// No serde derives on purpose: each provider maps this to its own wire format.
#[derive(Clone, Debug, PartialEq)]
pub struct CompletionRequest {
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: u32,
    /// Force the model to call this tool by name (None = model decides).
    pub tool_choice: Option<String>,
    /// Ask the provider to cache this request's stable prefix.
    ///
    /// Opt-in per request, NOT a global provider setting, because caching only
    /// pays when the same prefix is sent more than once. A cache write bills at
    /// ~1.25x and a read at ~0.1x, so break-even is two requests: a single-shot
    /// call (`summarize`, the forced `build_document` call, reflection) would
    /// pay the write premium and never read it back. Only `Agent::run` sets
    /// this, because only it re-sends a growing conversation with a
    /// byte-identical `system` + `tools` prefix on every turn.
    pub cache_prefix: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    /// Any stop reason this crate doesn't know yet (e.g. refusal, pause_turn).
    /// Lenient on purpose: an unknown stop reason must never fail response parsing.
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// The prompt tokens billed at the FULL rate. When prompt caching is in
    /// play this is the *uncached remainder only* — NOT the whole prompt.
    /// `total_input_tokens()` is what you want for "how big was the prompt".
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Prompt tokens written to the cache this call (billed ~1.25x). Absent on
    /// providers/responses that don't cache, hence `default` — a missing field
    /// must never fail response parsing (same posture as `StopReason::Unknown`).
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    /// Prompt tokens served FROM the cache this call (billed ~0.1x).
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, other: &Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_creation_input_tokens += other.cache_creation_input_tokens;
        self.cache_read_input_tokens += other.cache_read_input_tokens;
    }

    /// Every prompt token the call actually processed, cached or not.
    ///
    /// Cost analysis must use this rather than `input_tokens`: once caching is
    /// on, the API moves most of the prompt into the two cache fields and
    /// `input_tokens` collapses to the remainder. Reading `input_tokens` alone
    /// would show a dramatic "saving" that is purely a reporting artifact.
    pub fn total_input_tokens(&self) -> u64 {
        self.input_tokens + self.cache_creation_input_tokens + self.cache_read_input_tokens
    }
}

/// No serde derives on purpose: providers parse their own wire format into this.
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
        assert_eq!(v["id"], "tu_1");
        assert_eq!(v["input"]["title"], "mulch beds");
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
        let mut u = Usage { input_tokens: 10, output_tokens: 5, ..Default::default() };
        u.add(&Usage { input_tokens: 3, output_tokens: 7, ..Default::default() });
        assert_eq!(u, Usage { input_tokens: 13, output_tokens: 12, ..Default::default() });
    }

    #[test]
    fn usage_adds_accumulates_cache_tokens_too() {
        // A dropped cache field here would silently under-report spend across a
        // multi-turn run — the exact failure R9 exists to prevent.
        let mut u = Usage {
            input_tokens: 10,
            output_tokens: 5,
            cache_creation_input_tokens: 100,
            cache_read_input_tokens: 200,
        };
        u.add(&Usage {
            input_tokens: 1,
            output_tokens: 2,
            cache_creation_input_tokens: 3,
            cache_read_input_tokens: 4,
        });
        assert_eq!(
            u,
            Usage {
                input_tokens: 11,
                output_tokens: 7,
                cache_creation_input_tokens: 103,
                cache_read_input_tokens: 204,
            }
        );
    }

    #[test]
    fn total_input_tokens_spans_all_three_input_classes() {
        let u = Usage {
            input_tokens: 10,
            output_tokens: 999,
            cache_creation_input_tokens: 100,
            cache_read_input_tokens: 1_000,
        };
        // Not 10: reading `input_tokens` alone is the reporting artifact that
        // makes caching look like a 99% saving.
        assert_eq!(u.total_input_tokens(), 1_110);
    }

    #[test]
    fn usage_without_cache_fields_parses_leniently() {
        // The mock provider and any non-caching response omit these entirely.
        let u: Usage =
            serde_json::from_value(serde_json::json!({"input_tokens": 42, "output_tokens": 7}))
                .unwrap();
        assert_eq!(u.cache_creation_input_tokens, 0);
        assert_eq!(u.cache_read_input_tokens, 0);
        assert_eq!(u.total_input_tokens(), 42);
    }

    #[test]
    fn unknown_content_block_type_parses_leniently() {
        let v = serde_json::json!({"type": "thinking", "thinking": "hmm", "signature": "sig"});
        let back: ContentBlock = serde_json::from_value(v).unwrap();
        assert_eq!(back, ContentBlock::Unknown);
    }

    #[test]
    fn unknown_stop_reason_parses_leniently() {
        let r: StopReason = serde_json::from_value(serde_json::json!("refusal")).unwrap();
        assert_eq!(r, StopReason::Unknown);
    }
}
