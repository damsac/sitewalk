//! Transcript-in -> records/summary/reflection-out, over scripted LLM
//! responses (spec §10: E2E integration tests in Rust).

use std::sync::{Arc, Mutex};

use harness::{
    CompletionResponse, ContentBlock, HarnessError, Memory, MemoryStore, MockProvider,
    StopReason, Usage,
};
use murmur_core::{
    NewJob, ReflectionCoordinator, SessionProcessor, SessionStatus, Store,
};

struct NullMemoryStore;
impl MemoryStore for NullMemoryStore {
    fn load(&self) -> Result<Memory, HarnessError> {
        Ok(Memory::default())
    }
    fn save(&self, _m: &Memory) -> Result<(), HarnessError> {
        Ok(())
    }
}

fn tool_use(name: &str, input: serde_json::Value) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse { id: "tu".into(), name: name.into(), input }],
        stop_reason: StopReason::ToolUse,
        usage: Usage { input_tokens: 100, output_tokens: 25 },
    }
}

fn end_turn(text: &str) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::Text { text: text.into() }],
        stop_reason: StopReason::EndTurn,
        usage: Usage { input_tokens: 60, output_tokens: 10 },
    }
}

#[tokio::test]
async fn site_walk_end_to_end() {
    // A day in the life: job, session, walk, process, reflect.
    let store = Store::open_in_memory("marcos-phone").unwrap();
    let job = store
        .create_job(NewJob { name: "Johnson remodel".into(), ..Default::default() })
        .unwrap();
    let session = store.start_session(Some(&job.id)).unwrap();
    store
        .append_transcript(
            &session.id,
            "okay deck framing is soft near the ledger, Dev needs to sister two joists. \
             order twelve two-by-tens. tell the client the railing decision can wait.",
        )
        .unwrap();
    store.end_and_record_session(&session.id).unwrap();

    let store = Arc::new(Mutex::new(store));
    let memory = Arc::new(Mutex::new(Memory::default()));
    let memory_store: Arc<dyn MemoryStore> = Arc::new(NullMemoryStore);

    // Processing: two items, a contact, a report, then summary.
    let processor = SessionProcessor::new(
        Arc::new(MockProvider::new(vec![
            tool_use("add_item", serde_json::json!({"kind": "todo", "text": "order twelve 2x10s"})),
            tool_use("add_item", serde_json::json!({"kind": "safety", "text": "deck framing soft near ledger"})),
            tool_use("upsert_contact", serde_json::json!({"name": "Dev", "trade": "framer"})),
            tool_use("write_report", serde_json::json!({"title": "Johnson walk", "body": "## Deck\nSister two joists."})),
            end_turn("done"),
            tool_use("write_summary", serde_json::json!({"summary": "Deck walk: framing fix planned, lumber ordered."})),
        ])),
        store.clone(),
        memory.clone(),
        memory_store.clone(),
    );
    let results = processor.process_pending().await.unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].1.is_ok());

    let processing_totals = {
        let s = store.lock().unwrap();
        let processed = s.get_session(&session.id).unwrap();
        assert_eq!(processed.status, SessionStatus::Processed);
        assert_eq!(s.list_items_for_session(&session.id).unwrap().len(), 2);
        let artifacts = s.list_artifacts_for_session(&session.id).unwrap();
        assert_eq!(artifacts.len(), 1, "just the write_report artifact — phase B is gone (Plan 13 Stage 2)");
        assert!(artifacts.iter().any(|a| a.kind == "report"));
        assert!(!artifacts.iter().any(|a| a.kind == "document"), "no auto-built document at finish");
        assert_eq!(s.list_contacts().unwrap().len(), 1);
        assert_eq!(s.list_open_todos().unwrap().len(), 1);
        let (input, output) = s.usage_totals().unwrap();
        assert!(input > 0 && output > 0, "R9: cost logged");
        // the session library sees the summary without the transcript
        let summaries = s.list_session_summaries().unwrap();
        assert_eq!(summaries[0].summary.as_deref(), Some("Deck walk: framing fix planned, lumber ordered."));
        (input, output)
    };

    // Reflection: warmup cadence -> reflect on the session's summary.
    let coordinator = ReflectionCoordinator::new(
        Arc::new(MockProvider::new(vec![tool_use(
            "write_memory",
            serde_json::json!({"sections": {"people": ["Dev — framer"], "vocabulary": ["sister joists"]}}),
        )])),
        store.clone(),
        memory.clone(),
        memory_store,
    );
    let churn = coordinator.maybe_reflect().await.unwrap();
    assert!(churn.is_some());
    assert_eq!(memory.lock().unwrap().section_texts("people"), vec!["Dev — framer"]);
    let s = store.lock().unwrap();
    let signals = s.reflection_signals().unwrap();
    assert_eq!(signals.completed_reflections, 1);
    // reflection cost is logged on top of the processing spend (R9 wiring)
    let (input_after, output_after) = s.usage_totals().unwrap();
    assert!(
        input_after > processing_totals.0 && output_after > processing_totals.1,
        "reflection usage adds to the spend meter"
    );
}
