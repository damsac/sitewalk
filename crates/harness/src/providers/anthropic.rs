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
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(5))
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .expect("reqwest client with static config cannot fail"),
            api_key: api_key.into(),
            model: model.into(),
            base_url: "https://api.anthropic.com".into(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        // Normalize: a trailing slash would produce `{base}//v1/messages`.
        // Proxy/PPQ base URLs are routinely pasted with one.
        let base_url = base_url.into();
        self.base_url = base_url.trim_end_matches('/').to_string();
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
        if req.cache_prefix {
            // Top-level cache_control auto-places the breakpoint on the last
            // cacheable block, which is exactly the multi-turn pattern: each
            // turn extends the cached prefix, and the previous turn's entry is
            // still a valid read point. Placing it by hand would mean tracking
            // block indices across turns for no gain.
            //
            // Silently a no-op below the model's minimum cacheable prefix
            // (4096 tokens on Haiku 4.5) — no error, and no write premium
            // either, since nothing is cached. That is what makes it safe to
            // set unconditionally on the agent path, where live extraction's
            // small prefix sits under the bar and processing's does not.
            body["cache_control"] = serde_json::json!({"type": "ephemeral"});
        }

        let resp = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("authorization", format!("Bearer {}", self.api_key))
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
            tool_choice: None,
            cache_prefix: false,
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
        assert_eq!(resp.usage, Usage { input_tokens: 42, output_tokens: 7, ..Default::default() });
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
    async fn cache_prefix_emits_top_level_cache_control() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [{"type": "text", "text": "ok"}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 1, "output_tokens": 1}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let provider =
            AnthropicProvider::new("sk-test", "claude-haiku-4-5").with_base_url(server.uri());
        provider
            .complete(CompletionRequest { cache_prefix: true, ..request() })
            .await
            .unwrap();

        let received = &server.received_requests().await.unwrap()[0];
        let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
        assert_eq!(body["cache_control"]["type"], "ephemeral");
    }

    #[tokio::test]
    async fn no_cache_control_when_cache_prefix_is_off() {
        // The single-shot paths (summarize, build_document, reflection) must not
        // pay the ~1.25x cache-write premium for a prefix nothing will re-read.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [{"type": "text", "text": "ok"}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 1, "output_tokens": 1}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let provider =
            AnthropicProvider::new("sk-test", "claude-haiku-4-5").with_base_url(server.uri());
        provider.complete(request()).await.unwrap(); // cache_prefix: false

        let received = &server.received_requests().await.unwrap()[0];
        let body: serde_json::Value = serde_json::from_slice(&received.body).unwrap();
        assert!(
            body.get("cache_control").is_none(),
            "cache_control must be absent, not null: {body}"
        );
    }

    #[tokio::test]
    async fn parses_prompt_cache_token_fields_from_the_wire() {
        // A cached call reports FOUR token fields, and `input_tokens` shrinks to
        // the uncached remainder. Dropping the two cache fields on the floor
        // would make this response look like a 20-token prompt instead of 4020.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [{"type": "text", "text": "ok"}],
                "stop_reason": "end_turn",
                "usage": {
                    "input_tokens": 20,
                    "output_tokens": 7,
                    "cache_creation_input_tokens": 4000,
                    "cache_read_input_tokens": 0
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let provider =
            AnthropicProvider::new("sk-test", "claude-haiku-4-5-20251001").with_base_url(server.uri());
        let resp = provider.complete(request()).await.unwrap();

        assert_eq!(resp.usage.input_tokens, 20);
        assert_eq!(resp.usage.cache_creation_input_tokens, 4000);
        assert_eq!(resp.usage.cache_read_input_tokens, 0);
        assert_eq!(resp.usage.total_input_tokens(), 4020);
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

    #[tokio::test]
    async fn sends_bearer_and_x_api_key_for_ppq_compat() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("authorization", "Bearer sk-test"))
            .and(header("x-api-key", "sk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [{"type": "text", "text": "ok"}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 1, "output_tokens": 1}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let provider = AnthropicProvider::new("sk-test", "claude-haiku-4-5")
            .with_base_url(server.uri());
        provider.complete(request()).await.unwrap();
    }

    #[tokio::test]
    async fn base_url_with_trailing_slash_is_normalized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [{"type": "text", "text": "ok"}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 1, "output_tokens": 1}
            })))
            .expect(1)
            .mount(&server)
            .await;
        // A base_url carrying a trailing slash must not produce `//v1/messages`
        // (PPQ/proxy configs are frequently pasted with one).
        let provider = AnthropicProvider::new("sk-test", "claude-haiku-4-5")
            .with_base_url(format!("{}/", server.uri()));
        provider.complete(request()).await.unwrap();
    }

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
}
