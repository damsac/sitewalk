//! #357/#359 — document accuracy end to end: is what was said on the page,
//! and is it on the right line?
//!
//! Mirrors `summary_voice.rs`: drives the REAL `DocumentBuilder` over a real
//! session, then harvests the built artifact and grades it on both axes.
//! Under a mock the composition is whatever the script emits, so these are
//! not measurements of the model — they are the pin that both defects are
//! VISIBLE THROUGH THE SHIPPING PATH. Before this, neither was: a work order
//! that assigned Dana to the compost she was never mentioned with scored a
//! clean F0.5 and a clean summary voice, because every item was right and
//! only the ATTACHMENT was wrong.
//!
//! Movement on real output comes from the gated test at the bottom, which
//! builds five document kinds across both trades against the real API and
//! prints every misplacement and every invention.

use std::sync::{Arc, Mutex};

use evals::binding::{document_bindings, grade_bindings};
use evals::grounding::document_grounding;
use harness::{
    CompletionResponse, ContentBlock, LlmProvider, Memory, MockProvider, StopReason, Usage,
};
use murmur_core::{DocumentBuilder, ItemSource, Store};

/// Isaac's landscape walk, 2026-08-16 — the one that produced #357. Dana is
/// named against the weed eating and nothing else.
const LANDSCAPE: &str = "Front yard at the Kesler place. We're doing four yards of dark \
mulch in the front beds, and two bags of compost worked into the rose bed. Strip the old \
bark out before the mulch goes down. Weed eating along the fence line, about forty feet. \
Two hundred for the mulch, ninety for the compost, three hundred for the labor. \
Marcus is stripping the bark and laying mulch, Dana takes the weed eating. Gate code is \
7781.";

/// The property walk from the same day — the one whose move-out and condition
/// reports both put "normal wear" on the kitchen faucet (#359).
const PROPERTY: &str = "Unit four at 220 Bell, move-out walk, keys came back yesterday. \
There's a burn in the bedroom carpet near the window, that's damage. Scuffs down the \
hallway are normal wear. The kitchen faucet drips. Screen on the back door is torn. \
Lockbox is 4419.";

/// The trade phrases with consequences on a property document: they decide
/// what may lawfully be withheld from a deposit, so landing one on the wrong
/// item is not a wording problem.
const CLASSIFICATIONS: &[&str] = &["normal wear", "damage", "tenant damage"];

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

/// The invention half, through the same shipping path: a compose pass that
/// writes a gate code the operator never said puts it on the page as an
/// ordinary filled field, indistinguishable from a true one — present, well
/// worded, and wrong.
#[test]
fn an_invented_field_value_is_caught_end_to_end() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (real, invented) = rt.block_on(async {
        let mut out = Vec::new();
        for access in ["Gate code 7781", "Gate code 4412"] {
            let store = Store::open_in_memory("eval-device").unwrap();
            let (sid, _) = processed_session(&store);
            let store = Arc::new(Mutex::new(store));
            let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![tool_use(
                "compose_document",
                serde_json::json!({ "fields": [{"key": "access", "value": access}] }),
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
            out.push(document_grounding(&document, LANDSCAPE));
        }
        (out.remove(0), out.remove(0))
    });

    assert!(real.ok, "the code the walk states must not read as invented: {:?}", real.findings);
    assert!(real.checked > 0, "and it must actually have been checked");
    assert!(!invented.ok, "a gate code nobody said must not land silently");
    assert_eq!(invented.findings[0].tokens, vec!["4412"]);
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
/// times in three), so this reports and asserts only that both axes are
/// well-formed on real output. Read the printed verdicts.
///
/// ```sh
/// ANTHROPIC_API_KEY=sk-... cargo test -p evals --test document_accuracy \
///     -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore = "hits the real Anthropic API; set ANTHROPIC_API_KEY and run with --ignored"]
async fn real_documents_are_scored_on_both_axes() {
    let api_key = std::env::var("ANTHROPIC_API_KEY").expect("set ANTHROPIC_API_KEY");
    let model = std::env::var("SMOKE_MODEL").unwrap_or_else(|_| "claude-sonnet-4-5".into());
    let provider = Arc::new(harness::AnthropicProvider::from_env(api_key, model));

    // Both trades, and every document kind that carries money, crew or a
    // classification — the three slots an invention can hide in.
    for (template, transcript, kinds) in [
        ("landscape", LANDSCAPE, &["work_order", "estimate", "invoice"][..]),
        ("property", PROPERTY, &["move_out", "condition"][..]),
    ] {
        let store = Store::open_in_memory("eval-device").unwrap();
        let session = store.start_session_with_template(None, template).unwrap();
        store.append_transcript(&session.id, transcript).unwrap();
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
            provider.clone(),
            store.clone(),
            Arc::new(Mutex::new(Memory::default())),
            Arc::new(evals::run::NullMemoryStore),
        );

        for kind in kinds {
            let outcome = builder.build(&sid, kind).await.unwrap();
            let document: serde_json::Value = {
                let guard = store.lock().unwrap();
                serde_json::from_str(
                    &guard.get_artifact(&outcome.document_artifact_id).unwrap().body,
                )
                .unwrap()
            };

            let bound = grade_bindings(&document_bindings(&document, CLASSIFICATIONS), transcript);
            let ground = document_grounding(&document, transcript);
            println!(
                "\n===== {kind} =====\nbindings: {} supported / {} unsupported     \
                 grounding: {} checked / {} ungrounded",
                bound.supported,
                bound.unsupported,
                ground.checked,
                ground.findings.len()
            );
            for v in bound.verdicts.iter().filter(|v| !v.supported) {
                println!(
                    "  MISPLACED  {} → {}\n             said: {}",
                    v.attribute,
                    v.item,
                    v.sentence.as_deref().unwrap_or("(never spoken)")
                );
            }
            for f in &ground.findings {
                println!("  INVENTED   {} in {} — {:?}", f.slot, f.value, f.tokens);
            }

            assert_eq!(bound.supported + bound.unsupported, bound.verdicts.len());
            assert!(ground.checked >= ground.findings.len());
        }
    }
}
