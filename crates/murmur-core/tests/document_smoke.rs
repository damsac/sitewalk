//! Env-gated smoke test for the DOCUMENT path against the real API. Ignored
//! by default — it costs real tokens and needs a key. Run explicitly with:
//!
//! ```sh
//! ANTHROPIC_API_KEY=sk-... ANTHROPIC_BASE_URL=https://api.ppq.ai \
//!     cargo test -p murmur-core --test document_smoke -- --ignored --nocapture
//! ```
//!
//! Sibling of `anthropic_smoke.rs`, one layer further along: that one proves a
//! walk processes, this one proves the walk becomes a DOCUMENT.
//!
//! It exists because every other test of the compose pass uses `MockProvider`,
//! which proves the plumbing and nothing about the writing. The failure this
//! catches is the one mocks structurally cannot: a prompt that parses fine and
//! produces a document nobody would send — a "directive" that just restates
//! the item, an invented gate code, a name attached to a line nobody was
//! assigned. `--nocapture` prints the document so a human can read it, because
//! the interesting half of this is not assertable.
//!
//! The assertions are the ones that CAN be made mechanically: the structure
//! survives, every line keeps the operator's own words, and nothing the model
//! wrote about a line landed on a line it wasn't about.

use std::sync::{Arc, Mutex};

use harness::{AnthropicProvider, HarnessError, Memory, MemoryStore};
use murmur_core::{DocumentBuilder, SessionProcessor, Store};

/// The model under test, defaulting to the one the SHIPPED app builds
/// documents with — `EngineResolution.modelProcessing` → `providers.processing`
/// → `DocumentBuilder`.
///
/// It defaults to the real thing on purpose, and the purpose was learned the
/// hard way: this test first ran on haiku (cheaper, and the sibling smoke
/// test's choice) and reported a pricing failure that was then quoted as
/// production behaviour. Sonnet failed differently and by a different amount.
/// A smoke test pointed at a model nobody ships tells you about that model.
/// Override with `SMOKE_MODEL=` when you specifically want to compare.
fn model() -> String {
    std::env::var("SMOKE_MODEL").unwrap_or_else(|_| "claude-sonnet-4-5".to_string())
}

/// A walk that names people against tasks — the thing Isaac asked for, and
/// the case a mock cannot evaluate.
const TRANSCRIPT: &str = "Okay, front yard at the Hollis place. We're doing five yards \
of dark mulch in the front beds, and two bags of compost worked into the rose bed. \
Jose is going to strip the old bark out first and lay the mulch, Michael takes the \
edging along the walkway, sixty feet of it, spade edge. Gate code is 4412 on the side \
yard, park on the street because they're sealing the driveway Thursday. Watch the \
irrigation heads along that walkway bed, they sit shallow. Dog's in the back until \
about eight. We're starting Thursday first thing.";

struct NullMemoryStore;
impl MemoryStore for NullMemoryStore {
    fn load(&self) -> Result<Memory, HarnessError> {
        Ok(Memory::default())
    }
    fn save(&self, _m: &Memory) -> Result<(), HarnessError> {
        Ok(())
    }
}

/// Isaac's own EST-0005 walk, verbatim in shape: he names people against
/// tasks and rattles off materials. The line titles are what this pins —
/// that build produced "Juan to do mulching and composting" as a priced
/// line on a client's estimate.
const ESTIMATE_TRANSCRIPT: &str = "Alright, three beds here. We're going to weed them, \
compost them, and mulch them, get them ready for planting. Ten peppers, ten tomatoes, \
five artichokes going into those beds. Juan is going to do the mulching and the \
composting. Christos is going to prune the five pear trees in the backyard. Two hundred \
for the weeding and compost, three hundred for the mulch, two fifty for the plants, and \
five hundred labor.";

/// Isaac's flagstone walk (2026-08-10), the one that produced the estimate
/// with "Jose and John, $300" under a line and his $300 of stone attached to
/// the grading task. Every price here is stated; nothing should be invented,
/// no crew name should reach the page, and no dollar figure should appear in
/// a description.
const FLAGSTONE_TRANSCRIPT: &str = "We're putting a flagstone path in the existing area \
here. Grade and weed the path area first to get it flat, then lay the gravel down, then \
the flagstone goes in. Three hundred for the flagstone, two hundred for the gravel, three \
hundred for the labor — Jose and John are on the flagstone path. Then the backyard \
perimeter needs weed whacking, William's on the weed whacker, two hundred labor plus ten \
dollars of gas. And the poison oak has to come out, that's two hundred regular plus a \
hundred hazard pay.";

#[tokio::test]
#[ignore = "hits the real API; set ANTHROPIC_API_KEY and run with --ignored"]
async fn a_real_estimate_carries_only_what_an_estimate_needs() {
    let api_key = std::env::var("ANTHROPIC_API_KEY").expect("set ANTHROPIC_API_KEY");
    let provider = Arc::new(AnthropicProvider::from_env(api_key, model()));

    let store = Store::open_in_memory("smoke-device").unwrap();
    let session = store.start_session_with_template(None, "landscape").unwrap();
    store.append_transcript(&session.id, FLAGSTONE_TRANSCRIPT).unwrap();
    store.end_and_record_session(&session.id).unwrap();
    let store = Arc::new(Mutex::new(store));

    SessionProcessor::new(
        provider.clone(),
        store.clone(),
        Arc::new(Mutex::new(Memory::default())),
        Arc::new(NullMemoryStore),
    )
    .process(&session.id)
    .await
    .expect("processing failed");

    let outcome = DocumentBuilder::new(
        provider,
        store.clone(),
        Arc::new(Mutex::new(Memory::default())),
        Arc::new(NullMemoryStore),
    )
    .build(&session.id, "estimate")
    .await
    .expect("build failed");

    let guard = store.lock().unwrap();
    let artifact = guard.get_artifact(&outcome.document_artifact_id).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&artifact.body).unwrap();

    println!("\n===== FLAGSTONE ESTIMATE =====");
    let mut total = 0i64;
    for line in doc["lines"].as_array().unwrap() {
        let cents = line["amount_cents"].as_i64();
        total += cents.unwrap_or(0);
        println!(
            "  {:<46} {}\n      {}",
            line["title"].as_str().unwrap_or(""),
            cents.map_or("——".to_string(), |c| format!("${}", c / 100)),
            line["detail"].as_str().unwrap_or("")
        );
    }
    println!("  TOTAL ${}\n==============================\n", total / 100);

    for line in doc["lines"].as_array().unwrap() {
        let title = line["title"].as_str().unwrap();
        let detail = line["detail"].as_str().unwrap_or("");
        // A crew name is a work-order fact — never on the client's copy.
        for name in ["Jose", "John", "William"] {
            assert!(!detail.contains(name), "a crew name reached the estimate: {detail:?}");
            assert!(!title.contains(name), "a crew name reached a line title: {title:?}");
        }
        // The amount has its own column; a figure in the text repeats or
        // contradicts it (Isaac saw "$200 labor plus $10 gas" against $10).
        assert!(!detail.contains('$'), "a dollar figure in a description: {detail:?}");
        assert!(!title.contains('$'), "a dollar figure in a line title: {title:?}");
        assert_ne!(line["amount_cents"], serde_json::json!(0), "a zero-dollar line: {title:?}");
    }
}

#[tokio::test]
#[ignore = "hits the real API; set ANTHROPIC_API_KEY and run with --ignored"]
async fn a_real_estimates_lines_read_like_line_items() {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .expect("set ANTHROPIC_API_KEY to run the real-provider document smoke test");
    let provider = Arc::new(AnthropicProvider::from_env(api_key, model()));

    let store = Store::open_in_memory("smoke-device").unwrap();
    let session = store.start_session_with_template(None, "landscape").unwrap();
    store.append_transcript(&session.id, ESTIMATE_TRANSCRIPT).unwrap();
    store.end_and_record_session(&session.id).unwrap();
    let store = Arc::new(Mutex::new(store));

    SessionProcessor::new(
        provider.clone(),
        store.clone(),
        Arc::new(Mutex::new(Memory::default())),
        Arc::new(NullMemoryStore),
    )
    .process(&session.id)
    .await
    .expect("processing failed");

    let builder = DocumentBuilder::new(
        provider,
        store.clone(),
        Arc::new(Mutex::new(Memory::default())),
        Arc::new(NullMemoryStore),
    );
    let outcome = builder.build(&session.id, "estimate").await.expect("build failed");

    let guard = store.lock().unwrap();
    let artifact = guard.get_artifact(&outcome.document_artifact_id).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&artifact.body).unwrap();

    println!("\n===== ESTIMATE =====");
    for field in doc["fields"].as_array().unwrap() {
        println!("[{}] {}", field["label"].as_str().unwrap_or(""), field["value"].as_str().unwrap_or("—— (gap)"));
    }
    for line in doc["lines"].as_array().unwrap() {
        println!(
            "  {:<52} {}",
            line["title"].as_str().unwrap_or(""),
            line["amount_cents"]
        );
    }
    println!("====================\n");

    for line in doc["lines"].as_array().unwrap() {
        let title = line["title"].as_str().unwrap();
        // A person's name on a line means an assignment sentence became a
        // line item — the EST-0005 defect.
        for name in ["Juan", "Christos"] {
            assert!(
                !title.contains(name),
                "an assignment sentence became a line item: {title:?}"
            );
        }
        assert!(!title.contains(" to "), "a line reads as a sentence: {title:?}");
        // "$0" on a client's estimate states a price nobody committed to.
        assert_ne!(line["amount_cents"], serde_json::json!(0), "a zero-dollar line: {title:?}");
    }
}

#[tokio::test]
#[ignore = "hits the real API; set ANTHROPIC_API_KEY and run with --ignored"]
async fn real_walk_becomes_a_work_order() {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .expect("set ANTHROPIC_API_KEY to run the real-provider document smoke test");
    let provider = Arc::new(AnthropicProvider::from_env(api_key, model()));

    let store = Store::open_in_memory("smoke-device").unwrap();
    let session = store.start_session_with_template(None, "landscape").unwrap();
    store.append_transcript(&session.id, TRANSCRIPT).unwrap();
    store.end_and_record_session(&session.id).unwrap();
    let store = Arc::new(Mutex::new(store));

    let processor = SessionProcessor::new(
        provider.clone(),
        store.clone(),
        Arc::new(Mutex::new(Memory::default())),
        Arc::new(NullMemoryStore),
    );
    processor.process(&session.id).await.expect("processing failed");

    let builder = DocumentBuilder::new(
        provider,
        store.clone(),
        Arc::new(Mutex::new(Memory::default())),
        Arc::new(NullMemoryStore),
    );
    let outcome = builder.build(&session.id, "work_order").await.expect("build failed");

    let guard = store.lock().unwrap();
    let artifact = guard.get_artifact(&outcome.document_artifact_id).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&artifact.body).unwrap();

    // Print it — the half that matters here is whether it READS right.
    println!("\n===== WORK ORDER =====");
    for field in doc["fields"].as_array().unwrap() {
        println!(
            "[{}] {}: {}",
            field["section_key"].as_str().unwrap_or(""),
            field["label"].as_str().unwrap_or(""),
            field["value"].as_str().unwrap_or("—— (gap)")
        );
    }
    for line in doc["lines"].as_array().unwrap() {
        println!(
            "\n  {}\n    {}\n    assignee: {}  amount: {}",
            line["title"].as_str().unwrap_or(""),
            line["detail"].as_str().unwrap_or(""),
            line["assignee"].as_str().unwrap_or("—"),
            line["amount_cents"]
        );
    }
    println!("======================\n");

    assert!(!outcome.queued, "the compose call did not complete");

    // Structure: the fields the schema authored, in order, all present.
    let keys: Vec<&str> =
        doc["fields"].as_array().unwrap().iter().map(|f| f["key"].as_str().unwrap()).collect();
    assert_eq!(keys, vec!["crew", "schedule", "access", "safety"]);

    // A work order NEVER carries money, whatever the model returns.
    for line in doc["lines"].as_array().unwrap() {
        assert!(line["amount_cents"].is_null(), "money on a work order: {line}");
    }

    // Line count and titles are the deterministic render's, not the model's:
    // the compose pass may only write onto lines, never author them.
    let items = guard.list_items_for_session(&session.id).unwrap();
    assert_eq!(
        doc["lines"].as_array().unwrap().len(),
        items.len(),
        "the compose pass added or dropped a line"
    );
    for (line, item) in doc["lines"].as_array().unwrap().iter().zip(&items) {
        assert_eq!(line["title"].as_str().unwrap(), item.text, "a line was rewritten");
        assert_eq!(line["item_id"].as_str().unwrap(), item.id, "a line lost its item");
    }
}
