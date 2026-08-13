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
    /// An image the model can see. Send-only: the API never returns one.
    ///
    /// Image and Document are two variants rather than one merged `Media`
    /// variant because the `#[serde(tag = "type")]` above already maps the
    /// variant name to the wire `type` string for free. A merged variant would
    /// have to choose between `"image"` and `"document"` at runtime, which
    /// means a hand-written Serialize impl reimplementing the discriminator the
    /// enum already provides. The block shapes differ anyway — document blocks
    /// carry optional `title`/`context`/`citations` that image blocks do not.
    Image {
        source: ImageSource,
    },
    /// A document (today: a PDF) the model can read. Send-only.
    Document {
        source: DocumentSource,
        /// Passed to the model but never cited from; length-limited by the API.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// Metadata about the document. Passed to the model, never cited from —
        /// this is where longer provenance belongs, since `title` is short.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        citations: Option<CitationsConfig>,
    },
    /// Any block type this crate doesn't know yet (e.g. thinking, server_tool_use).
    /// Lenient on purpose. Unknown blocks are dropped before messages are re-sent
    /// to a provider (see Agent::run) — we can't faithfully round-trip them.
    #[serde(other)]
    Unknown,
}

/// Where the bytes of an image block come from.
///
/// A tagged enum rather than a struct even though base64 is the only source we
/// need today: the API also accepts `{"type":"url", ...}` and
/// `{"type":"file", "file_id": ...}`. One level of nesting now makes adding
/// those an additive change here instead of a breaking change at every call
/// site — the same reason `ContentBlock` itself is an enum.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageSource {
    Base64 {
        media_type: ImageMediaType,
        /// Standard base64, no line breaks.
        data: String,
    },
}

/// The image formats the API accepts. Animations are not supported — only the
/// first frame of a GIF or animated WebP is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageMediaType {
    #[serde(rename = "image/jpeg")]
    Jpeg,
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/gif")]
    Gif,
    #[serde(rename = "image/webp")]
    Webp,
}

/// Where the bytes of a document block come from. Tagged for the same reason as
/// [`ImageSource`] — `url`, `file`, `text`, and `content` sources exist on the
/// wire and can be added without touching callers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DocumentSource {
    Base64 {
        media_type: DocumentMediaType,
        /// Standard base64, no line breaks.
        data: String,
    },
}

/// The only media type a base64 document source accepts. An enum with one
/// variant on purpose: it puts "PDF is the only option" in the type system
/// instead of a doc comment, and a second format stays additive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocumentMediaType {
    #[serde(rename = "application/pdf")]
    Pdf,
}

/// Opt in to citations for one document block. All-or-nothing per request:
/// the API rejects a mix of cited and uncited documents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitationsConfig {
    pub enabled: bool,
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
    fn image_block_serializes_to_anthropic_wire_shape() {
        let block = ContentBlock::Image {
            source: ImageSource::Base64 {
                media_type: ImageMediaType::Png,
                data: "aGVsbG8=".into(),
            },
        };
        let v = serde_json::to_value(&block).unwrap();
        // The serde tag IS the wire type — no per-provider mapping exists to
        // catch a rename, so pin the exact strings the API accepts.
        assert_eq!(v["type"], "image");
        assert_eq!(v["source"]["type"], "base64");
        assert_eq!(v["source"]["media_type"], "image/png");
        assert_eq!(v["source"]["data"], "aGVsbG8=");
        let back: ContentBlock = serde_json::from_value(v).unwrap();
        assert_eq!(back, block);
    }

    #[test]
    fn every_image_media_type_serializes_to_its_mime_string() {
        // A wrong variant rename here is a 400 at the API, not a compile error.
        for (variant, mime) in [
            (ImageMediaType::Jpeg, "image/jpeg"),
            (ImageMediaType::Png, "image/png"),
            (ImageMediaType::Gif, "image/gif"),
            (ImageMediaType::Webp, "image/webp"),
        ] {
            assert_eq!(serde_json::to_value(variant).unwrap(), mime);
            let back: ImageMediaType = serde_json::from_value(mime.into()).unwrap();
            assert_eq!(back, variant);
        }
    }

    #[test]
    fn document_block_serializes_to_anthropic_wire_shape() {
        let block = ContentBlock::Document {
            source: DocumentSource::Base64 {
                media_type: DocumentMediaType::Pdf,
                data: "JVBERi0=".into(),
            },
            title: Some("Invoice template".into()),
            context: Some("Operator-uploaded".into()),
            citations: Some(CitationsConfig { enabled: true }),
        };
        let v = serde_json::to_value(&block).unwrap();
        assert_eq!(v["type"], "document");
        assert_eq!(v["source"]["type"], "base64");
        assert_eq!(v["source"]["media_type"], "application/pdf");
        assert_eq!(v["source"]["data"], "JVBERi0=");
        assert_eq!(v["title"], "Invoice template");
        assert_eq!(v["context"], "Operator-uploaded");
        assert_eq!(v["citations"]["enabled"], true);
        let back: ContentBlock = serde_json::from_value(v).unwrap();
        assert_eq!(back, block);
    }

    #[test]
    fn document_optional_fields_are_absent_not_null_when_unset() {
        // `"title": null` is not the same as an omitted key to the API — a
        // null would be rejected where an absent field is fine.
        let block = ContentBlock::Document {
            source: DocumentSource::Base64 {
                media_type: DocumentMediaType::Pdf,
                data: "JVBERi0=".into(),
            },
            title: None,
            context: None,
            citations: None,
        };
        let v = serde_json::to_value(&block).unwrap();
        assert!(v.get("title").is_none(), "title must be absent: {v}");
        assert!(v.get("context").is_none(), "context must be absent: {v}");
        assert!(v.get("citations").is_none(), "citations must be absent: {v}");
        let back: ContentBlock = serde_json::from_value(v).unwrap();
        assert_eq!(back, block);
    }

    #[test]
    fn a_whole_message_of_media_blocks_serializes_as_the_provider_sends_it() {
        // AnthropicProvider builds its body with `"messages": req.messages`, so
        // serde IS the wire format — this is the shape that actually goes out.
        let msg = Message {
            role: Role::User,
            content: vec![
                ContentBlock::Image {
                    source: ImageSource::Base64 {
                        media_type: ImageMediaType::Png,
                        data: "aGVsbG8=".into(),
                    },
                },
                ContentBlock::Text { text: "what is this?".into() },
            ],
        };
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["role"], "user");
        assert_eq!(v["content"][0]["type"], "image");
        assert_eq!(v["content"][1]["type"], "text");
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
