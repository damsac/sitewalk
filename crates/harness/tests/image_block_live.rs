//! Env-gated live check that an `Image` content block survives the wire.
//!
//! `AnthropicProvider::complete` builds its body with `"messages": req.messages`,
//! so serde IS the wire format — there is no per-variant provider mapping to get
//! wrong. That also means a variant nothing sends is a variant nothing has ever
//! proven. This test sends one, against the real API, and asserts the model
//! actually saw the pixels.
//!
//! Ignored by default — it costs real tokens and needs a key. Run explicitly:
//!
//! ```sh
//! ANTHROPIC_API_KEY=sk-... nix develop -c \
//!     cargo test -p harness --test image_block_live -- --ignored --nocapture
//! ```

use harness::{
    AnthropicProvider, CompletionRequest, ContentBlock, ImageMediaType, ImageSource, LlmProvider,
    Message, Role,
};

/// Cheapest current haiku-class model — this is a wire check, not an eval.
const MODEL: &str = "claude-haiku-4-5";

/// A 64x64 solid red PNG, 136 bytes. Inlined rather than shipped as a binary
/// fixture so the test is readable and the repo stays text-only.
const RED_SQUARE_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAEAAAABACAIAAAAlC+aJAAAAT0lEQVR42u3PQQkAAAgEsEty/UMZxgi+hcEKLNO+FgEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQGBywLPLIEA68ZURwAAAABJRU5ErkJggg==";

#[tokio::test]
#[ignore = "hits the real Anthropic API; set ANTHROPIC_API_KEY and run with --ignored"]
async fn real_anthropic_accepts_an_image_block() {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .expect("set ANTHROPIC_API_KEY to run the real-provider image smoke test");

    let provider = AnthropicProvider::from_env(api_key, MODEL);

    let resp = provider
        .complete(CompletionRequest {
            system: "You answer in a single word.".into(),
            messages: vec![Message {
                role: Role::User,
                content: vec![
                    ContentBlock::Image {
                        source: ImageSource::Base64 {
                            media_type: ImageMediaType::Png,
                            data: RED_SQUARE_PNG_BASE64.into(),
                        },
                    },
                    ContentBlock::Text {
                        text: "What is the dominant color of this image? \
                               Reply with exactly one word."
                            .into(),
                    },
                ],
            }],
            tools: vec![],
            max_tokens: 16,
            tool_choice: None,
            cache_prefix: false,
        })
        .await
        .expect("live image request failed");

    let text = resp
        .content
        .iter()
        .find_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .expect("no text block in response");

    eprintln!(
        "live: model said {text:?}; stop_reason={:?}; usage={:?}",
        resp.stop_reason, resp.usage
    );

    // Not just "it was a 200": a 200 proves the block parsed, this proves the
    // pixels reached the model. A block that serialized to the wrong shape
    // would 400; one the model couldn't see would answer something else.
    assert!(
        text.to_lowercase().contains("red"),
        "model did not see the red image; it said: {text:?}"
    );
    assert!(resp.usage.input_tokens > 0, "no input tokens billed");
    assert!(resp.usage.output_tokens > 0, "no output tokens billed");
}
