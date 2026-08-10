//! Prompts for the processing pipeline. Product rules live here:
//! R6 (under-extraction bias) and R7 (real outcomes) are prompt-enforced;
//! the tools themselves stay mechanical.

use std::sync::Arc;

use harness::{
    CompletionRequest, ContentBlock, HarnessError, LlmProvider, Message, ToolSpec, Usage,
};

use crate::domain::CapturedItem;
use crate::pipeline::notes::{parse_notes_value, NotesEntry};

const WRITE_NOTES: &str = "write_notes";

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
         - An item is ONE PIECE OF WORK OR ONE MATERIAL, named the way it would \
         appear as a line on paperwork: a short noun phrase. \"Mulch, three beds\", \
         \"Weed and compost, three beds\", \"Prune five pear trees\". NOT a sentence \
         about the job: not \"Juan is going to do the mulching\", not \"we should \
         probably weed those beds first\".\n\
         - Group what the operator groups. If they name several things together and \
         give them ONE price — \"ten peppers, ten tomatoes and five artichokes… two \
         fifty for the plants\" — that is ONE item (\"Plants: 10 peppers, 10 \
         tomatoes, 5 artichokes\"), not three. Splitting it puts their one price on \
         one line and leaves the others blank, which is worse paperwork than the way \
         they said it. Match the grain of their pricing.\n\
         - When the operator states a PRICE, record it as its own item with the \
         figure kept exactly as spoken: \"$200 weeding and compost\", \"$500 labor\", \
         \"$300 flagstone\". This is the one place the noun-phrase rule does not \
         shorten anything — the amount is the whole content of that item, and a later \
         step reads the figure out and attaches it to the work it names. An item that \
         drops the number loses the price entirely; nothing downstream can recover it.\n\
         - But a WORK or MATERIAL item never carries a price in its own title. \
         \"Poison oak removal\" — not \"Poison oak removal, $200 regular + $100 hazard \
         pay\". The money is a separate price item; the work item is just the work, or \
         the amount ends up printed twice on the paperwork, once in the description \
         and once in the amount column.\n\
         - A material the operator names IS an item, even when they mention it only \
         while pricing it. \"Three hundred for the flagstone, two hundred for the \
         gravel\" names two materials: record \"Flagstone\" and \"Gravel\" as items \
         alongside the two price items, or their prices have no line to land on and \
         attach themselves to whatever else happens to mention that word.\n\
         - WHO does the work is never an item. A person assigned to a task belongs \
         in the notes (scope_of_work), not on a line of its own — a line reading \
         \"Juan to do the mulching\" duplicates the work item it refers to and lands \
         on a client's estimate. Same for WHEN it happens.\n\
         - Site conditions and access details are never items either — gate codes, \
         parking, dogs, working hours, hazards. They belong in the notes. An item is \
         something the operator does or buys.\n\
         - Use upsert_contact for people mentioned with a role (sub, client, supplier).\n\
         - Call write_report at most once, and only if the session has enough \
         substance for a report worth sharing.\n\
         - Use update_memory only for durable facts about the user, their people, \
         projects, or vocabulary — never for session content.\n\
         - Transcripts are speech-to-text: expect misrecognized jargon and names; \
         prefer terms from what you know about the user.{memory_block}"
    )
}

fn notes_tool_spec() -> ToolSpec {
    ToolSpec {
        name: WRITE_NOTES.into(),
        description: "Record the session's narrative summary, spoken total (if any), and \
                       comprehensive coordination notes."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "summary": { "type": "string", "description": "2-4 plain sentences: what, why, when" },
                "spoken_total_cents": {
                    "type": "integer",
                    "description": "The operator's stated target/grand total for the WHOLE job, in cents \
                                     — ONLY when a specific dollar total was clearly spoken (e.g. \"keep it \
                                     under twelve hundred\" -> 120000). Omit entirely if no total was stated \
                                     or you are unsure — never guess."
                },
                "notes": {
                    "type": "array",
                    "description": "At most 12 entries; prefer fewer, denser entries. Each entry is a \
                                     client/team coordination detail the terse board doesn't carry — the \
                                     full spoken context behind a decision, a constraint, or a site \
                                     condition. Capture only what was said; never invent a budget, \
                                     deadline, or access detail — a missed note is cheaper than a \
                                     fabricated constraint.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "bucket": {
                                "type": "string",
                                "enum": ["scope_of_work", "constraints", "conditions_and_issues"]
                            },
                            "label": { "type": "string", "description": "terse, mirrors a board label" },
                            "detail": { "type": "string", "description": "the full spoken context" }
                        },
                        "required": ["bucket", "label", "detail"]
                    }
                }
            },
            "required": ["summary"]
        }),
    }
}

/// One-shot forced notes call (the Plan 02 reflection-engine pattern; Plan
/// 14 D1: the same pass that already returned `spoken_total_cents` now also
/// returns the narrative summary's richer detail as `notes[]`).
///
/// Provider errors stay `Err` (no tokens were incurred). A successful call
/// that lacks a `write_notes` block returns `Ok((None, None, [], usage))` so
/// the caller can log the spend (R9) before deciding it's a failure. The
/// transcript excerpt is passed through as-is — it already carries its own
/// `## transcript` header from the context assembler.
///
/// D5a: the optional `spoken_total_cents` is captured HERE — the only pass
/// that legitimately reads the transcript — and threaded as a scalar hint
/// into the on-demand pricing pass (`DocumentBuilder::build`) later, so the
/// pricing prompt itself never needs transcript access.
///
/// C2 (R7): a `notes` value that's truncated (model hit `max_tokens` mid-array)
/// or malformed (non-array, garbled entries) degrades to `buckets: []` — the
/// parseable `summary` is still returned. `parse_notes_value` is the same
/// tolerant walk the stored artifact uses, so a garbled tool response and a
/// garbled stored artifact degrade identically.
pub(crate) async fn summarize(
    provider: Arc<dyn LlmProvider>,
    transcript_excerpt: &str,
    max_tokens: u32,
) -> Result<(Option<String>, Option<i64>, Vec<NotesEntry>, Usage), HarnessError> {
    let response = provider
        .complete(CompletionRequest {
            system: "You are building a client/team coordination artifact from one transcribed \
                     field-work session. Write a narrative summary (2-4 plain sentences: what, \
                     why, when) AND comprehensive notes grouped into three buckets: \
                     scope_of_work (directives with client detail baked in — \"darker mulch than \
                     last year\"), constraints (budget, permits, deadline, site access/gate \
                     codes, client preferences), and conditions_and_issues (site findings \
                     affecting the work). At most 12 notes entries; prefer fewer, denser entries. \
                     Capture only what was said; never invent a budget, deadline, or access \
                     detail — a missed note is cheaper than a fabricated constraint."
                .into(),
            messages: vec![Message::user_text(transcript_excerpt)],
            tools: vec![notes_tool_spec()],
            max_tokens,
            tool_choice: Some(WRITE_NOTES.into()),
            // Single-shot. Note this call carries the SAME transcript the
            // extraction agent just cached — but it can't read that cache: the
            // system prompt and tool set both differ, and tools render at
            // position 0, so the prefix diverges immediately. Sharing it would
            // mean reshaping the two-phase split (see the v1 design doc §2.1).
            cache_prefix: false,
        })
        .await?;

    let tool_input = response.content.iter().find_map(|b| match b {
        ContentBlock::ToolUse { name, input, .. } if name == WRITE_NOTES => Some(input),
        _ => None,
    });
    let summary =
        tool_input.and_then(|i| i.get("summary").and_then(|s| s.as_str()).map(str::to_string));
    let spoken_total_cents =
        tool_input.and_then(|i| i.get("spoken_total_cents").and_then(|s| s.as_i64()));
    // C2: a missing/non-array/garbled `notes` field yields [] via
    // parse_notes_value's tolerant walk — never a panic, never an Err.
    let buckets = tool_input
        .and_then(|i| i.get("notes"))
        .map(parse_notes_value)
        .unwrap_or_default();
    Ok((summary, spoken_total_cents, buckets, response.usage))
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
         - An item is ONE PIECE OF WORK OR ONE MATERIAL, named as a short noun \
         phrase, the way it would appear as a line on paperwork: \"Mulch, three \
         beds\", \"Ten pepper plants\". Never a sentence about the job, never WHO is \
         doing it, never a site or access detail.\n\
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

    /// An item becomes a LINE on a client's estimate, so it has to be shaped
    /// like one. Isaac's EST-0005 went out with "Juan to do mulching and
    /// composting" as a priced line — an assignment sentence, duplicating the
    /// mulch and compost lines above it, with a person's name on the copy the
    /// client reads.
    #[test]
    fn both_extraction_prompts_ask_for_line_shaped_items() {
        for p in [
            extraction_system_prompt(""),
            live_extraction_system_prompt(""),
        ] {
            assert!(p.contains("noun phrase"), "items must be shaped like lines");
            assert!(
                p.contains("ONE PIECE OF WORK OR ONE MATERIAL"),
                "an item is labor or materials, not commentary"
            );
        }
    }

    /// Who does the work, and the site's own facts, have their own homes now
    /// — an assignee column and the notes buckets. Left as items they become
    /// duplicate lines on the paperwork.
    #[test]
    fn the_end_of_session_prompt_keeps_people_and_site_facts_off_the_lines() {
        let p = extraction_system_prompt("");
        assert!(p.contains("WHO does the work is never an item"));
        assert!(p.contains("scope_of_work"), "it says where they go instead");
        assert!(p.contains("gate codes"), "site and access details are named");
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
                name: "write_notes".into(),
                input: serde_json::json!({"summary": "Walked the deck; two todos."}),
            }],
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 40, output_tokens: 12, ..Default::default() },
        }]));
        let (summary, spoken_total_cents, buckets, usage) =
            summarize(provider.clone(), "transcript text", 512).await.unwrap();
        assert_eq!(summary.as_deref(), Some("Walked the deck; two todos."));
        assert_eq!(spoken_total_cents, None, "no total was stated");
        assert_eq!(buckets, Vec::new(), "no notes array in the response -> []");
        assert_eq!(usage, Usage { input_tokens: 40, output_tokens: 12, ..Default::default() });
        let reqs = provider.requests();
        assert_eq!(reqs[0].tool_choice.as_deref(), Some("write_notes"));
        assert!(reqs[0].max_tokens >= 1);
        // the excerpt is the user message verbatim — no extra prefix
        assert_eq!(
            reqs[0].messages[0].content,
            vec![ContentBlock::Text { text: "transcript text".into() }]
        );
    }

    #[tokio::test]
    async fn summarize_returns_the_notes_buckets_when_present() {
        let provider = Arc::new(MockProvider::new(vec![CompletionResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "write_notes".into(),
                input: serde_json::json!({
                    "summary": "Estimate walk on the front yard.",
                    "notes": [
                        {"bucket": "scope_of_work", "label": "Mulch", "detail": "Darker than last year."},
                        {"bucket": "unknown_bucket", "label": "x", "detail": "dropped"}
                    ]
                }),
            }],
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 40, output_tokens: 12, ..Default::default() },
        }]));
        let (_summary, _spoken_total_cents, buckets, _usage) =
            summarize(provider, "t", 512).await.unwrap();
        assert_eq!(
            buckets,
            vec![crate::pipeline::notes::NotesEntry {
                bucket: "scope_of_work".into(),
                label: "Mulch".into(),
                detail: "Darker than last year.".into(),
            }],
            "unknown bucket dropped, valid entry kept"
        );
    }

    #[tokio::test]
    async fn summarize_degrades_a_malformed_notes_field_to_empty_buckets_not_an_error() {
        let provider = Arc::new(MockProvider::new(vec![CompletionResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "write_notes".into(),
                input: serde_json::json!({
                    "summary": "Still a valid summary.",
                    "notes": "not an array — a truncated/garbled response"
                }),
            }],
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 40, output_tokens: 12, ..Default::default() },
        }]));
        let (summary, _spoken_total_cents, buckets, _usage) =
            summarize(provider, "t", 512).await.unwrap();
        assert_eq!(summary.as_deref(), Some("Still a valid summary."), "summary is preserved (R7)");
        assert_eq!(buckets, Vec::new(), "garbled notes -> [] not a hard failure");
    }

    #[test]
    fn notes_prompt_carries_r6_and_the_entry_cap() {
        // The system prompt text is constructed inline in `summarize`; pin the
        // static tool schema's cap/R6 language instead (schema description is
        // sent to the model exactly like the system prompt).
        let spec = notes_tool_spec();
        let notes_desc = spec.input_schema["properties"]["notes"]["description"].as_str().unwrap();
        assert!(notes_desc.contains("At most 12 entries"), "entry-count cap");
        assert!(notes_desc.contains("never invent a budget, deadline"), "R6 clause");
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
            usage: Usage { input_tokens: 50, output_tokens: 10, ..Default::default() },
        }]));
        let (summary, spoken_total_cents, buckets, usage) = summarize(provider, "t", 512).await.unwrap();
        assert!(summary.is_none(), "missing tool call is not an Err — spend must be loggable");
        assert_eq!(spoken_total_cents, None);
        assert_eq!(buckets, Vec::new());
        assert_eq!(usage, Usage { input_tokens: 50, output_tokens: 10, ..Default::default() });
    }

    #[tokio::test]
    async fn summarize_captures_the_spoken_total_when_stated() {
        let provider = Arc::new(MockProvider::new(vec![CompletionResponse {
            content: vec![ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "write_notes".into(),
                input: serde_json::json!({
                    "summary": "Mulch and railing; keep it under twelve hundred.",
                    "spoken_total_cents": 120000
                }),
            }],
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 40, output_tokens: 12, ..Default::default() },
        }]));
        let (summary, spoken_total_cents, _buckets, _usage) =
            summarize(provider, "transcript text", 512).await.unwrap();
        assert!(summary.is_some());
        assert_eq!(spoken_total_cents, Some(120000));
    }
}
