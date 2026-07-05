//! Runner plumbing shared by hermetic and real-API modes. Runs one scenario
//! through the *real* pipeline types (`SessionProcessor`, optionally
//! `LiveExtractor`) against whatever `LlmProvider` is supplied, then reads the
//! store back into an `Observed` and prices the R9 usage rows.

use std::sync::{Arc, Mutex};

use harness::{ContentBlock, HarnessError, LlmProvider, Memory, MemoryStore, MockProvider};
use murmur_core::{LiveExtractOutcome, LiveExtractor, SessionProcessor, Store};

use crate::corpus::Scenario;
use crate::grade::{grade, Observed, ObservedContact, ObservedItem, ScenarioScore};
use crate::report::{CostReport, ScenarioReport};

/// Memory store stub — evals don't persist memory.
pub struct NullMemoryStore;
impl MemoryStore for NullMemoryStore {
    fn load(&self) -> Result<Memory, HarnessError> {
        Ok(Memory::default())
    }
    fn save(&self, _m: &Memory) -> Result<(), HarnessError> {
        Ok(())
    }
}

/// Runs one scenario end-to-end through `process()` with `provider`, then reads
/// the store into an `Observed` and grades it. `model` is only used to price the
/// usage rows. Returns the per-scenario report.
pub async fn run_scenario(
    scenario: &Scenario,
    provider: Arc<dyn LlmProvider>,
    model: &str,
) -> Result<ScenarioReport, murmur_core::CoreError> {
    let store = Store::open_in_memory("eval-device")?;
    let session = store.start_session(None)?;
    if !scenario.transcript.trim().is_empty() {
        store.append_transcript(&session.id, &scenario.transcript)?;
    }
    store.end_and_record_session(&session.id)?;
    let sid = session.id.clone();
    let store = Arc::new(Mutex::new(store));

    let processor = SessionProcessor::new(
        provider,
        store.clone(),
        Arc::new(Mutex::new(Memory::default())),
        Arc::new(NullMemoryStore),
    );
    // Processing failure is itself an observable outcome (empty board); don't
    // abort the whole suite. Grade whatever landed.
    let _ = processor.process(&sid).await;

    let (observed, cost) = observe(&store, &sid, model)?;
    let score: ScenarioScore = grade(&scenario.truth, &observed);
    Ok(ScenarioReport { id: scenario.id.clone(), score, cost })
}

/// Drives the LIVE path (`LiveExtractor`) over a scenario, hermetically, with a
/// deterministic `MockProvider` (Plan 09 D7). The session stays `Recording` (the
/// live path only extracts while recording), the transcript is appended once,
/// and `maybe_extract` is looped to catch up to the transcript end (mirroring
/// `carried_scenarios.rs`). Returns the graded live board PLUS the ASSEMBLED
/// live prompt — the first turn's user text, the true regression signal a
/// prompt edit moves (D7). `MockProvider` is concrete (not `dyn`) so we can read
/// `requests()`.
pub async fn run_live_scenario(
    scenario: &Scenario,
    provider: Arc<MockProvider>,
    model: &str,
) -> Result<(ScenarioScore, String), murmur_core::CoreError> {
    let store = Store::open_in_memory("eval-live-device")?;
    let session = store.start_session(None)?;
    if !scenario.transcript.trim().is_empty() {
        store.append_transcript(&session.id, &scenario.transcript)?;
    }
    let sid = session.id.clone();
    let total_chars = scenario.transcript.chars().count();
    let store = Arc::new(Mutex::new(store));

    let mut live = LiveExtractor::new(
        provider.clone(),
        store.clone(),
        Arc::new(Mutex::new(Memory::default())),
        &sid,
    );
    live.min_new_chars = 1; // deterministic: extract even short corpus scenarios
    // Catch up: loop until the cursor reaches the transcript end. Bounded so a
    // non-advancing pass (Skipped) can't spin.
    let mut passes = 0;
    while live.cursor() < total_chars && passes < 16 {
        if matches!(live.maybe_extract().await?, LiveExtractOutcome::Skipped) {
            break;
        }
        passes += 1;
    }

    let (observed, _cost) = observe(&store, &sid, model)?;
    let score = grade(&scenario.truth, &observed);

    // The honest regression gate (D7): the assembled live prompt the extractor
    // sent — first request, first user text block (same access as carried_scenarios).
    let reqs = provider.requests();
    let assembled_prompt = match reqs
        .first()
        .and_then(|r| r.messages.first())
        .and_then(|m| m.content.first())
    {
        Some(ContentBlock::Text { text }) => text.clone(),
        _ => String::new(),
    };
    Ok((score, assembled_prompt))
}

/// Reads the store into a grader `Observed` plus a priced `CostReport`.
pub fn observe(
    store: &Arc<Mutex<Store>>,
    session_id: &str,
    model: &str,
) -> Result<(Observed, CostReport), murmur_core::CoreError> {
    let guard = store
        .lock()
        .map_err(|_| murmur_core::CoreError::InvalidState("store lock poisoned".into()))?;
    let items = guard
        .list_items_for_session(session_id)?
        .into_iter()
        .map(|i| ObservedItem { kind: i.kind, text: i.text })
        .collect();
    let contacts = guard
        .list_contacts()?
        .into_iter()
        .map(|c| ObservedContact { name: c.name, trade: c.trade })
        .collect();
    let summary_present = guard
        .get_session(session_id)?
        .summary
        .map(|s| !s.trim().is_empty() && s != "(empty session)")
        .unwrap_or(false);
    let (input_tokens, output_tokens) = guard
        .list_llm_usage_for_session(session_id)?
        .iter()
        .fold((0u64, 0u64), |(i, o), r| (i + r.input_tokens, o + r.output_tokens));
    let cost = CostReport::estimate(model, input_tokens, output_tokens);
    Ok((Observed { items, contacts, summary_present }, cost))
}
