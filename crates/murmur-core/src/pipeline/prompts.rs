//! Prompts for the processing pipeline. Product rules live here:
//! R6 (under-extraction bias) and R7 (real outcomes) are prompt-enforced;
//! the tools themselves stay mechanical.

use std::sync::Arc;

use harness::{
    CompletionRequest, ContentBlock, HarnessError, LlmProvider, Message, ToolSpec, Usage,
};

use crate::domain::CapturedItem;

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
                "summary": { "type": "string", "description": "1-2 plain sentences: what happened, key outcomes" },
                "spoken_total_cents": {
                    "type": "integer",
                    "description": "The operator's stated target/grand total for the WHOLE job, in cents \
                                     — ONLY when a specific dollar total was clearly spoken (e.g. \"keep it \
                                     under twelve hundred\" -> 120000). Omit entirely if no total was stated \
                                     or you are unsure — never guess."
                }
            },
            "required": ["summary"]
        }),
    }
}

/// One-shot forced summary call (the Plan 02 reflection-engine pattern).
///
/// Provider errors stay `Err` (no tokens were incurred). A successful call
/// that lacks a `write_summary` block returns `Ok((None, None, usage))` so
/// the caller can log the spend (R9) before deciding it's a failure. The
/// transcript excerpt is passed through as-is — it already carries its own
/// `## transcript` header from the context assembler.
///
/// D5a: the optional `spoken_total_cents` is captured HERE — the only pass
/// that legitimately reads the transcript — and threaded as a scalar hint
/// into the on-demand pricing pass (`DocumentBuilder::build`) later, so the
/// pricing prompt itself never needs transcript access.
pub(crate) async fn summarize(
    provider: Arc<dyn LlmProvider>,
    transcript_excerpt: &str,
    max_tokens: u32,
) -> Result<(Option<String>, Option<i64>, Usage), HarnessError> {
    let response = provider
        .complete(CompletionRequest {
            system: "Summarize one transcribed field-work session in 1-2 plain sentences \
                     for a session list. Lead with what happened; include key outcomes."
                .into(),
            messages: vec![Message::user_text(transcript_excerpt)],
            tools: vec![summary_tool_spec()],
            max_tokens,
            tool_choice: Some(WRITE_SUMMARY.into()),
        })
        .await?;

    let tool_input = response.content.iter().find_map(|b| match b {
        ContentBlock::ToolUse { name, input, .. } if name == WRITE_SUMMARY => Some(input),
        _ => None,
    });
    let summary =
        tool_input.and_then(|i| i.get("summary").and_then(|s| s.as_str()).map(str::to_string));
    let spoken_total_cents =
        tool_input.and_then(|i| i.get("spoken_total_cents").and_then(|s| s.as_i64()));
    Ok((summary, spoken_total_cents, response.usage))
}

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
        let (summary, spoken_total_cents, usage) =
            summarize(provider.clone(), "transcript text", 512).await.unwrap();
        assert_eq!(summary.as_deref(), Some("Walked the deck; two todos."));
        assert_eq!(spoken_total_cents, None, "no total was stated");
        assert_eq!(usage, Usage { input_tokens: 40, output_tokens: 12 });
        let reqs = provider.requests();
        assert_eq!(reqs[0].tool_choice.as_deref(), Some("write_summary"));
        assert!(reqs[0].max_tokens >= 1);
        // the excerpt is the user message verbatim — no extra prefix
        assert_eq!(
            reqs[0].messages[0].content,
            vec![ContentBlock::Text { text: "transcript text".into() }]
        );
    }

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

    #[tokio::test]
    async fn summarize_without_tool_call_returns_no_summary_but_usage() {
        let provider = Arc::new(MockProvider::new(vec![CompletionResponse {
            content: vec![ContentBlock::Text { text: "no tool".into() }],
            stop_reason: StopReason::EndTurn,
            usage: Usage { input_tokens: 50, output_tokens: 10 },
        }]));
        let (summary, spoken_total_cents, usage) = summarize(provider, "t", 512).await.unwrap();
        assert!(summary.is_none(), "missing tool call is not an Err — spend must be loggable");
        assert_eq!(spoken_total_cents, None);
        assert_eq!(usage, Usage { input_tokens: 50, output_tokens: 10 });
    }

    #[tokio::test]
    async fn summarize_captures_the_spoken_total_when_stated() {
        let provider = Arc::new(MockProvider::new(vec![CompletionResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "write_summary".into(),
                input: serde_json::json!({
                    "summary": "Mulch and railing; keep it under twelve hundred.",
                    "spoken_total_cents": 120000
                }),
            }],
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 40, output_tokens: 12 },
        }]));
        let (summary, spoken_total_cents, _usage) =
            summarize(provider, "transcript text", 512).await.unwrap();
        assert!(summary.is_some());
        assert_eq!(spoken_total_cents, Some(120000));
    }
}
