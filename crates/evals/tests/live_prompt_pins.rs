//! Hermetic live-prompt pins (Plan 09 Task 6, D7). Two pins over the LIVE path
//! (`LiveExtractor`), both under a deterministic `MockProvider` — no key, no
//! network, runs under `cargo test --workspace`:
//!
//!   1. GOLDEN assembled-prompt snapshot — the TRUE regression gate. A live-prompt
//!      edit diffs the committed golden and forces a conscious re-bless
//!      (`MURMUR_BLESS=1`). Under a mock the board items are whatever the script
//!      emits (near-circular), so the honest signal is the assembled REQUEST text
//!      the extractor sends, exactly as `carried_scenarios.rs` asserts.
//!   2. GRADER-over-live-board — pins the plumbing (grader + swap-at-finish board
//!      read) to a fixed F0.5 for the canned script.
//!
//! NOTE (D7): non-circular F0.5 MOVEMENT from live-prompt edits needs the gated
//! real-API runner (`examples/eval.rs`) extended to the live path — deferred to
//! the optimization-loop work, not built here.

use std::sync::Arc;

use evals::corpus::{load_corpus, Scenario};
use evals::run::run_live_scenario;
use harness::{CompletionResponse, ContentBlock, MockProvider, StopReason, Usage};

fn tool_use(name: &str, input: serde_json::Value) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse { id: "tu".into(), name: name.into(), input }],
        stop_reason: StopReason::ToolUse,
        usage: Usage { input_tokens: 30, output_tokens: 8 },
    }
}

fn end_turn(t: &str) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::Text { text: t.into() }],
        stop_reason: StopReason::EndTurn,
        usage: Usage { input_tokens: 10, output_tokens: 2 },
    }
}

/// The canned "perfect" live extraction of `punch_list_short`: its three
/// ground-truth todos, then an end turn. Deterministic → a fixed board + F0.5.
fn punch_list_script() -> Vec<CompletionResponse> {
    vec![
        tool_use("add_item", serde_json::json!({"kind":"todo","text":"replace kitchen faucet cartridge, still dripping"})),
        tool_use("add_item", serde_json::json!({"kind":"todo","text":"electrician back on dead bedroom outlet by window"})),
        tool_use("add_item", serde_json::json!({"kind":"todo","text":"fix closet door latch, sprung hinge"})),
        end_turn("captured"),
    ]
}

// The golden lives in a `golden/` SUBDIR of `fixtures/`, not directly in it:
// `corpus::load_corpus` treats every loose `.txt` in `fixtures/` as one half of
// a paired corpus scenario and errors on a `.txt` with no matching `.json`. A
// subdir is invisible to its non-recursive `read_dir` scan, so the golden stays
// committed under the evals fixtures tree without tripping that invariant.
const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/golden/live_prompt_golden.txt");

fn punch_list_scenario() -> Scenario {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");
    load_corpus(dir).unwrap().into_iter().find(|s| s.id == "punch_list_short")
        .expect("punch_list_short scenario in corpus")
}

#[tokio::test]
async fn live_prompt_matches_golden_snapshot() {
    let scenario = punch_list_scenario();
    let provider = Arc::new(MockProvider::new(punch_list_script()));
    let (_score, prompt) = run_live_scenario(&scenario, provider, "claude-haiku-4-5").await.unwrap();

    // Re-bless escape: MURMUR_BLESS=1 (or a missing golden on first authoring)
    // rewrites the committed snapshot, so a DELIBERATE prompt change is a
    // conscious act while an accidental one is a red test.
    let bless = std::env::var("MURMUR_BLESS").as_deref() == Ok("1");
    if bless || !std::path::Path::new(GOLDEN).exists() {
        std::fs::write(GOLDEN, &prompt).unwrap();
    }
    let golden = std::fs::read_to_string(GOLDEN).unwrap();
    assert_eq!(
        prompt.trim_end(),
        golden.trim_end(),
        "assembled live prompt drifted from the golden — re-bless with MURMUR_BLESS=1 if intentional"
    );
}

#[tokio::test]
async fn live_board_grades_to_a_fixed_f_half() {
    let scenario = punch_list_scenario();
    let provider = Arc::new(MockProvider::new(punch_list_script()));
    let (score, _prompt) = run_live_scenario(&scenario, provider, "claude-haiku-4-5").await.unwrap();
    // Deterministic for the canned script — pins grader + swap-at-finish board read.
    assert!(
        (score.f_half - EXPECTED_F_HALF).abs() < 1e-9,
        "live-board F0.5 moved: got {}, pinned {EXPECTED_F_HALF}",
        score.f_half
    );
}

// Pinned by running the deterministic "perfect" script once: the three scripted
// todos exactly match the three ground-truth items (no distractors) → F0.5 = 1.0.
const EXPECTED_F_HALF: f64 = 1.0;
