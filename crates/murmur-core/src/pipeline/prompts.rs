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
