//! #298 — the summary-voice eval, hermetic. Mirrors `document_fill_quality.rs`:
//! drives the REAL pipeline over a real corpus scenario with a `MockProvider`,
//! then reads the graded result. Under a mock the summary text is whatever the
//! script emits, so this is not a measurement of the model — it is the pin that
//! the defect is VISIBLE END TO END. Before #298 it could not have been: the
//! suite read `summary_present` and nothing else, so the summaries Isaac hit on
//! TestFlight scored `summaries ok: 4/4`.
//!
//! Movement on real output comes from the gated runner
//! (`cargo run -p evals --example eval`), which now prints each summary and its
//! voice verdict.

use std::sync::Arc;

use evals::corpus::{load_corpus, Scenario};
use evals::run::run_scenario;
use harness::{CompletionResponse, ContentBlock, MockProvider, StopReason, Usage};

/// Verbatim from a device, quoted in #298.
const DEVICE_SUMMARY: &str = "Field session to discuss mulch work. Only the word \"mulch\" was \
                              clearly audible in the recording, with no additional context \
                              provided about scope, timing, or constraints.";

/// What the same walk should read like: the job, and then stop.
const RECORD_OF_THE_JOB: &str = "Mulch work discussed.";

fn tool_use(name: &str, input: serde_json::Value) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse { id: "tu".into(), name: name.into(), input }],
        stop_reason: StopReason::ToolUse,
        usage: Usage { input_tokens: 30, output_tokens: 8, ..Default::default() },
    }
}

fn end_turn(t: &str) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::Text { text: t.into() }],
        stop_reason: StopReason::EndTurn,
        usage: Usage { input_tokens: 10, output_tokens: 2, ..Default::default() },
    }
}

/// A script that extracts nothing and writes `summary` — the quiet-walk shape
/// the issue is about.
fn quiet_walk_script(summary: &str) -> Vec<CompletionResponse> {
    vec![
        end_turn("nothing to capture"),
        tool_use("write_notes", serde_json::json!({ "summary": summary })),
    ]
}

fn empty_scenario() -> Scenario {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");
    load_corpus(dir)
        .unwrap()
        .into_iter()
        .find(|s| s.id == "empty_session")
        .expect("empty_session scenario in corpus")
}

#[tokio::test]
async fn the_device_summary_is_caught_end_to_end() {
    let scenario = empty_scenario();
    let provider = Arc::new(MockProvider::new(quiet_walk_script(DEVICE_SUMMARY)));
    let report = run_scenario(&scenario, provider, "claude-haiku-4-5").await.unwrap();

    let voice = &report.score.summary_voice;
    assert!(!voice.ok, "the summary Isaac hit must not score as ok");
    assert_eq!(voice.preamble.as_deref(), Some("field session"));
    assert!(voice.narration.contains(&"recording".to_string()));
    assert_eq!(report.summary.as_deref(), Some(DEVICE_SUMMARY), "reported verbatim");

    // And the extraction score is untouched by it — the two defects are
    // separately visible, which is the whole point of a second axis.
    assert!(report.score.summary_ok, "a present summary is still present");
}

#[tokio::test]
async fn a_record_of_the_job_passes_the_same_path() {
    let scenario = empty_scenario();
    let provider = Arc::new(MockProvider::new(quiet_walk_script(RECORD_OF_THE_JOB)));
    let report = run_scenario(&scenario, provider, "claude-haiku-4-5").await.unwrap();

    let voice = &report.score.summary_voice;
    assert!(voice.ok, "a short true line is the good outcome: {voice:?}");
    assert_eq!(voice.sentences, 1);
    assert!(voice.words < 10, "and it is short: {} words", voice.words);
}
