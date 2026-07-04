//! Runner plumbing shared by hermetic and real-API modes. Runs one scenario
//! through the *real* pipeline types (`SessionProcessor`, optionally
//! `LiveExtractor`) against whatever `LlmProvider` is supplied, then reads the
//! store back into an `Observed` and prices the R9 usage rows.

use std::sync::{Arc, Mutex};

use harness::{HarnessError, LlmProvider, Memory, MemoryStore};
use murmur_core::{SessionProcessor, Store};

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
