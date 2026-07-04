//! Env-gated smoke test against the real Anthropic API. Ignored by default —
//! it costs real tokens and needs a key. Run explicitly with:
//!
//! ```sh
//! ANTHROPIC_API_KEY=sk-... nix shell nixpkgs#cargo nixpkgs#rustc -c \
//!     cargo test -p murmur-core --test anthropic_smoke -- --ignored
//! ```
//!
//! Wire-level integration only: asserts the pipeline round-trips (Processed
//! status, non-empty summary, nonzero logged usage) — not extraction quality.

use std::sync::{Arc, Mutex};

use harness::{AnthropicProvider, HarnessError, Memory, MemoryStore};
use murmur_core::{SessionProcessor, SessionStatus, Store};

/// Cheapest current haiku-class model — this is a smoke test, not an eval.
const MODEL: &str = "claude-haiku-4-5";

const TRANSCRIPT: &str = "Walked the back deck with Dev the framer today. \
Joists on the north side are soft, probably rot — need to order two sixteen-foot \
pressure-treated two-by-tens and get them swapped before the railing goes on. \
Dev says call him Thursday to schedule. Also remind me to invoice the Hendersons \
for the fence job.";

/// The smoke test doesn't need persistent memory.
struct NullMemoryStore;
impl MemoryStore for NullMemoryStore {
    fn load(&self) -> Result<Memory, HarnessError> {
        Ok(Memory::default())
    }
    fn save(&self, _m: &Memory) -> Result<(), HarnessError> {
        Ok(())
    }
}

#[tokio::test]
#[ignore = "hits the real Anthropic API; set ANTHROPIC_API_KEY and run with --ignored"]
async fn real_anthropic_site_walk() {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .expect("set ANTHROPIC_API_KEY to run the real-provider smoke test");

    let store = Store::open_in_memory("smoke-device").unwrap();
    let session = store.start_session(None).unwrap();
    store.append_transcript(&session.id, TRANSCRIPT).unwrap();
    store.end_and_record_session(&session.id).unwrap();
    let store = Arc::new(Mutex::new(store));

    let processor = SessionProcessor::new(
        Arc::new(AnthropicProvider::new(api_key, MODEL)),
        store.clone(),
        Arc::new(Mutex::new(Memory::default())),
        Arc::new(NullMemoryStore),
    );

    let outcome = processor.process(&session.id).await.expect("processing failed");

    assert_eq!(outcome.session.status, SessionStatus::Processed);
    let summary = outcome.session.summary.expect("summary missing");
    assert!(!summary.trim().is_empty(), "summary is empty");

    let store = store.lock().unwrap();
    let usage_rows = store.list_llm_usage_for_session(&session.id).unwrap();
    assert_eq!(usage_rows.len(), 1, "expected one processing usage row");
    assert!(usage_rows[0].input_tokens > 0, "input tokens not recorded");
    assert!(usage_rows[0].output_tokens > 0, "output tokens not recorded");
}
