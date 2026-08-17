//! #357/#359 — the attribute-binding eval, end to end.
//!
//! Mirrors `summary_voice.rs`: drives the REAL `DocumentBuilder` over a real
//! session, then harvests the built artifact and grades it. Under a mock the
//! composition is whatever the script emits, so this is not a measurement of
//! the model — it is the pin that the defect is VISIBLE THROUGH THE SHIPPING
//! PATH. Before this, it was not: a work order that assigned Dana to the
//! compost she was never mentioned with scored a clean F0.5 and a clean
//! summary voice, because every item was right and only the ATTACHMENT was
//! wrong.
//!
//! Movement on real output comes from the gated test at the bottom, which
//! builds the same work order against the real API and prints every binding
//! with the sentence that does or does not support it.

use std::sync::{Arc, Mutex};

use evals::binding::{document_bindings, grade_bindings};
use harness::{
    CompletionResponse, ContentBlock, LlmProvider, Memory, MockProvider, StopReason, Usage,
};
use murmur_core::{DocumentBuilder, ItemSource, Store};

/// Isaac's landscape walk, 2026-08-16 — the one that produced #357. Dana is
/// named against the weed eating and nothing else.
const LANDSCAPE: &str = "Front yard at the Kesler place. We're doing four yards of dark \
mulch in the front beds, and two bags of compost worked into the rose bed. Strip the old \
bark out before the mulch goes down. Weed eating along the fence line, about forty feet. \
Marcus is stripping the bark and laying mulch, Dana takes the weed eating. Gate code is \
7781.";

fn tool_use(name: &str, input: serde_json::Value) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse { id: "tu".into(), name: name.into(), input }],
        stop_reason: StopReason::ToolUse,
        usage: Usage { input_tokens: 30, output_tokens: 8, ..Default::default() },
    }
}

/// A processed landscape session over the walk above, with the four work
/// items the walk states. Returns the session id and the item ids in order.
fn processed_session(store: &Store) -> (String, Vec<String>) {
    let session = store.start_session_with_template(None, "landscape").unwrap();
    let mut ids = Vec::new();
    for text in [
        "Strip old bark, front beds",
        "Dark mulch, 4 yards, front beds",
        "Compost, 2 bags, worked into rose bed",
        "Weed eating, fence line, 40 ft",
    ] {
        ids.push(
            store
                .add_item_with_source(&session.id, "todo", text, ItemSource::Authoritative)
                .unwrap()
                .id,
        );
    }
    store.append_transcript(&session.id, LANDSCAPE).unwrap();
    store.end_and_record_session(&session.id).unwrap();
    store
        .finish_session_processed(
            &session.id,
            "Mulch, compost and weed eating at the Kesler front yard.",
            &Usage::default(),
            &ids,
        )
        .unwrap();
    (session.id, ids)
}

/// Builds a work order whose compose pass assigns crew as `assign` says
/// (item index → name), then returns the graded bindings.
fn work_order_bindings(assign: &[(usize, &str)]) -> evals::binding::BindingScore {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let store = Store::open_in_memory("eval-device").unwrap();
        let (sid, ids) = processed_session(&store);
        let lines: Vec<serde_json::Value> = assign
            .iter()
            .map(|(i, who)| {
                serde_json::json!({
                    "item_id": ids[*i], "detail": "as walked", "assignee": who
                })
            })
            .collect();
        let store = Arc::new(Mutex::new(store));
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![tool_use(
            "compose_document",
            serde_json::json!({ "fields": [], "lines": lines }),
        )]));
        let builder = DocumentBuilder::new(
            provider,
            store.clone(),
            Arc::new(Mutex::new(Memory::default())),
            Arc::new(evals::run::NullMemoryStore),
        );
        let outcome = builder.build(&sid, "work_order").await.unwrap();
        let guard = store.lock().unwrap();
        let art = guard.get_artifact(&outcome.document_artifact_id).unwrap();
        let document: serde_json::Value = serde_json::from_str(&art.body).unwrap();
        grade_bindings(&document_bindings(&document, &[]), LANDSCAPE)
    })
}

/// #357 as it shipped. Marcus is right, Dana is on the compost — and the
/// compost is the one line the walk never says her name near.
#[test]
fn the_misplaced_assignee_is_caught_end_to_end() {
    let score = work_order_bindings(&[(0, "Marcus"), (2, "Dana")]);
    assert!(!score.ok, "a name on work nobody assigned must not score as ok");
    assert_eq!(score.unsupported, 1);
    let bad = score.verdicts.iter().find(|v| !v.supported).unwrap();
    assert_eq!(bad.attribute, "Dana");
    assert!(bad.item.to_lowercase().contains("compost"));
    // And the sentence is reported, so the failure reads as an argument
    // rather than a boolean: here is what was said, and it isn't this.
    assert!(bad.sentence.as_deref().unwrap().contains("Dana takes the weed eating"));
}

/// The same document, correct: both names on the work they were spoken with.
#[test]
fn assignees_on_the_work_they_were_spoken_with_pass_the_same_path() {
    let score = work_order_bindings(&[(0, "Marcus"), (3, "Dana")]);
    assert!(score.ok, "the correct assignment must pass: {:?}", score.verdicts);
    assert_eq!(score.supported, 2);
}

/// A work order that names nobody is `ok` — R6's posture, made scoreable. An
/// axis that punished blanks would push the model toward exactly the
/// confident invention #357 is about.
#[test]
fn a_work_order_that_assigns_nobody_is_ok() {
    let score = work_order_bindings(&[]);
    assert!(score.ok);
    assert_eq!(score.supported, 0);
}

/// The gated real run — the WHOLE path, transcript to document, because the
/// compose pass is not given the transcript. It writes from the summary and
/// the notes artifact that `SessionProcessor` produced at finish, so a walk
/// staged with items alone composes nothing at all (verified: the model
/// returned `{"fields": [], "lines": []}`, correctly, having been handed
/// nothing to write from).
///
/// That is itself the shape of #357. "Dana takes the weed eating" survives
/// into the document only if extraction kept it as ONE fact; if the notes
/// bundle it with the mulch, the compose pass is choosing a line from a
/// sentence that names two.
///
/// No thresholds — one run cannot separate a defect from variance (the
/// work-order instructions block took three runs to reveal it was empty two
/// times in three), so this reports and asserts only that the axis is
/// well-formed on real output. Read the printed verdicts.
///
/// ```sh
/// ANTHROPIC_API_KEY=sk-... cargo test -p evals --test attribute_binding \
///     -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore = "hits the real Anthropic API; set ANTHROPIC_API_KEY and run with --ignored"]
async fn real_work_order_bindings_are_reported() {
    let api_key = std::env::var("ANTHROPIC_API_KEY").expect("set ANTHROPIC_API_KEY");
    let model = std::env::var("SMOKE_MODEL").unwrap_or_else(|_| "claude-sonnet-4-5".into());
    let provider = Arc::new(harness::AnthropicProvider::from_env(api_key, model));

    let store = Store::open_in_memory("eval-device").unwrap();
    let session = store.start_session_with_template(None, "landscape").unwrap();
    store.append_transcript(&session.id, LANDSCAPE).unwrap();
    store.end_and_record_session(&session.id).unwrap();
    let sid = session.id.clone();
    let store = Arc::new(Mutex::new(store));

    murmur_core::SessionProcessor::new(
        provider.clone(),
        store.clone(),
        Arc::new(Mutex::new(Memory::default())),
        Arc::new(evals::run::NullMemoryStore),
    )
    .process(&sid)
    .await
    .expect("the walk processes");

    let builder = DocumentBuilder::new(
        provider,
        store.clone(),
        Arc::new(Mutex::new(Memory::default())),
        Arc::new(evals::run::NullMemoryStore),
    );
    let outcome = builder.build(&sid, "work_order").await.unwrap();

    let document: serde_json::Value = {
        let guard = store.lock().unwrap();
        serde_json::from_str(&guard.get_artifact(&outcome.document_artifact_id).unwrap().body)
            .unwrap()
    };
    assert!(!outcome.queued, "the compose pass failed and the document fell back to a retry queue");

    let score = grade_bindings(&document_bindings(&document, &[]), LANDSCAPE);
    println!("\n--- bindings: {} supported, {} unsupported ---", score.supported, score.unsupported);
    for v in &score.verdicts {
        println!(
            "  [{}] {} → {}\n      said: {}",
            if v.supported { "ok " } else { "BAD" },
            v.attribute,
            v.item,
            v.sentence.as_deref().unwrap_or("(never spoken)")
        );
    }
    assert_eq!(score.supported + score.unsupported, score.verdicts.len());
}
