//! On-demand document builder (Plan 13 D2/D4/D5/D5a/D6/D7/D8/D9): renders the
//! document structure deterministically from the session's authoritative
//! items (no LLM), then for pricing kinds runs one focused items-only
//! pricing pass (R6). A document always lands — pricing failure degrades to
//! an unpriced structure-only document, never a hard failure (R7).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use harness::{
    CompletionRequest, ContentBlock, HarnessError, LlmProvider, Memory, MemoryStore, Message,
    ToolSpec, Usage,
};

use crate::domain::{Artifact, CapturedItem, SessionStatus};
use crate::error::CoreError;
use crate::pipeline::{doc_kinds_for_template, is_pricing_kind};
use crate::store::Store;

/// C1: whether a rendered line defaults to `is_gap: true`. `PerPricingKind`
/// is the on-demand build's normal policy (D4); `AllGap` is reserved for a
/// degraded/offline render where nothing has been through the LLM at all —
/// "amount not yet priced" != "nothing has been through the LLM" (a naive
/// `is_pricing_kind` delegation would wrongly flip a degraded non-pricing
/// document to looks-confirmed).
///
/// `AllGap` is not constructed by any Stage-1 production caller —
/// `DocumentBuilder::build` (the only caller) always renders with
/// `PerPricingKind`; the offline/degraded fallback stays on its own
/// `ffi::convert::partial_document_from_items` implementation by design (see
/// `render_structure_document`'s doc comment). It exists as the documented
/// N2 parity contract, pinned by the cross-check tests below — hence the
/// explicit `allow` rather than deleting a variant this plan defines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GapPolicy {
    PerPricingKind,
    #[allow(dead_code)]
    AllGap,
}

/// Deterministic, no-LLM structure render (D4): one line per item, in item
/// order, using a FRESH `new_id()` per line (`line.id != item_id`, the
/// `build_document`-tool convention). Field shape matches
/// `BuildDocumentTool`'s emitted lines exactly.
///
/// **Parity contract with the FFI offline fallback** (N2 —
/// `crates/ffi/src/convert.rs::partial_document_from_items`): calling this
/// with `GapPolicy::AllGap` produces lines with IDENTICAL
/// title/detail/qty/amount_cents/section/item_id/is_gap semantics — title =
/// item.text, detail = "", qty = "", amount_cents = None, section = None,
/// is_gap = true, item_id = Some(item.id) — waiving only the `id` field (the
/// offline fallback legacy-sets `line.id = item.id`; this render uses a
/// fresh id). The two implementations are kept in lockstep by this
/// documented contract (pinned by the cross-check test below), not by
/// sharing code across the crate boundary — `ffi` depends on `murmur-core`,
/// never the reverse, so this function can't call (or be called by) it.
pub(crate) fn render_structure_document(
    doc_kind: &str,
    items: &[CapturedItem],
    gap: GapPolicy,
) -> Vec<serde_json::Value> {
    items
        .iter()
        .map(|item| {
            let is_gap = match gap {
                GapPolicy::AllGap => true,
                GapPolicy::PerPricingKind => is_pricing_kind(doc_kind),
            };
            serde_json::json!({
                "id": crate::ids::new_id(),
                "title": item.text,
                "detail": "",
                "qty": "",
                "amount_cents": null,
                "section": null,
                "is_gap": is_gap,
                "item_id": item.id,
            })
        })
        .collect()
}

const PRICE_ITEMS: &str = "price_items";

fn price_items_tool_spec() -> ToolSpec {
    ToolSpec {
        name: PRICE_ITEMS.into(),
        description: "Attach a price to each item that should carry one. You may price only \
                       items from the given list, by their exact item_id — you cannot add, \
                       rename, or drop items."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "prices": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "item_id": { "type": "string" },
                            "amount_cents": { "type": "integer" }
                        },
                        "required": ["item_id", "amount_cents"]
                    }
                },
                "total_cents": { "type": "integer" }
            },
            "required": ["prices"]
        }),
    }
}

fn format_pricing_items(items: &[CapturedItem]) -> String {
    items
        .iter()
        .map(|i| format!("- [{}] {} (item_id: {})", i.kind, i.text, i.id))
        .collect::<Vec<_>>()
        .join("\n")
}

/// D5/D5a: one focused pricing pass whose input is the items only (never the
/// transcript, R6) plus an optional single scalar hint (the operator's
/// spoken grand total, captured at summary time). The forced tool's output
/// schema can only attach an amount to an `item_id` already in `items` — it
/// has no line-authoring power: the line count is fixed by the deterministic
/// structure render, only amounts can move. Echo-and-validate + first-wins
/// dedup mirror Plan 12's `item_id` pattern.
///
/// Usage is accumulated into `usage` as soon as a response arrives (R9: a
/// response that fails to parse into a valid tool call still cost tokens) —
/// BEFORE this function's own success/failure is decided, matching
/// `run_build_document`/`summarize`'s pattern elsewhere in this pipeline.
/// (This is a small, deliberate signature addition vs the plan's literal
/// `-> Result<HashMap<...>, HarnessError>` — an out-param is needed so a
/// degrade path that reached the API but got an unparseable response still
/// logs its cost; a `provider.complete` `Err` itself carries no usage, so the
/// zero-token case is unaffected.)
pub(crate) async fn price_items(
    provider: &Arc<dyn LlmProvider>,
    items: &[CapturedItem],
    spoken_total_cents: Option<i64>,
    memory_prompt: &str,
    max_tokens: u32,
    usage: &mut Usage,
) -> Result<HashMap<String, i64>, HarnessError> {
    let memory_block = if memory_prompt.trim().is_empty() {
        String::new()
    } else {
        format!("\n\nWhat you know about this user:\n{memory_prompt}")
    };
    let system = format!(
        "You price items from a field-work session for a tradesperson's estimate/invoice. \
         Put a price only on an item whose value you can reasonably infer from its text and \
         what you know about this user's pricing history — never guess wildly. You may price \
         only items from the given list, by their exact item_id.{memory_block}"
    );
    let hint_block = spoken_total_cents
        .map(|cents| {
            format!(
                "\n\nOperator's stated target total: ${:.2} — allocate line prices consistent with this.",
                cents as f64 / 100.0
            )
        })
        .unwrap_or_default();
    let items_block = format_pricing_items(items);
    let user_message = format!("Price these items.\n\n{items_block}{hint_block}");

    let response = provider
        .complete(CompletionRequest {
            system,
            messages: vec![Message::user_text(user_message)],
            tools: vec![price_items_tool_spec()],
            max_tokens,
            tool_choice: Some(PRICE_ITEMS.to_string()),
        })
        .await?;
    usage.add(&response.usage);

    let input = response.content.iter().find_map(|b| match b {
        ContentBlock::ToolUse { name, input, .. } if name == PRICE_ITEMS => Some(input.clone()),
        _ => None,
    });
    let input = input.ok_or_else(|| {
        HarnessError::Provider("price_items response missing price_items call".into())
    })?;

    let valid_ids: HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
    let mut map: HashMap<String, i64> = HashMap::new();
    if let Some(prices) = input.get("prices").and_then(|v| v.as_array()) {
        for p in prices {
            let (Some(id), Some(amount)) = (
                p.get("item_id").and_then(|v| v.as_str()),
                p.get("amount_cents").and_then(|v| v.as_i64()),
            ) else {
                continue;
            };
            // First-wins dedup; unknown/hallucinated ids are dropped — never
            // fail the whole pass over one bad row.
            if valid_ids.contains(id) && !map.contains_key(id) {
                map.insert(id.to_string(), amount);
            }
        }
    }
    Ok(map)
}

/// Applies validated prices onto rendered lines: a line whose `item_id` is a
/// key in `map` gets `amount_cents` set and flips `is_gap: false`. `map` is
/// already deduped by `price_items`; `claimed` is belt-and-suspenders against
/// a future caller passing a map with a colliding line target.
fn apply_prices(map: &HashMap<String, i64>, lines: &mut [serde_json::Value]) {
    let mut claimed: HashSet<String> = HashSet::new();
    for line in lines.iter_mut() {
        let Some(item_id) = line.get("item_id").and_then(|v| v.as_str()).map(str::to_string)
        else {
            continue;
        };
        if claimed.contains(&item_id) {
            continue;
        }
        if let Some(&amount) = map.get(&item_id) {
            line["amount_cents"] = serde_json::json!(amount);
            line["is_gap"] = serde_json::json!(false);
            claimed.insert(item_id);
        }
    }
}

/// Total-shape per `doc_kind` (mirrors `ffi::convert::partial_document_from_items`):
/// an inspection has no summable dollar total; everything else sums its lines.
fn total_shape(doc_kind: &str) -> (&'static str, &'static str) {
    match doc_kind {
        "inspection" => ("static", "findings"),
        _ => ("sum", "total"),
    }
}

/// The result of one `DocumentBuilder::build` call (D7: burn-per-tap — every
/// call mints a fresh document number and writes a new snapshot artifact).
#[derive(Debug)]
pub struct BuildDocumentOutcome {
    pub document_artifact_id: String,
    pub usage: Usage,
    /// D5/C: true when the pricing pass could not run to completion (a
    /// pricing-kind build whose LLM call failed) — reused posture of the
    /// offline `queued` flag ("document incomplete — pricing did not run").
    /// `false` for a non-pricing kind or a fully-priced/attempted document.
    pub queued: bool,
}

/// On-demand document builder (D1/D8/D9), engine-keyed by the caller (FFI
/// `MurmurEngine::build_document`, not `WalkSession`-scoped — the walk may
/// already be over and its `WalkSession` handle dropped).
pub struct DocumentBuilder {
    provider: Arc<dyn LlmProvider>,
    store: Arc<Mutex<Store>>,
    memory: Arc<Mutex<Memory>>,
    /// Reserved: a future price-book seam (D6) may consult saved facts
    /// directly rather than only the rendered `to_prompt()` text.
    #[allow(dead_code)]
    memory_store: Arc<dyn MemoryStore>,
    /// Pricing-call output budget.
    pub max_tokens: u32,
}

impl DocumentBuilder {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        store: Arc<Mutex<Store>>,
        memory: Arc<Mutex<Memory>>,
        memory_store: Arc<dyn MemoryStore>,
    ) -> Self {
        DocumentBuilder { provider, store, memory, memory_store, max_tokens: 1024 }
    }

    fn locked(&self) -> Result<std::sync::MutexGuard<'_, Store>, CoreError> {
        self.store
            .lock()
            .map_err(|_| CoreError::InvalidState("store lock poisoned".into()))
    }

    /// D8 validation, D4 structure render, D5/D5a pricing pass, D7 mint +
    /// persist. Never a hard failure on a pricing LLM error (R7) — degrades
    /// to `queued: true` with an unpriced structure-only document instead.
    pub async fn build(
        &self,
        session_id: &str,
        doc_kind: &str,
    ) -> Result<BuildDocumentOutcome, CoreError> {
        let (session, items) = {
            let store = self.locked()?;
            let session = store.get_session(session_id)?;
            if session.status != SessionStatus::Processed {
                return Err(CoreError::InvalidState(format!(
                    "cannot build a document for a {} session",
                    session.status.as_str()
                )));
            }
            let legal = doc_kinds_for_template(session.template.as_deref());
            if !legal.contains(&doc_kind) {
                return Err(CoreError::InvalidState(format!(
                    "'{doc_kind}' is not a legal document kind for template {:?}",
                    session.template
                )));
            }
            let items = store.list_items_for_session(session_id)?;
            (session, items)
        };

        let mut lines = render_structure_document(doc_kind, &items, GapPolicy::PerPricingKind);
        let mut usage = Usage::default();
        let mut queued = false;

        if is_pricing_kind(doc_kind) && !items.is_empty() {
            let hint = self.session_spoken_total(session_id)?;
            let memory_prompt = self
                .memory
                .lock()
                .map_err(|_| CoreError::InvalidState("memory lock poisoned".into()))?
                .to_prompt();
            match price_items(
                &self.provider,
                &items,
                hint,
                &memory_prompt,
                self.max_tokens,
                &mut usage,
            )
            .await
            {
                Ok(map) => apply_prices(&map, &mut lines),
                // R7: never a hard failure — the structure-only document
                // still lands, just unpriced and flagged queued (D5 degrade).
                Err(_) => queued = true,
            }
        }

        let (total_kind, total_label_key) = total_shape(doc_kind);
        let payload = serde_json::json!({
            "doc_kind": doc_kind,
            "job_date_unix": session.started_at,
            "total_kind": total_kind,
            "total_label_key": total_label_key,
            "static_total_cents": serde_json::Value::Null,
            "lines": lines,
            "queued": queued,
        });

        // D7: always mint a fresh number and write a new snapshot artifact —
        // burn per tap, never reuse (regenerate leaves prior snapshots intact).
        let artifact = self
            .locked()?
            .mint_document_number_and_add_artifact(session_id, doc_kind, None, payload)?;

        // D9: log a "document"-purpose usage row only if a call was actually
        // made (non-pricing kinds and the empty-items skip make zero calls).
        if usage != Usage::default() {
            self.locked()?.record_llm_usage(Some(session_id), "document", &usage)?;
        }

        Ok(BuildDocumentOutcome { document_artifact_id: artifact.id, usage, queued })
    }

    /// D5a: reads the `session_meta` artifact (if any) written by `process()`
    /// on success and returns its `spoken_total_cents` scalar. `None` when no
    /// meta artifact exists, or it exists but the field is absent (no total
    /// was clearly stated, R6).
    fn session_spoken_total(&self, session_id: &str) -> Result<Option<i64>, CoreError> {
        let artifacts = self.locked()?.list_artifacts_for_session(session_id)?;
        let meta: Option<&Artifact> = artifacts.iter().rev().find(|a| a.kind == "session_meta");
        Ok(meta
            .and_then(|a| serde_json::from_str::<serde_json::Value>(&a.body).ok())
            .and_then(|v| v.get("spoken_total_cents").and_then(|n| n.as_i64())))
    }
}

#[cfg(test)]
mod tests {
    use harness::{ContentBlock, MockProvider, StopReason};

    use crate::domain::ItemSource;
    use crate::store::Store;

    use super::*;

    struct NullMemoryStore;
    impl MemoryStore for NullMemoryStore {
        fn load(&self) -> Result<Memory, HarnessError> {
            Ok(Memory::default())
        }
        fn save(&self, _m: &Memory) -> Result<(), HarnessError> {
            Ok(())
        }
    }

    fn tool_use(name: &str, input: serde_json::Value) -> harness::CompletionResponse {
        harness::CompletionResponse {
            content: vec![ContentBlock::ToolUse { id: "tu_1".into(), name: name.into(), input }],
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 80, output_tokens: 15 },
        }
    }

    fn items(store: &Store, sid: &str, texts: &[(&str, &str)]) -> Vec<CapturedItem> {
        texts
            .iter()
            .map(|(kind, text)| {
                store.add_item_with_source(sid, kind, text, ItemSource::Authoritative).unwrap()
            })
            .collect()
    }

    // ---- Task 2: render_structure_document / GapPolicy -----------------

    #[test]
    fn per_pricing_kind_flags_gaps_only_for_pricing_kinds() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        let its = items(&store, &session.id, &[("todo", "order lumber"), ("safety", "loose railing")]);

        let est_lines = render_structure_document("estimate", &its, GapPolicy::PerPricingKind);
        assert_eq!(est_lines.len(), 2);
        for (line, item) in est_lines.iter().zip(&its) {
            assert_eq!(line["amount_cents"], serde_json::Value::Null);
            assert_eq!(line["is_gap"], true, "estimate lines are gaps until priced");
            assert_eq!(line["item_id"], item.id);
            assert_eq!(line["section"], serde_json::Value::Null);
            assert_eq!(line["title"], item.text);
            assert_ne!(line["id"], line["item_id"], "on-demand render uses a fresh new_id");
        }

        let insp_lines = render_structure_document("inspection", &its, GapPolicy::PerPricingKind);
        for line in &insp_lines {
            assert_eq!(line["is_gap"], false, "a normal finding is not a gap");
        }
    }

    #[test]
    fn all_gap_flags_every_line_regardless_of_kind() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        let its = items(&store, &session.id, &[("todo", "order lumber")]);

        let lines = render_structure_document("inspection", &its, GapPolicy::AllGap);
        assert_eq!(
            lines[0]["is_gap"],
            true,
            "a degraded inspection is wholly unconfirmed, not merely unpriced (C1)"
        );
    }

    /// N2 cross-check: `AllGap` output matches the documented field contract
    /// of `ffi::convert::partial_document_from_items` (title/detail/qty/
    /// amount_cents/section/item_id/is_gap), waiving only `id`.
    #[test]
    fn all_gap_matches_the_offline_fallback_contract_except_id() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        let its = items(&store, &session.id, &[("todo", "haul debris")]);

        let lines = render_structure_document("estimate", &its, GapPolicy::AllGap);
        let line = &lines[0];
        let item = &its[0];
        assert_eq!(line["title"], item.text);
        assert_eq!(line["detail"], "");
        assert_eq!(line["qty"], "");
        assert_eq!(line["amount_cents"], serde_json::Value::Null);
        assert_eq!(line["section"], serde_json::Value::Null);
        assert_eq!(line["is_gap"], true);
        assert_eq!(line["item_id"], item.id);
        // id is deliberately NOT compared — the offline fallback sets
        // line.id = item.id; this render always mints a fresh id.
    }

    // ---- Task 3: price_items echo-validate ------------------------------

    #[tokio::test]
    async fn price_items_echoes_and_validates_first_wins() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        let its = items(&store, &session.id, &[("todo", "mulch"), ("todo", "edging")]);
        let a1 = its[0].id.clone();
        let a2 = its[1].id.clone();

        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![tool_use(
            "price_items",
            serde_json::json!({
                "prices": [
                    {"item_id": a1, "amount_cents": 28500},
                    {"item_id": "bogus", "amount_cents": 999},
                    {"item_id": a2, "amount_cents": 31000},
                    {"item_id": a2, "amount_cents": 99999}
                ]
            }),
        )]));

        let mut usage = Usage::default();
        let map = price_items(&provider, &its, None, "", 512, &mut usage).await.unwrap();
        assert_eq!(map.get(a1.as_str()), Some(&28500));
        assert_eq!(map.get(a2.as_str()), Some(&31000), "first-wins on the duplicate a2");
        assert_eq!(map.get("bogus"), None, "hallucinated id dropped");
        assert_eq!(map.len(), 2);
        assert_eq!(usage, Usage { input_tokens: 80, output_tokens: 15 });
    }

    #[tokio::test]
    async fn price_items_fed_the_spoken_total_hint_never_the_transcript() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        let its = items(&store, &session.id, &[("todo", "mulch")]);
        let a1 = its[0].id.clone();

        let provider = Arc::new(MockProvider::new(vec![tool_use(
            "price_items",
            serde_json::json!({"prices": [{"item_id": a1, "amount_cents": 120000}]}),
        )]));
        let dyn_provider: Arc<dyn LlmProvider> = provider.clone();
        let mut usage = Usage::default();
        price_items(&dyn_provider, &its, Some(120000), "", 512, &mut usage).await.unwrap();

        let reqs = provider.requests();
        let ContentBlock::Text { text } = &reqs[0].messages[0].content[0] else {
            panic!("expected text content");
        };
        assert!(text.contains("1200.00"), "the scalar hint reaches the prompt: {text}");
        assert!(!text.to_lowercase().contains("transcript"), "never the transcript: {text}");
    }

    // ---- Task 3: DocumentBuilder::build ---------------------------------

    fn processed_session_with_items(
        texts: &[(&str, &str)],
    ) -> (Store, String) {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session_with_template(None, "landscape").unwrap();
        let mut run_item_ids = Vec::new();
        for (kind, text) in texts {
            let item =
                store.add_item_with_source(&session.id, kind, text, ItemSource::Authoritative).unwrap();
            run_item_ids.push(item.id);
        }
        store.append_transcript(&session.id, "site walk").unwrap();
        store.end_and_record_session(&session.id).unwrap();
        store
            .finish_session_processed(
                &session.id,
                "Walked the site.",
                &Usage::default(),
                &run_item_ids,
            )
            .unwrap();
        (store, session.id)
    }

    fn builder(
        store: Arc<Mutex<Store>>,
        provider: Arc<dyn LlmProvider>,
    ) -> DocumentBuilder {
        DocumentBuilder::new(provider, store, Arc::new(Mutex::new(Memory::default())), Arc::new(NullMemoryStore))
    }

    #[tokio::test]
    async fn build_happy_path_prices_and_mints_a_document() {
        let (store, sid) = processed_session_with_items(&[("todo", "mulch"), ("safety", "loose railing")]);
        // Re-read the real minted ids so the scripted response can echo one.
        let a1 = store.list_items_for_session(&sid).unwrap()[0].id.clone();
        let store = Arc::new(Mutex::new(store));

        let provider = Arc::new(MockProvider::new(vec![tool_use(
            "price_items",
            serde_json::json!({"prices": [{"item_id": a1, "amount_cents": 28500}]}),
        )]));
        let b = builder(store.clone(), provider);

        let outcome = b.build(&sid, "estimate").await.unwrap();
        assert!(!outcome.queued);
        assert_eq!(outcome.usage, Usage { input_tokens: 80, output_tokens: 15 });

        let store = store.lock().unwrap();
        let art = store.get_artifact(&outcome.document_artifact_id).unwrap();
        let v: serde_json::Value = serde_json::from_str(&art.body).unwrap();
        assert_eq!(v["doc_number"], 1);
        let lines = v["lines"].as_array().unwrap();
        let priced = lines.iter().find(|l| l["item_id"] == a1).unwrap();
        assert_eq!(priced["amount_cents"], 28500);
        assert_eq!(priced["is_gap"], false);
        let gap = lines.iter().find(|l| l["item_id"] != a1).unwrap();
        assert_eq!(gap["amount_cents"], serde_json::Value::Null);
        assert_eq!(gap["is_gap"], true);

        let usage_rows = store.list_llm_usage_for_session(&sid).unwrap();
        let document_rows: Vec<_> = usage_rows.iter().filter(|r| r.purpose == "document").collect();
        assert_eq!(
            document_rows.len(),
            1,
            "exactly one 'document'-purpose row (the fixture's finish_session_processed \
             already logs its own 'processing' row separately)"
        );
    }

    #[tokio::test]
    async fn build_degrades_to_unpriced_and_queued_on_llm_failure() {
        let (store, sid) = processed_session_with_items(&[("todo", "mulch")]);
        let store = Arc::new(Mutex::new(store));
        // Empty response queue -> provider errors on the pricing call.
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![]));
        let b = builder(store.clone(), provider);

        let outcome = b.build(&sid, "estimate").await.unwrap();
        assert!(outcome.queued, "pricing LLM failure degrades, never a hard failure (R7)");

        let store = store.lock().unwrap();
        let art = store.get_artifact(&outcome.document_artifact_id).unwrap();
        let v: serde_json::Value = serde_json::from_str(&art.body).unwrap();
        assert_eq!(v["doc_number"], 1, "the document still mints and lands");
        assert_eq!(v["queued"], true);
        for line in v["lines"].as_array().unwrap() {
            assert_eq!(line["amount_cents"], serde_json::Value::Null);
        }
    }

    #[tokio::test]
    async fn build_non_pricing_kind_makes_zero_calls() {
        let (store, sid) = processed_session_with_items(&[("todo", "mulch")]);
        let store = Arc::new(Mutex::new(store));
        let provider = Arc::new(MockProvider::new(vec![]));
        let b = builder(store.clone(), provider.clone());

        let outcome = b.build(&sid, "work_order").await.unwrap();
        assert!(!outcome.queued);
        assert_eq!(outcome.usage, Usage::default());
        assert!(provider.requests().is_empty(), "non-pricing kinds never call the LLM");

        let store = store.lock().unwrap();
        assert!(
            store
                .list_llm_usage_for_session(&sid)
                .unwrap()
                .iter()
                .all(|r| r.purpose != "document"),
            "non-pricing kinds log no 'document'-purpose usage row"
        );
    }

    #[tokio::test]
    async fn build_rejects_non_processed_sessions_and_illegal_kinds() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session_with_template(None, "landscape").unwrap();
        let store = Arc::new(Mutex::new(store));
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![]));
        let b = builder(store.clone(), provider);

        // Not Processed yet (still Recording).
        assert!(matches!(b.build(&session.id, "estimate").await, Err(CoreError::InvalidState(_))));

        let (store2, sid2) = processed_session_with_items(&[]);
        let store2 = Arc::new(Mutex::new(store2));
        let provider2: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![]));
        let b2 = builder(store2, provider2);
        // "inspection" is not legal for the landscape template.
        assert!(matches!(b2.build(&sid2, "inspection").await, Err(CoreError::InvalidState(_))));
    }

    #[tokio::test]
    async fn build_empty_but_processed_session_yields_a_truthful_zero_line_document() {
        let (store, sid) = processed_session_with_items(&[]);
        let store = Arc::new(Mutex::new(store));
        let provider = Arc::new(MockProvider::new(vec![]));
        let b = builder(store.clone(), provider.clone());

        let outcome = b.build(&sid, "estimate").await.unwrap();
        assert!(!outcome.queued);
        assert!(provider.requests().is_empty(), "nothing to price for zero items — skip the call");

        let store = store.lock().unwrap();
        let art = store.get_artifact(&outcome.document_artifact_id).unwrap();
        let v: serde_json::Value = serde_json::from_str(&art.body).unwrap();
        assert_eq!(v["doc_number"], 1);
        assert!(v["lines"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn build_threads_the_spoken_total_hint_and_absence_when_no_meta_artifact() {
        let (store, sid) = processed_session_with_items(&[("todo", "mulch"), ("safety", "loose railing")]);
        let a1 = store.list_items_for_session(&sid).unwrap()[0].id.clone();
        store
            .add_artifact(&sid, "session_meta", "meta", &serde_json::json!({"spoken_total_cents": 120000}).to_string())
            .unwrap();
        let store = Arc::new(Mutex::new(store));

        let provider = Arc::new(MockProvider::new(vec![tool_use(
            "price_items",
            serde_json::json!({"prices": [{"item_id": a1, "amount_cents": 95000}]}),
        )]));
        let b = builder(store.clone(), provider.clone());
        b.build(&sid, "estimate").await.unwrap();

        let reqs = provider.requests();
        let ContentBlock::Text { text } = &reqs[0].messages[0].content[0] else {
            panic!("expected text content");
        };
        assert!(text.contains("1200.00"), "the spoken total hint reaches the pricing prompt: {text}");

        // A session with no session_meta artifact gets no hint line at all.
        let (store2, sid2) = processed_session_with_items(&[("todo", "haul")]);
        let a2 = store2.list_items_for_session(&sid2).unwrap()[0].id.clone();
        let store2 = Arc::new(Mutex::new(store2));
        let provider2 = Arc::new(MockProvider::new(vec![tool_use(
            "price_items",
            serde_json::json!({"prices": [{"item_id": a2, "amount_cents": 5000}]}),
        )]));
        let b2 = builder(store2, provider2.clone());
        b2.build(&sid2, "estimate").await.unwrap();
        let reqs2 = provider2.requests();
        let ContentBlock::Text { text: text2 } = &reqs2[0].messages[0].content[0] else {
            panic!("expected text content");
        };
        assert!(!text2.contains("stated target total"), "no hint line without a meta artifact: {text2}");
    }
}
