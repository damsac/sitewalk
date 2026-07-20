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

use crate::domain::{Artifact, CapturedItem, DocumentSchema, SchemaField, SessionStatus};
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
/// `PerPricingKind`; the offline/degraded fallback lived on its own
/// `ffi::convert::partial_document_from_items` implementation by design (see
/// `render_structure_document`'s doc comment) until notes-first left it
/// caller-less and it was removed. It exists as the documented
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
/// **Parity contract with the FFI offline fallback** (N2 — formerly
/// `crates/ffi/src/convert.rs::partial_document_from_items`, removed once
/// notes-first left it caller-less): calling this
/// with `GapPolicy::AllGap` produces lines with IDENTICAL
/// title/detail/qty/amount_cents/section/item_id/is_gap semantics — title =
/// item.text, detail = "", qty = item.right (which for the removed offline
/// fallback's un-`right` items was ""), amount_cents = None, section = None,
/// is_gap = true, item_id = Some(item.id) — waiving only the `id` field (the
/// offline fallback legacy-sets `line.id = item.id`; this render uses a
/// fresh id). The two implementations are kept in lockstep by this
/// documented contract (pinned by the cross-check test below), not by
/// sharing code across the crate boundary — `ffi` depends on `murmur-core`,
/// never the reverse, so this function can't call (or be called by) it.
// Since Plan 19 the production build path renders through `render_lines`
// with the resolved schema's `priced` flag; this kind-keyed wrapper stays as
// the pinned N2/GapPolicy parity surface (its tests below are part of the
// launch-safety Δ=0 net), hence the explicit allow rather than deleting it.
#[allow(dead_code)]
pub(crate) fn render_structure_document(
    doc_kind: &str,
    items: &[CapturedItem],
    gap: GapPolicy,
) -> Vec<serde_json::Value> {
    render_lines(items, gap, is_pricing_kind(doc_kind))
}

/// The schema-driven render (Plan 19 §4 step 3): identical line shape, with
/// `is_gap` driven by the resolved schema's `line_items.priced` instead of
/// `is_pricing_kind(doc_kind)` — for every built-in the two agree exactly
/// (pinned by `builtin_schemas_reproduce_todays_pricing_and_total_shape`).
pub(crate) fn render_lines(
    items: &[CapturedItem],
    gap: GapPolicy,
    priced: bool,
) -> Vec<serde_json::Value> {
    items
        .iter()
        .map(|item| {
            let is_gap = match gap {
                GapPolicy::AllGap => true,
                GapPolicy::PerPricingKind => priced,
            };
            serde_json::json!({
                "id": crate::ids::new_id(),
                "title": item.text,
                "detail": "",
                "qty": item.right,
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

const FILL_FIELDS: &str = "fill_fields";

fn fill_fields_tool_spec() -> ToolSpec {
    ToolSpec {
        name: FILL_FIELDS.into(),
        description: "Fill named document fields from the session. Put a value only on a field \
                       whose answer was clearly stated — omit any field you are unsure about. \
                       You may fill only fields from the given list, by their exact key."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "fields": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "key": { "type": "string" },
                            "value": { "type": "string" }
                        },
                        "required": ["key", "value"]
                    }
                }
            },
            "required": ["fields"]
        }),
    }
}

/// Plan 19 Stage 5: the one focused fill pass — `price_items`' exact twin,
/// including its degrade contract. Input is the items + session summary ONLY
/// (never the transcript, R6); the items block reuses `format_pricing_items`
/// (ONE item-formatting helper, no divergent shape). Echo-and-validate
/// against the offered field keys, first-wins dedup, drop unknown keys.
/// Usage is accumulated as soon as a response arrives (R9: an unparseable
/// response still cost tokens) — BEFORE success/failure is decided. A
/// provider `Err` carries no usage; a completed response whose tool block is
/// missing/unparseable is `Err(HarnessError::Provider(..))` after
/// `usage.add`, exactly like `price_items`. A tool block that IS present but
/// simply omits a field is NOT an error — that field is a truthful gap.
pub(crate) async fn fill_fields(
    provider: &Arc<dyn LlmProvider>,
    fields: &[SchemaField],
    items: &[CapturedItem],
    summary: &str,
    max_tokens: u32,
    usage: &mut Usage,
) -> Result<HashMap<String, String>, HarnessError> {
    let system = "You fill named fields of a field-work document for a tradesperson. Put a \
                  value only on a field whose answer was clearly stated in the session — never \
                  guess. You may fill only fields from the given list, by their exact key.";
    let fields_block = fields
        .iter()
        .map(|f| format!("- [{}] {}", f.key, f.label))
        .collect::<Vec<_>>()
        .join("\n");
    let items_block = format_pricing_items(items);
    // The exact WE-B user message (§6) — the items block is
    // `format_pricing_items` verbatim; only the Fields/summary framing is
    // the fill prompt's own.
    let user_message = format!(
        "Fill these document fields from the session. Put a value only on a field whose\n\
         answer was clearly stated — omit any field you are unsure about; a blank field\n\
         is cheaper than a wrong one.\n\
         \n\
         Fields:\n{fields_block}\n\
         \n\
         Session items:\n{items_block}\n\
         \n\
         Session summary:\n{summary}"
    );

    let response = provider
        .complete(CompletionRequest {
            system: system.to_string(),
            messages: vec![Message::user_text(user_message)],
            tools: vec![fill_fields_tool_spec()],
            max_tokens,
            tool_choice: Some(FILL_FIELDS.to_string()),
        })
        .await?;
    usage.add(&response.usage);

    let input = response.content.iter().find_map(|b| match b {
        ContentBlock::ToolUse { name, input, .. } if name == FILL_FIELDS => Some(input.clone()),
        _ => None,
    });
    let input = input.ok_or_else(|| {
        HarnessError::Provider("fill_fields response missing fill_fields call".into())
    })?;

    let valid_keys: HashSet<&str> = fields.iter().map(|f| f.key.as_str()).collect();
    let mut map: HashMap<String, String> = HashMap::new();
    if let Some(entries) = input.get("fields").and_then(|v| v.as_array()) {
        for e in entries {
            let (Some(key), Some(value)) =
                (e.get("key").and_then(|v| v.as_str()), e.get("value").and_then(|v| v.as_str()))
            else {
                continue;
            };
            // First-wins dedup; unknown/hallucinated keys are dropped — never
            // fail the whole pass over one bad row.
            if valid_keys.contains(key) && !map.contains_key(key) {
                map.insert(key.to_string(), value.to_string());
            }
        }
    }
    Ok(map)
}

/// Assembles the payload `fields[]` (Plan 19 Stage 5): one entry per
/// authored `filled`/`static`-section field, in schema order. `static` fill →
/// its authored value (is_gap false); `walk`/`manual` → the fill value if
/// present, else a truthful gap (`value: null, is_gap: true`). `manual`
/// fields are always gaps in v1 (operator completes at review — no LLM).
fn assemble_fields(
    schema: &DocumentSchema,
    values: &HashMap<String, String>,
) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for section in &schema.sections {
        if section.kind == "line_items" {
            continue;
        }
        for f in &section.fields {
            let (value, is_gap) = if f.fill == "static" {
                (f.static_value.clone(), false)
            } else {
                match values.get(&f.key) {
                    Some(v) => (Some(v.clone()), false),
                    None => (None, true),
                }
            };
            out.push(serde_json::json!({
                "section_key": section.key,
                "key": f.key,
                "label": f.label,
                "kind": f.kind,
                "fill": f.fill,
                "value": value,
                "is_gap": is_gap,
            }));
        }
    }
    out
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
        let (session, items, schema) = {
            let store = self.locked()?;
            let session = store.get_session(session_id)?;
            if session.status != SessionStatus::Processed {
                return Err(CoreError::InvalidState(format!(
                    "cannot build a document for a {} session",
                    session.status.as_str()
                )));
            }
            // Plan 19 §4 step 1 — the legality UNION: the built-in vocabulary
            // (unchanged for built-ins, so every existing "illegal kind" test
            // is preserved verbatim) OR a custom trade-matched schema.
            let template = session.template.as_deref();
            let legal = doc_kinds_for_template(template).contains(&doc_kind)
                || store.has_active_schema(doc_kind, template)?;
            if !legal {
                return Err(CoreError::InvalidState(format!(
                    "'{doc_kind}' is not a legal document kind for template {:?}",
                    session.template
                )));
            }
            // §4 step 2 — resolve the active schema. A legal kind with no
            // resolvable schema (an operator tombstoned a built-in) fails
            // truthfully (R7) — NEVER a silent hardcoded fallback, which
            // would resurrect a deleted built-in.
            let schema = store.resolve_active_schema(doc_kind, template)?.ok_or_else(|| {
                CoreError::InvalidState(format!(
                    "no active schema for '{doc_kind}' (template {:?}) — it was removed",
                    session.template
                ))
            })?;
            let items = store.list_items_for_session(session_id)?;
            (session, items, schema)
        };

        // §4 step 3 — deterministic render from the schema's line_items
        // section (save-time validation guarantees exactly one; a corrupt row
        // degrades to unpriced rather than panicking across the boundary).
        let priced = schema
            .sections
            .iter()
            .find(|s| s.kind == "line_items")
            .is_some_and(|s| s.priced);
        let mut lines = render_lines(&items, GapPolicy::PerPricingKind, priced);
        let mut usage = Usage::default();
        let mut queued = false;

        if priced && !items.is_empty() {
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

        // §4 step 5 — the fill pass: ONE focused call iff the schema has ≥1
        // LLM-fillable (`fill: "walk"`) field. Built-ins have none → zero
        // calls → byte-identical (the launch-safety spine). `manual` fields
        // are never offered to the model — they are always gaps in v1.
        let walk_fields: Vec<SchemaField> = schema
            .sections
            .iter()
            .filter(|s| s.kind == "filled")
            .flat_map(|s| s.fields.iter().filter(|f| f.fill == "walk").cloned())
            .collect();
        let mut fill_values: HashMap<String, String> = HashMap::new();
        if !walk_fields.is_empty() {
            match fill_fields(
                &self.provider,
                &walk_fields,
                &items,
                session.summary.as_deref().unwrap_or(""),
                self.max_tokens,
                &mut usage,
            )
            .await
            {
                Ok(map) => fill_values = map,
                // Mirrors the pricing degrade exactly (R7): a model call this
                // build needed didn't complete — regenerate to retry. Every
                // walk field then falls to a truthful gap below.
                Err(_) => queued = true,
            }
        }
        let fields = assemble_fields(&schema, &fill_values);

        // §4 step 6 — the total shape comes from the schema envelope (for
        // every built-in this equals the old total_shape(doc_kind) exactly).
        let payload = serde_json::json!({
            "doc_kind": doc_kind,
            "job_date_unix": session.started_at,
            "total_kind": schema.total_kind,
            "total_label_key": schema.total_label_key,
            "static_total_cents": serde_json::Value::Null,
            "lines": lines,
            "queued": queued,
            // Plan 19 Stage 5 — ADDITIVE body keys (the Plan 12 item_id
            // precedent): the schema row's numbering prefix (§4 step 7) and
            // the authored fields. Built-ins emit `fields: []` and today's
            // prefix, so the Stage 4 byte-identical net holds on the shared
            // fields.
            "number_prefix": schema.number_prefix,
            "fields": fields,
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
    /// of the FFI's former offline fallback, `partial_document_from_items`
    /// (title/detail/qty/amount_cents/section/item_id/is_gap), waiving only
    /// `id`. The fallback was removed once notes-first left it caller-less;
    /// the contract stays pinned here.
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

    /// Plan 16 Task 2 (D2-16): a quantity edit reaches every rebuilt
    /// document — `qty = item.right` — while un-edited items (right == "")
    /// keep today's `qty == ""` exactly (behavior-preserving).
    #[test]
    fn qty_renders_from_item_right() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        let its = items(&store, &session.id, &[("part", "bark mulch"), ("todo", "order lumber")]);
        store.update_item(&its[0].id, None, None, Some("3 CU YD")).unwrap();
        let its = store.list_items_for_session(&session.id).unwrap();

        let lines = render_structure_document("estimate", &its, GapPolicy::PerPricingKind);
        assert_eq!(lines[0]["qty"], "3 CU YD", "the right edit propagates as qty");
        assert_eq!(lines[1]["qty"], "", "an un-edited item keeps today's empty qty");
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

    // ---- Plan 19 Stage 4: schema-driven build (launch-safety) -----------

    /// Template-generic sibling of `processed_session_with_items` (which
    /// stays untouched — Δ=0 discipline): `None` template supported.
    fn processed_session_with_template(
        template: Option<&str>,
        texts: &[(&str, &str)],
    ) -> (Store, String) {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = match template {
            Some(t) => store.start_session_with_template(None, t).unwrap(),
            None => store.start_session(None).unwrap(),
        };
        let mut run_item_ids = Vec::new();
        for (kind, text) in texts {
            let item = store
                .add_item_with_source(&session.id, kind, text, ItemSource::Authoritative)
                .unwrap();
            run_item_ids.push(item.id);
        }
        store.append_transcript(&session.id, "site walk").unwrap();
        store.end_and_record_session(&session.id).unwrap();
        store
            .finish_session_processed(&session.id, "Walked the site.", &Usage::default(), &run_item_ids)
            .unwrap();
        (store, session.id)
    }

    #[tokio::test]
    async fn build_resolves_the_seeded_schema_for_every_trade_kind() {
        let cases: &[(Option<&str>, &[&str])] = &[
            (Some("landscape"), &["estimate", "invoice", "work_order"]),
            (Some("property"), &["condition", "move_out"]),
            (Some("inspection"), &["inspection"]),
            (None, &["report"]),
        ];
        for (template, kinds) in cases {
            for kind in *kinds {
                let (store, sid) =
                    processed_session_with_template(*template, &[("todo", "order lumber")]);
                let store = Arc::new(Mutex::new(store));
                // Empty mock: pricing kinds degrade to queued (a document
                // still lands), non-pricing kinds make zero calls.
                let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![]));
                let b = builder(store.clone(), provider);
                let outcome = b.build(&sid, kind).await.unwrap_or_else(|e| {
                    panic!("{kind} must build for template {template:?}: {e}")
                });
                let store = store.lock().unwrap();
                let art = store.get_artifact(&outcome.document_artifact_id).unwrap();
                let v: serde_json::Value = serde_json::from_str(&art.body).unwrap();
                assert_eq!(v["doc_kind"], *kind, "the seeded schema resolved and built");
                assert_eq!(v["doc_number"], 1);
            }
        }
    }

    /// The Stage 4 golden: for each of the 7 built-ins, the decoded payload's
    /// today-existing fields equal the pre-refactor values (computed here
    /// from the OLD hardcoded functions `is_pricing_kind`/`total_shape`,
    /// which stay in `pipeline/mod.rs` as the parity reference). `id`/
    /// `doc_number` excluded — non-deterministic pre-refactor too.
    #[tokio::test]
    async fn builtin_output_is_byte_identical_per_trade_kind() {
        use crate::pipeline::total_shape;
        let builtins: &[(Option<&str>, &str)] = &[
            (Some("landscape"), "estimate"),
            (Some("landscape"), "invoice"),
            (Some("landscape"), "work_order"),
            (Some("property"), "condition"),
            (Some("property"), "move_out"),
            (Some("inspection"), "inspection"),
            (None, "report"),
        ];
        for (template, kind) in builtins {
            let (store, sid) = processed_session_with_template(
                *template,
                &[("todo", "order lumber"), ("part", "bark mulch")],
            );
            let ids: Vec<String> = store
                .list_items_for_session(&sid)
                .unwrap()
                .into_iter()
                .map(|i| i.id)
                .collect();
            let store = Arc::new(Mutex::new(store));
            let pricing = is_pricing_kind(kind);
            // Pricing kinds get a scripted price on item 1 (both pre- and
            // post-refactor paths make exactly one pricing call).
            let responses = if pricing {
                vec![tool_use(
                    "price_items",
                    serde_json::json!({"prices": [{"item_id": ids[0], "amount_cents": 28500}]}),
                )]
            } else {
                vec![]
            };
            let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(responses));
            let b = builder(store.clone(), provider);
            let outcome = b.build(&sid, kind).await.unwrap();

            let store = store.lock().unwrap();
            let art = store.get_artifact(&outcome.document_artifact_id).unwrap();
            let v: serde_json::Value = serde_json::from_str(&art.body).unwrap();
            let (total_kind, total_label_key) = total_shape(kind);
            assert_eq!(v["doc_kind"], *kind);
            assert_eq!(v["total_kind"], total_kind, "{kind}: total_kind is today's");
            assert_eq!(v["total_label_key"], total_label_key, "{kind}: label key is today's");
            assert_eq!(v["static_total_cents"], serde_json::Value::Null);
            assert_eq!(v["queued"], false, "{kind}: no degrade in the golden path");
            let lines = v["lines"].as_array().unwrap();
            assert_eq!(lines.len(), 2);
            let expected: Vec<(&str, &str, Option<i64>, bool)> = if pricing {
                vec![
                    ("order lumber", "", Some(28500), false),
                    ("bark mulch", "", None, true),
                ]
            } else {
                vec![("order lumber", "", None, false), ("bark mulch", "", None, false)]
            };
            for ((line, item_id), (title, qty, amount, is_gap)) in
                lines.iter().zip(&ids).zip(expected)
            {
                assert_eq!(line["title"], title, "{kind}");
                assert_eq!(line["detail"], "", "{kind}");
                assert_eq!(line["qty"], qty, "{kind}");
                assert_eq!(
                    line["amount_cents"],
                    amount.map_or(serde_json::Value::Null, |a| serde_json::json!(a)),
                    "{kind}"
                );
                assert_eq!(line["section"], serde_json::Value::Null, "{kind}: section stays null");
                assert_eq!(line["is_gap"], is_gap, "{kind}");
                assert_eq!(line["item_id"], *item_id, "{kind}");
            }
        }
    }

    #[tokio::test]
    async fn build_errors_when_the_builtin_schema_was_tombstoned() {
        let (store, sid) = processed_session_with_items(&[("todo", "mulch")]);
        store
            .remove_document_schema(crate::domain::BUILTIN_SCHEMA_ID_ESTIMATE)
            .unwrap();
        let store = Arc::new(Mutex::new(store));
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![]));
        let b = builder(store.clone(), provider);
        let err = b.build(&sid, "estimate").await.unwrap_err();
        assert!(
            matches!(err, CoreError::InvalidState(_)),
            "truthful failure, never a hardcoded fallback (that would resurrect): {err}"
        );
        let store = store.lock().unwrap();
        assert!(
            store
                .list_artifacts_for_session(&sid)
                .unwrap()
                .iter()
                .all(|a| a.kind != "document"),
            "no document landed"
        );
    }

    // ---- Plan 19 Stage 5: fill_fields + number_prefix + fields[] ---------

    use crate::domain::{SchemaSection};

    fn walk_field(key: &str, label: &str) -> SchemaField {
        SchemaField {
            key: key.into(),
            kind: "text".into(),
            label: label.into(),
            fill: "walk".into(),
            static_value: None,
        }
    }

    /// The WE-B custom schema (§6): hoa_addendum / landscape / HOA;
    /// S1 line_items (priced=false), S2 filled "Approvals" (hoa_no,
    /// reviewed_by — both walk), S3 static "Terms" (terms_body).
    fn hoa_schema() -> DocumentSchema {
        DocumentSchema {
            id: "custom-hoa".into(),
            kind: "hoa_addendum".into(),
            label: "HOA Addendum".into(),
            number_prefix: "HOA".into(),
            trade_key: Some("landscape".into()),
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
                        walk_field("hoa_no", "HOA approval #"),
                        walk_field("reviewed_by", "Reviewed by"),
                    ],
                },
                SchemaSection {
                    key: "terms".into(),
                    kind: "static".into(),
                    label: "Terms".into(),
                    priced: false,
                    fields: vec![SchemaField {
                        key: "terms_body".into(),
                        kind: "static".into(),
                        label: "Terms".into(),
                        fill: "static".into(),
                        static_value: Some("Valid for 30 days.".into()),
                    }],
                },
            ],
            schema_version: 1,
            created_at: 0,
            updated_at: 0,
            device_id: String::new(),
        }
    }

    /// The WE-B pinned session: landscape, I1 todo "Install boxwood hedge",
    /// I2 part "bark mulch" (right "3 CU YD"), summary as pinned. Returns
    /// (store, session_id, [id_a, id_b]).
    fn we_b_session() -> (Store, String, Vec<String>) {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session_with_template(None, "landscape").unwrap();
        let a = store
            .add_item_with_source(&session.id, "todo", "Install boxwood hedge", ItemSource::Authoritative)
            .unwrap();
        let b = store
            .add_item_with_source(&session.id, "part", "bark mulch", ItemSource::Authoritative)
            .unwrap();
        store.update_item(&b.id, None, None, Some("3 CU YD")).unwrap();
        store.append_transcript(&session.id, "front yard walk").unwrap();
        store.end_and_record_session(&session.id).unwrap();
        store
            .finish_session_processed(
                &session.id,
                "Walked the front yard; HOA approval 41827 on file.",
                &Usage::default(),
                &[a.id.clone(), b.id.clone()],
            )
            .unwrap();
        store.save_document_schema(&hoa_schema()).unwrap();
        (store, session.id, vec![a.id, b.id])
    }

    fn fill_response(fields: serde_json::Value) -> harness::CompletionResponse {
        tool_use("fill_fields", serde_json::json!({ "fields": fields }))
    }

    fn decoded_document(
        store: &Arc<Mutex<Store>>,
        artifact_id: &str,
    ) -> serde_json::Value {
        let store = store.lock().unwrap();
        let art = store.get_artifact(artifact_id).unwrap();
        serde_json::from_str(&art.body).unwrap()
    }

    #[tokio::test]
    async fn fill_fields_echoes_and_validates_and_drops_unknown_keys() {
        let fields = vec![walk_field("hoa_no", "HOA approval #"), walk_field("reviewed_by", "Reviewed by")];
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![fill_response(
            serde_json::json!([
                {"key": "hoa_no", "value": "41827"},
                {"key": "gate_code", "value": "9999"},
                {"key": "hoa_no", "value": "override-attempt"},
                {"key": "reviewed_by", "value": "Dana"}
            ]),
        )]));
        let mut usage = Usage::default();
        let map = fill_fields(&provider, &fields, &[], "summary", 512, &mut usage).await.unwrap();
        assert_eq!(map.get("hoa_no").map(String::as_str), Some("41827"), "first-wins dedup");
        assert_eq!(map.get("reviewed_by").map(String::as_str), Some("Dana"));
        assert_eq!(map.get("gate_code"), None, "hallucinated key dropped");
        assert_eq!(map.len(), 2);
        assert_eq!(usage, Usage { input_tokens: 80, output_tokens: 15 }, "R9: usage accumulated");
    }

    /// R6 + the WE-B exact prompt: items (via `format_pricing_items`
    /// verbatim — no `right_text`) + summary reach the request; the
    /// transcript never does.
    #[tokio::test]
    async fn fill_fields_fed_items_and_summary_never_the_transcript() {
        let store = Store::open_in_memory("device-a").unwrap();
        let session = store.start_session(None).unwrap();
        // Literal WE-B item ids, hand-built (not store-minted) so the pinned
        // message can be compared EXACTLY.
        let mk = |id: &str, kind: &str, text: &str, right: &str| CapturedItem {
            id: id.into(),
            session_id: session.id.clone(),
            kind: kind.into(),
            text: text.into(),
            right: right.into(),
            source: ItemSource::Authoritative,
            done: false,
            created_at: 0,
            updated_at: 0,
            device_id: "device-a".into(),
        };
        let items =
            vec![mk("item-A", "todo", "Install boxwood hedge", ""), mk("item-B", "part", "bark mulch", "3 CU YD")];
        let fields = vec![walk_field("hoa_no", "HOA approval #"), walk_field("reviewed_by", "Reviewed by")];
        let provider = Arc::new(MockProvider::new(vec![fill_response(serde_json::json!([
            {"key": "hoa_no", "value": "41827"}
        ]))]));
        let dyn_provider: Arc<dyn LlmProvider> = provider.clone();
        let mut usage = Usage::default();
        fill_fields(
            &dyn_provider,
            &fields,
            &items,
            "Walked the front yard; HOA approval 41827 on file.",
            512,
            &mut usage,
        )
        .await
        .unwrap();

        let reqs = provider.requests();
        let ContentBlock::Text { text } = &reqs[0].messages[0].content[0] else {
            panic!("expected text content");
        };
        let expected = "Fill these document fields from the session. Put a value only on a field whose\n\
                        answer was clearly stated — omit any field you are unsure about; a blank field\n\
                        is cheaper than a wrong one.\n\
                        \n\
                        Fields:\n\
                        - [hoa_no] HOA approval #\n\
                        - [reviewed_by] Reviewed by\n\
                        \n\
                        Session items:\n\
                        - [todo] Install boxwood hedge (item_id: item-A)\n\
                        - [part] bark mulch (item_id: item-B)\n\
                        \n\
                        Session summary:\n\
                        Walked the front yard; HOA approval 41827 on file.";
        assert_eq!(text, expected, "the exact WE-B user message — note right_text is absent \
                    (format_pricing_items omits it; the fill pass does not re-add it)");
        assert!(!text.to_lowercase().contains("transcript"), "never the transcript (R6)");
    }

    /// WE-B end-to-end (§6): lines + fields + static, exact.
    #[tokio::test]
    async fn custom_schema_full_render() {
        let (store, sid, ids) = we_b_session();
        let store = Arc::new(Mutex::new(store));
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![fill_response(
            serde_json::json!([{"key": "hoa_no", "value": "41827"}]),
        )]));
        let b = builder(store.clone(), provider);
        let outcome = b.build(&sid, "hoa_addendum").await.unwrap();
        assert!(!outcome.queued);

        let v = decoded_document(&store, &outcome.document_artifact_id);
        // line_items (priced=false → is_gap false, today's non-pricing posture)
        let lines = v["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["title"], "Install boxwood hedge");
        assert_eq!(lines[0]["detail"], "");
        assert_eq!(lines[0]["qty"], "");
        assert_eq!(lines[0]["amount_cents"], serde_json::Value::Null);
        assert_eq!(lines[0]["section"], serde_json::Value::Null);
        assert_eq!(lines[0]["is_gap"], false);
        assert_eq!(lines[0]["item_id"], ids[0]);
        assert_eq!(lines[1]["title"], "bark mulch");
        assert_eq!(lines[1]["qty"], "3 CU YD");
        assert_eq!(lines[1]["is_gap"], false);
        assert_eq!(lines[1]["item_id"], ids[1]);
        // fields[] in schema order — the exact WE-B rows
        assert_eq!(
            v["fields"],
            serde_json::json!([
                {"section_key": "approvals", "key": "hoa_no", "label": "HOA approval #",
                 "kind": "text", "fill": "walk", "value": "41827", "is_gap": false},
                {"section_key": "approvals", "key": "reviewed_by", "label": "Reviewed by",
                 "kind": "text", "fill": "walk", "value": null, "is_gap": true},
                {"section_key": "terms", "key": "terms_body", "label": "Terms",
                 "kind": "static", "fill": "static", "value": "Valid for 30 days.", "is_gap": false}
            ])
        );
        assert_eq!(v["total_kind"], "sum");
        assert_eq!(v["total_label_key"], "total");
        assert_eq!(v["number_prefix"], "HOA");
        assert_eq!(v["doc_number"], 1, "HOA-0001 once Swift consumes the prefix");
        assert_eq!(v["queued"], false);
    }

    /// R6 (WE-B F2): a field the model omitted is a truthful gap row.
    #[tokio::test]
    async fn omitted_field_renders_as_a_gap_row() {
        let (store, sid, _) = we_b_session();
        let store = Arc::new(Mutex::new(store));
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![fill_response(
            serde_json::json!([{"key": "hoa_no", "value": "41827"}]),
        )]));
        let b = builder(store.clone(), provider);
        let outcome = b.build(&sid, "hoa_addendum").await.unwrap();
        let v = decoded_document(&store, &outcome.document_artifact_id);
        let reviewed = &v["fields"][1];
        assert_eq!(reviewed["key"], "reviewed_by");
        assert_eq!(reviewed["value"], serde_json::Value::Null);
        assert_eq!(reviewed["is_gap"], true, "not stated → gap, never fabricated (R6)");
    }

    #[tokio::test]
    async fn static_field_passes_through_its_value() {
        let (store, sid, _) = we_b_session();
        let store = Arc::new(Mutex::new(store));
        let provider: Arc<dyn LlmProvider> =
            Arc::new(MockProvider::new(vec![fill_response(serde_json::json!([]))]));
        let b = builder(store.clone(), provider);
        let outcome = b.build(&sid, "hoa_addendum").await.unwrap();
        let v = decoded_document(&store, &outcome.document_artifact_id);
        let terms = &v["fields"][2];
        assert_eq!(terms["fill"], "static");
        assert_eq!(terms["value"], "Valid for 30 days.");
        assert_eq!(terms["is_gap"], false, "an authored constant is never a gap");
    }

    #[tokio::test]
    async fn manual_field_is_always_a_gap_in_v1() {
        let (store, sid, _) = we_b_session();
        // A schema whose only filled field is manual: no LLM call at all.
        let mut schema = hoa_schema();
        schema.id = "custom-manual".into();
        schema.kind = "site_signoff".into();
        schema.number_prefix = "SIGN".into();
        schema.sections[1].fields = vec![SchemaField {
            key: "signed_by".into(),
            kind: "text".into(),
            label: "Signed by".into(),
            fill: "manual".into(),
            static_value: None,
        }];
        store.save_document_schema(&schema).unwrap();
        let store = Arc::new(Mutex::new(store));
        let provider = Arc::new(MockProvider::new(vec![]));
        let b = builder(store.clone(), provider.clone());
        let outcome = b.build(&sid, "site_signoff").await.unwrap();
        assert!(!outcome.queued);
        assert!(provider.requests().is_empty(), "manual fields are never offered to the model");
        let v = decoded_document(&store, &outcome.document_artifact_id);
        assert_eq!(v["fields"][0]["key"], "signed_by");
        assert_eq!(v["fields"][0]["value"], serde_json::Value::Null);
        assert_eq!(v["fields"][0]["is_gap"], true, "operator completes at review — gap in v1");
    }

    /// R7: a fill provider `Err` degrades exactly like the pricing degrade
    /// (`document.rs` pricing match): `queued = true`, fields fall to gaps,
    /// never a hard build failure.
    #[tokio::test]
    async fn fill_call_failure_sets_queued_and_degrades_fields_to_gaps() {
        let (store, sid, _) = we_b_session();
        let store = Arc::new(Mutex::new(store));
        // Empty response queue -> provider errors on the fill call.
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![]));
        let b = builder(store.clone(), provider);
        let outcome = b.build(&sid, "hoa_addendum").await.unwrap();
        assert!(outcome.queued, "a model call this build needed didn't complete");

        let v = decoded_document(&store, &outcome.document_artifact_id);
        assert_eq!(v["queued"], true);
        assert_eq!(v["fields"][0]["is_gap"], true, "hoa_no degraded to a gap");
        assert_eq!(v["fields"][0]["value"], serde_json::Value::Null);
        assert_eq!(v["fields"][1]["is_gap"], true, "reviewed_by degraded to a gap");
        assert_eq!(v["fields"][2]["is_gap"], false, "the static field is untouched by the degrade");
        assert_eq!(v["doc_number"], 1, "the document still mints and lands (R7)");
    }

    /// The contrast pin: the call SUCCEEDED but omitted a field — a truthful
    /// gap WITHOUT `queued` (two distinct meanings, §4 step 5).
    #[tokio::test]
    async fn model_declined_field_is_a_gap_without_queued() {
        let (store, sid, _) = we_b_session();
        let store = Arc::new(Mutex::new(store));
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![fill_response(
            serde_json::json!([{"key": "hoa_no", "value": "41827"}]),
        )]));
        let b = builder(store.clone(), provider);
        let outcome = b.build(&sid, "hoa_addendum").await.unwrap();
        assert!(!outcome.queued, "the call completed — nothing to retry");
        let v = decoded_document(&store, &outcome.document_artifact_id);
        assert_eq!(v["queued"], false);
        assert_eq!(v["fields"][1]["is_gap"], true, "the declined field is simply a gap");
    }

    /// WE-C (§6): per-kind independent counters via the existing
    /// document_sequences mechanism; `number_prefix` from each resolved
    /// schema row. The six interleaved builds, exact.
    #[tokio::test]
    async fn number_prefix_comes_from_the_schema_row_across_interleaved_builds() {
        // Zero items: pricing (estimate) and fill (none authored) both skip —
        // pure numbering.
        let (store, sid) = processed_session_with_items(&[]);
        let mut hoa = hoa_schema();
        hoa.sections[1].fields.clear(); // no fill calls in the WE-C trace
        store.save_document_schema(&hoa).unwrap();
        let mut punch = hoa_schema();
        punch.id = "custom-punch".into();
        punch.kind = "punch_list".into();
        punch.number_prefix = "PUN".into();
        punch.sections[1].fields.clear();
        store.save_document_schema(&punch).unwrap();
        let store = Arc::new(Mutex::new(store));
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new(vec![]));
        let b = builder(store.clone(), provider);

        let expected: &[(&str, u64, &str)] = &[
            ("estimate", 1, "EST"),
            ("hoa_addendum", 1, "HOA"),
            ("estimate", 2, "EST"),
            ("punch_list", 1, "PUN"),
            ("hoa_addendum", 2, "HOA"),
            ("estimate", 3, "EST"),
        ];
        for (kind, number, prefix) in expected {
            let outcome = b.build(&sid, kind).await.unwrap();
            let v = decoded_document(&store, &outcome.document_artifact_id);
            assert_eq!(v["doc_number"], *number, "{kind}: per-kind independent counter");
            assert_eq!(v["number_prefix"], *prefix, "{kind}: prefix from the schema ROW");
        }
    }

    /// The byte-identical guard on the additive keys: built-ins emit
    /// `fields: []` and today's prefix.
    #[tokio::test]
    async fn builtins_emit_empty_fields_and_todays_prefix() {
        let (store, sid) = processed_session_with_items(&[("todo", "mulch")]);
        let store = Arc::new(Mutex::new(store));
        let provider = Arc::new(MockProvider::new(vec![]));
        let b = builder(store.clone(), provider.clone());
        let outcome = b.build(&sid, "work_order").await.unwrap();
        let v = decoded_document(&store, &outcome.document_artifact_id);
        assert_eq!(v["fields"], serde_json::json!([]), "zero authored fields on a built-in");
        assert_eq!(v["number_prefix"], "WO", "today's Swift-side prefix, now also in the body");
        assert!(provider.requests().is_empty(), "zero fill calls (launch-safety)");
    }
}
