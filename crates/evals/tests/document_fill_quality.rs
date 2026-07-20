//! Plan 19 Stage 6 — the custom-schema fill-quality eval. Hermetic
//! (`MockProvider`), mirrors `carried_scenarios.rs`: drives the REAL
//! `DocumentBuilder` over a processed session built from a small corpus
//! transcript plus a saved custom schema. Measures fill quality = the
//! gap-row posture under the R6 under-extraction bias: a `filled` field NOT
//! mentioned in the walk must render as a truthful gap (the seam does not
//! fabricate), and a mentioned one must fill.
//!
//! No grader change — the grader scores extraction, not documents; this is
//! a characterization pin of the fill contract, the honest analog of the R6
//! distractor-FP signal.

use std::sync::{Arc, Mutex};

use harness::{
    CompletionResponse, ContentBlock, LlmProvider, Memory, MockProvider, StopReason, Usage,
};
use murmur_core::{
    DocumentBuilder, DocumentSchema, ItemSource, SchemaField, SchemaSection, Store,
};

fn tool_use(name: &str, input: serde_json::Value) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse { id: "tu".into(), name: name.into(), input }],
        stop_reason: StopReason::ToolUse,
        usage: Usage { input_tokens: 30, output_tokens: 8 },
    }
}

/// The `punch_list_short` corpus transcript — loaded through the real corpus
/// loader so this eval stays tied to the shipped fixture set.
fn corpus_transcript() -> String {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let scenarios = evals::corpus::load_corpus(&dir).expect("corpus loads");
    scenarios
        .into_iter()
        .find(|s| s.id == "punch_list_short")
        .expect("punch_list_short exists")
        .transcript
}

/// A custom property-trade schema with one `walk` field. `hoa_no` is chosen
/// because NO HOA approval is mentioned anywhere in the punch-list walk.
fn unit_turn_schema() -> DocumentSchema {
    DocumentSchema {
        id: "custom-unit-turn".into(),
        kind: "unit_turn".into(),
        label: "Unit Turn Sheet".into(),
        number_prefix: "TURN".into(),
        trade_key: Some("property".into()),
        total_kind: "sum".into(),
        total_label_key: "total".into(),
        sections: vec![
            SchemaSection {
                key: "line_items".into(),
                kind: "line_items".into(),
                label: "Items".into(),
                priced: false,
                fields: vec![],
            },
            SchemaSection {
                key: "approvals".into(),
                kind: "filled".into(),
                label: "Approvals".into(),
                priced: false,
                fields: vec![
                    SchemaField {
                        key: "hoa_no".into(),
                        kind: "text".into(),
                        label: "HOA approval #".into(),
                        fill: "walk".into(),
                        static_value: None,
                    },
                    SchemaField {
                        key: "unit".into(),
                        kind: "text".into(),
                        label: "Unit".into(),
                        fill: "walk".into(),
                        static_value: None,
                    },
                ],
            },
        ],
        schema_version: 1,
        created_at: 0,
        updated_at: 0,
        device_id: String::new(),
    }
}

/// A processed property session over the corpus transcript, with the custom
/// schema saved. Items mirror the fixture's expected board.
fn processed_session(store: &Store) -> String {
    let session = store.start_session_with_template(None, "property").unwrap();
    let mut ids = Vec::new();
    for (kind, text) in [
        ("todo", "replace kitchen faucet cartridge"),
        ("todo", "get electrician on the dead bedroom outlet"),
        ("todo", "fix sprung closet door hinge"),
    ] {
        ids.push(
            store
                .add_item_with_source(&session.id, kind, text, ItemSource::Authoritative)
                .unwrap()
                .id,
        );
    }
    store.append_transcript(&session.id, &corpus_transcript()).unwrap();
    store.end_and_record_session(&session.id).unwrap();
    store
        .finish_session_processed(
            &session.id,
            "Punch list for unit twelve: faucet cartridge, dead outlet, closet hinge.",
            &Usage::default(),
            &ids,
        )
        .unwrap();
    store.save_document_schema(&unit_turn_schema()).unwrap();
    session.id
}

fn build_fields(
    responses: Vec<CompletionResponse>,
) -> (serde_json::Value, bool) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let store = Store::open_in_memory("eval-device").unwrap();
        let sid = processed_session(&store);
        let store = Arc::new(Mutex::new(store));
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(responses));
        let builder = DocumentBuilder::new(
            provider,
            store.clone(),
            Arc::new(Mutex::new(Memory::default())),
            Arc::new(evals::run::NullMemoryStore),
        );
        let outcome = builder.build(&sid, "unit_turn").await.unwrap();
        let store = store.lock().unwrap();
        let art = store.get_artifact(&outcome.document_artifact_id).unwrap();
        let v: serde_json::Value = serde_json::from_str(&art.body).unwrap();
        (v["fields"].clone(), outcome.queued)
    })
}

/// The mock returns the fill tool with NO entry for the unmentioned field:
/// the rendered row is `value: null, is_gap: true` — the model declined, the
/// seam did not fabricate (R6), and `queued` stays false (nothing to retry).
#[test]
fn custom_field_absent_from_the_transcript_renders_as_a_gap() {
    let (fields, queued) = build_fields(vec![tool_use(
        "fill_fields",
        serde_json::json!({"fields": [{"key": "unit", "value": "12"}]}),
    )]);
    assert_eq!(fields[0]["key"], "hoa_no");
    assert_eq!(fields[0]["value"], serde_json::Value::Null);
    assert_eq!(fields[0]["is_gap"], true, "unmentioned → gap, never fabricated");
    assert!(!queued, "a declined field is not a failed call");
}

/// The positive control: a field the walk DID state fills with the mock's
/// value and is not a gap.
#[test]
fn custom_field_stated_in_the_transcript_is_filled() {
    let (fields, queued) = build_fields(vec![tool_use(
        "fill_fields",
        serde_json::json!({"fields": [{"key": "unit", "value": "12"}]}),
    )]);
    assert_eq!(fields[1]["key"], "unit");
    assert_eq!(fields[1]["value"], "12", "unit twelve is stated in the walk");
    assert_eq!(fields[1]["is_gap"], false);
    assert!(!queued);
}
