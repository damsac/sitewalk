//! Hermetic end-to-end: seed corpus → pipeline (MockProvider) → grade → report.
//! No network, runs under `cargo test --workspace`. Proves the harness wiring;
//! a perfect scripted extraction must grade ~1.0 so real-API regressions are
//! attributable to the model, not the harness.

use std::sync::Arc;

use evals::corpus::load_corpus;
use evals::report::{render_table, SuiteReport};
use evals::run::run_scenario;
use harness::{CompletionResponse, ContentBlock, MockProvider, StopReason, Usage};

fn tool_use(name: &str, input: serde_json::Value) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse { id: "tu".into(), name: name.into(), input }],
        stop_reason: StopReason::ToolUse,
        usage: Usage { input_tokens: 50, output_tokens: 10 },
    }
}
fn end_turn(t: &str) -> CompletionResponse {
    CompletionResponse { content: vec![ContentBlock::Text { text: t.into() }], stop_reason: StopReason::EndTurn, usage: Usage { input_tokens: 20, output_tokens: 4 } }
}
fn summary(t: &str) -> CompletionResponse {
    tool_use("write_summary", serde_json::json!({ "summary": t }))
}

/// Builds a MockProvider script that emits exactly the scenario's ground-truth
/// items + contacts, then a summary — the "perfect model".
fn perfect_script(scenario: &evals::corpus::Scenario) -> Vec<CompletionResponse> {
    let mut r = Vec::new();
    for it in &scenario.truth.items {
        r.push(tool_use("add_item", serde_json::json!({ "kind": it.kind, "text": it.text })));
    }
    for c in &scenario.truth.contacts {
        let mut input = serde_json::json!({ "name": c.name });
        if let Some(t) = &c.trade { input["trade"] = serde_json::json!(t); }
        r.push(tool_use("upsert_contact", input));
    }
    r.push(end_turn("done"));
    r.push(summary("Session processed."));
    r
}

#[tokio::test]
async fn perfect_model_grades_near_one_across_seed_corpus() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");
    let corpus = load_corpus(dir).unwrap();
    let mut reports = Vec::new();
    for scenario in &corpus {
        // empty/near-empty: no items scripted, just a summary
        let script = if scenario.truth.items.is_empty() && scenario.truth.contacts.is_empty() {
            vec![end_turn("nothing"), summary("Nothing notable.")]
        } else {
            perfect_script(scenario)
        };
        let report = run_scenario(scenario, Arc::new(MockProvider::new(script)), "claude-haiku-4-5").await.unwrap();
        reports.push(report);
    }
    let suite = SuiteReport::assemble("claude-haiku-4-5", reports);
    // A perfect scripted extraction must score high — proves harness fidelity.
    assert!(suite.aggregate.mean_f_half > 0.95, "harness lost fidelity: {}", suite.aggregate.mean_f_half);
    assert_eq!(suite.aggregate.mean_distractor_fp_rate, 0.0, "perfect model extracts no distractors");
    // table renders without panic
    let _ = render_table(&suite);
}

#[tokio::test]
async fn over_extracting_model_scores_lower_than_perfect() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");
    let deck = load_corpus(dir).unwrap().into_iter().find(|s| s.id == "deck_walk_contacts").unwrap();
    // perfect
    let perfect = run_scenario(&deck, Arc::new(MockProvider::new(perfect_script(&deck))), "m").await.unwrap();
    // greedy model: perfect + both distractors promoted to todos
    let mut greedy = perfect_script(&deck);
    let insert_at = greedy.len() - 2; // before end_turn + summary
    for d in &deck.truth.distractors {
        greedy.insert(insert_at, tool_use("add_item", serde_json::json!({ "kind": "todo", "text": d })));
    }
    let over = run_scenario(&deck, Arc::new(MockProvider::new(greedy)), "m").await.unwrap();
    assert!(over.score.distractor_fp_rate > 0.0, "distractors should be caught");
    assert!(over.score.f_half < perfect.score.f_half, "R6: over-extraction must score lower");
}
