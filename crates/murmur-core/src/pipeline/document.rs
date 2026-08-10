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
use crate::pipeline::spoken_price::{fold_spoken_prices, FoldedPrices};
use crate::pipeline::notes::{parse_notes_artifact, NotesEntry, NOTE_BUCKETS};
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
                // Written only by the compose pass, on `directive` documents.
                "assignee": null,
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
    // R6 / CORE.md #4, stated as narrowly as it can be: a price comes from
    // what THIS operator has charged, or it does not exist.
    //
    // The previous wording — "an item whose value you can reasonably infer
    // from its text" — was a licence to price from general market knowledge,
    // and it took it: on Isaac's own walk (2026-08-10) the operator stated
    // four prices totalling $1,250 and the pass returned $3,300, having
    // invented $750 for pruning, $750 for tomato plants and $1,250 for
    // artichokes. Numbers he never said, on a document a client signs, in the
    // product whose core promise is "unheard ≠ invented, ever".
    //
    // A gap is recoverable in five seconds — the operator taps it and types
    // the number they already know. An invented price is recoverable only if
    // somebody notices it, and the person best placed to notice is the client.
    let system = format!(
        "You price items from a field-work session for a tradesperson's estimate/invoice. \
         Put a price on an item ONLY when this specific operator's own pricing history tells \
         you what they charge for it. You have no other source of prices: you do not know this \
         trade's going rate, this region's rates, or what these materials cost, and a plausible \
         number is worse than no number — it is a commitment the operator never made, on a \
         document their client may sign. Leave everything else unpriced; they will fill it in \
         themselves in seconds. Omitting an item is always correct. You may price only items \
         from the given list, by their exact item_id.{memory_block}"
    );
    // The total is a CEILING TO CHECK AGAINST, never a budget to allocate.
    //
    // It used to say "allocate line prices consistent with this", which is an
    // instruction to invent: on Isaac's walk (2026-08-10) the operator stated
    // four line prices summing to $1,250 and the pass put $1,250 — the whole
    // total — on "Prune five pear trees", a line he never priced. It defeated
    // the narrowed rule above single-handedly, because the rule says where
    // prices come from and this sentence said to make some up.
    //
    // A stated total is still worth sending: it catches a line priced far
    // beyond what the whole job was quoted at. It just must not become a pot
    // to distribute.
    let hint_block = spoken_total_cents
        .map(|cents| {
            format!(
                "\n\nThe operator quoted ${:.2} for the whole job. Use this ONLY as a \
                 sanity check on prices you already have grounds for — never as a budget to \
                 distribute across the lines, and never as a reason to price a line you would \
                 otherwise leave alone.",
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
            // One forced call, never re-sent — a cache write would bill ~1.25x
            // with nothing to read it back.
            cache_prefix: false,
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
            //
            // Zero and negative are dropped too. A model that returns 0 has no
            // price for that line, and rendering "$0" states a price to the
            // client — Isaac's EST-0005 went out with a "$0" line on it. An
            // honest gap says "this needs a number"; "$0" says "this is free",
            // which is a commitment nobody made. An operator who really means
            // no-charge can type it on the line.
            if amount > 0 && valid_ids.contains(id) && !map.contains_key(id) {
                map.insert(id.to_string(), amount);
            }
        }
    }
    Ok(map)
}

const COMPOSE_DOCUMENT: &str = "compose_document";

/// What the compose pass wrote onto one line.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LineComposition {
    /// The second line under the item: what it covers, what to do, or what
    /// was seen — per the schema's `line_detail`.
    pub detail: String,
    /// Who is doing it. `directive` documents only, and only when a person
    /// was actually named for that line.
    pub assignee: Option<String>,
}

/// One pass's output: the authored fields, and the per-line writing.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Composition {
    /// Field key → value. Absent = a truthful gap.
    pub fields: HashMap<String, String>,
    /// `item_id` → what to write on that line.
    pub lines: HashMap<String, LineComposition>,
}

fn compose_document_tool_spec(wants_lines: bool, wants_assignee: bool) -> ToolSpec {
    let mut line_props = serde_json::json!({
        "item_id": { "type": "string" },
        "detail": { "type": "string", "description": "the second line under this item" },
    });
    if wants_assignee {
        line_props["assignee"] = serde_json::json!({
            "type": "string",
            "description": "the person named as doing THIS line, exactly as spoken. Omit \
                             unless someone was actually named for it."
        });
    }
    let mut properties = serde_json::json!({
        "fields": {
            "type": "array",
            "description": "Values for the named fields. Omit any field you are unsure about.",
            "items": {
                "type": "object",
                "properties": {
                    "key": { "type": "string" },
                    "value": { "type": "string" }
                },
                "required": ["key", "value"]
            }
        }
    });
    if wants_lines {
        properties["lines"] = serde_json::json!({
            "type": "array",
            "description": "One entry per line you can say something REAL about, by its exact \
                             item_id. Omit any line you would have to invent detail for.",
            "items": {
                "type": "object",
                "properties": line_props,
                "required": ["item_id", "detail"]
            }
        });
    }
    ToolSpec {
        name: COMPOSE_DOCUMENT.into(),
        description: "Write the prose of a field-work document: the named fields, and the \
                       second line under each item. You may fill only the fields and items \
                       given, by their exact key and item_id — you cannot add, rename, drop \
                       or reorder anything."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": ["fields"]
        }),
    }
}

/// The per-line brief, keyed by the schema's `line_detail`. Each one names
/// the READER, because that is what actually decides the writing: the same
/// item becomes "delivered and installed, 3 cu yd" for a client weighing a
/// price, "strip the old bark before laying — watch the irrigation heads"
/// for the crew doing it, and "south slope, three shingles lifted at the
/// ridge" for whoever reads the record next year.
fn line_brief(line_detail: &str) -> Option<String> {
    // The rule that stops this pass padding, appended to every brief so it
    // cannot drift out of one of them.
    //
    // It is here because the first real-API run produced it: handed the item
    // "Strip old bark from front beds", the model wrote back "Strip old bark
    // from front beds before mulching." — new words, no new information, on
    // every single line. That is worse than a blank second line. It doubles
    // the length of the document AND teaches the reader that the second line
    // is noise, so they skim past the one line that actually carries a
    // warning. Mocks cannot catch this: the shape was perfect.
    const NOTHING_TO_ADD: &str = " If the line's own title already says everything you would \
         write, OMIT that line entirely — a second line that restates the first is padding, and \
         a document that pads is one a reader stops trusting. Never restate the title in other \
         words. Write only where you are ADDING something the title does not say.";

    let brief = match line_detail {
        "inclusion" => {
            "For each line, write what that line COVERS — quantity, material, and what is \
             included in its price (delivered? hauled away? how many coats?). A short phrase, \
             not a sentence. It is read by a client deciding whether the number beside it is \
             fair, so it must justify the number without repeating it."
        }
        "directive" => {
            "This is a WORK ORDER: the crew reads it, standing at the site, to do the job \
             without calling anyone. For each line write ONLY what the title does not already \
             tell them — the order to work in, the technique, the thing to watch out for, the \
             spec to match. One short imperative sentence, in trade language. Where the \
             operator named a person for a line, set `assignee` to that name exactly as \
             spoken; a line nobody was named for gets no assignee."
        }
        "observation" => {
            "For each line, write what was actually OBSERVED — where it is, how bad, how much, \
             what condition. One or two short phrases. This is a record that may be read months \
             from now by someone who was not there (a tenant disputing a deduction, a buyer, an \
             adjuster), so specifics beat adjectives."
        }
        _ => return None,
    };
    Some(format!("{brief}{NOTHING_TO_ADD}"))
}

/// Renders the coordination notes the walk already produced as context.
///
/// These cost NOTHING to include: `summarize()` captured them at finish and
/// they are sitting in the session's `notes` artifact. They are also exactly
/// what the terse item list is missing — the gate code, the client's
/// preference, the condition that changes the work — so a document written
/// without them is guaranteed to be thinner than the walk was.
fn format_notes_context(buckets: &[NotesEntry]) -> String {
    if buckets.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n\nWhat was said around the work:\n");
    for bucket in NOTE_BUCKETS {
        let entries: Vec<&NotesEntry> = buckets.iter().filter(|b| b.bucket == bucket).collect();
        if entries.is_empty() {
            continue;
        }
        out.push_str(&format!("{}:\n", bucket.replace('_', " ")));
        for e in entries {
            out.push_str(&format!("- {}: {}\n", e.label, e.detail));
        }
    }
    // No trailing newline — every block in the user message supplies its own
    // leading separator, so one here doubles up.
    out.trim_end().to_string()
}

/// The output budget for one compose call, scaled to the document.
///
/// The pricing pass writes an integer per line and fits in a flat budget
/// forever. This one writes PROSE — a paragraph per field plus a sentence or
/// two per line — so a flat budget is a cliff: the model runs out mid-tool-call,
/// the JSON is truncated, the parse fails, and the whole pass degrades to
/// `queued` with every field a gap and every line bare.
///
/// That failure is silent and lands hardest on exactly the walks worth the
/// most — a thirty-item walk on a big job blows a 1024-token budget while a
/// four-item walk sails through, so the operator experiences it as "the long
/// ones don't work" with no error to report. Hence: budget by the size of the
/// thing being written, floored at the caller's value so no document gets
/// LESS room than before, and capped so a runaway item list cannot authorize
/// an unbounded spend.
fn compose_budget(floor: u32, field_count: usize, item_count: usize, writes_lines: bool) -> u32 {
    const PER_FIELD: u32 = 128; // a long_text field is a short paragraph
    const PER_LINE: u32 = 64; // one or two short sentences, plus JSON overhead
    const CEILING: u32 = 8192;
    let lines = if writes_lines { item_count as u32 * PER_LINE } else { 0 };
    let wanted = 256 + (field_count as u32 * PER_FIELD) + lines;
    wanted.clamp(floor, CEILING)
}

/// The document's prose, in one call.
///
/// This is `price_items`' twin in every structural way — forced single-shot
/// tool, echo-and-validate against ids we supplied, first-wins dedup,
/// unknown ids dropped rather than failing the pass, usage banked before
/// success is decided (R9) — and it replaces the narrower `fill_fields`.
///
/// **Why one pass and not two.** Fields and line details are the same act of
/// writing: the scope paragraph is a summary of the very lines being
/// detailed, so a model that writes both at once writes a document that
/// agrees with itself, and a second call would pay the same input tokens
/// twice to produce something that disagrees with the first. Pricing stays
/// separate — money is a different risk class with its own validation and
/// its own spoken-total hint, and it must not be entangled with prose.
///
/// **What it cannot do.** It cannot add, drop, rename or reorder a line, or
/// invent a field key: the structure is already fixed by the deterministic
/// render, and only the writing is at stake. R6 runs through the prompt: a
/// line it has nothing real to say about gets nothing, because a document
/// that pads is a document that eventually pads with something false.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn compose_document(
    provider: &Arc<dyn LlmProvider>,
    document_label: &str,
    line_detail: &str,
    fields: &[SchemaField],
    items: &[CapturedItem],
    summary: &str,
    notes: &[NotesEntry],
    max_tokens: u32,
    usage: &mut Usage,
) -> Result<Composition, HarnessError> {
    let brief = line_brief(line_detail);
    let wants_assignee = line_detail == "directive";
    let system = format!(
        "You write the prose of a {document_label} for a small trade operator, from one \
         recorded site walk. Everything you write must come from what was actually said on \
         that walk. Never invent a quantity, a price, a name, a date, an access detail or a \
         condition — a document that is thin where the walk was thin is correct; one that \
         reads well because you filled the gaps is the single worst thing this product can \
         produce. Write in the operator's own trade language, not in marketing language, and \
         never mention the recording, the transcript, or yourself."
    );

    let fields_block = if fields.is_empty() {
        String::new()
    } else {
        let rendered = fields
            .iter()
            .map(|f| match &f.hint {
                Some(h) => format!("- [{}] {} — {h}", f.key, f.label),
                None => format!("- [{}] {}", f.key, f.label),
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("\n\nFields to write:\n{rendered}")
    };
    let lines_block =
        brief.as_deref().map(|b| format!("\n\nThe lines:\n{b}")).unwrap_or_default();
    let items_block = format_pricing_items(items);
    let notes_block = format_notes_context(notes);
    let summary_block = if summary.trim().is_empty() {
        String::new()
    } else {
        format!("\n\nSession summary:\n{summary}")
    };
    let user_message = format!(
        "Write this {document_label}.{fields_block}{lines_block}\
         \n\nThe items on it:\n{items_block}{notes_block}{summary_block}"
    );

    let response = provider
        .complete(CompletionRequest {
            system,
            messages: vec![Message::user_text(user_message)],
            tools: vec![compose_document_tool_spec(brief.is_some(), wants_assignee)],
            max_tokens: compose_budget(max_tokens, fields.len(), items.len(), brief.is_some()),
            tool_choice: Some(COMPOSE_DOCUMENT.to_string()),
            // Single-shot — see the PRICE_ITEMS note above.
            cache_prefix: false,
        })
        .await?;
    usage.add(&response.usage);

    let input = response.content.iter().find_map(|b| match b {
        ContentBlock::ToolUse { name, input, .. } if name == COMPOSE_DOCUMENT => Some(input.clone()),
        _ => None,
    });
    let input = input.ok_or_else(|| {
        HarnessError::Provider("compose_document response missing compose_document call".into())
    })?;

    let valid_keys: HashSet<&str> = fields.iter().map(|f| f.key.as_str()).collect();
    let mut out = Composition::default();
    if let Some(entries) = input.get("fields").and_then(|v| v.as_array()) {
        for e in entries {
            let (Some(key), Some(value)) =
                (e.get("key").and_then(|v| v.as_str()), e.get("value").and_then(|v| v.as_str()))
            else {
                continue;
            };
            // First-wins dedup; unknown/hallucinated keys are dropped — never
            // fail the whole pass over one bad row.
            if valid_keys.contains(key) && !out.fields.contains_key(key) && !value.trim().is_empty()
            {
                out.fields.insert(key.to_string(), value.trim().to_string());
            }
        }
    }

    let valid_ids: HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
    if let Some(entries) = input.get("lines").and_then(|v| v.as_array()) {
        for e in entries {
            let Some(id) = e.get("item_id").and_then(|v| v.as_str()) else { continue };
            if !valid_ids.contains(id) || out.lines.contains_key(id) {
                continue;
            }
            let detail =
                e.get("detail").and_then(|v| v.as_str()).unwrap_or_default().trim().to_string();
            // The assignee is only READ on a directive document. A model that
            // volunteers one on an estimate is answering a question nobody
            // asked, and a name in the price column would be a rendering bug
            // wearing a data bug's clothes.
            let assignee = if wants_assignee {
                e.get("assignee")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            } else {
                None
            };
            if detail.is_empty() && assignee.is_none() {
                continue;
            }
            out.lines.insert(id.to_string(), LineComposition { detail, assignee });
        }
    }
    Ok(out)
}

/// Writes the composed prose onto the rendered lines. A line whose `item_id`
/// the pass wrote about gains its `detail` and `assignee`; every other line
/// is left exactly as the deterministic render produced it.
fn apply_composition(composed: &HashMap<String, LineComposition>, lines: &mut [serde_json::Value]) {
    let mut claimed: HashSet<String> = HashSet::new();
    for line in lines.iter_mut() {
        let Some(item_id) = line.get("item_id").and_then(|v| v.as_str()).map(str::to_string) else {
            continue;
        };
        if claimed.contains(&item_id) {
            continue;
        }
        if let Some(written) = composed.get(&item_id) {
            if !written.detail.is_empty() {
                line["detail"] = serde_json::json!(written.detail);
            }
            if let Some(assignee) = &written.assignee {
                line["assignee"] = serde_json::json!(assignee);
            }
            claimed.insert(item_id);
        }
    }
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
            // Empty-walk guard (field fix, jefe-2026-07-24): a walk that
            // captured no items must NOT mint a document artifact. Building one
            // anyway produced the "blank work order" ghost — an all-dashes,
            // PHOTOS×0 document with a live SEND button — and, under D7
            // burn-per-tap, a fresh ghost artifact on every preview tap. Skip
            // truthfully instead: the FFI maps this to EngineError::Document and
            // the notes screen surfaces it, leaving the action available to
            // retry once the walk actually has content. (Same "don't create the
            // record until there's content" class as the Athanor empty-chat
            // report.)
            if items.is_empty() {
                return Err(CoreError::InvalidState(
                    "nothing to build — this walk has no items yet".into(),
                ));
            }
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

        // Fold spoken prices onto the work they describe, BEFORE the render
        // (Isaac's field report 2026-08-09 — "$200 mulch" was landing as its
        // own line under "5 yards mulch"). Only for documents that HAVE an
        // amount column: on an unpriced kind the amount lives nowhere but the
        // title, so lifting it out would delete it. See `spoken_price`.
        let folded =
            if priced { fold_spoken_prices(&items) } else { FoldedPrices::unfolded(&items) };
        let mut lines = render_lines(&folded.items, GapPolicy::PerPricingKind, priced);
        // Spoken amounts land first and are never overwritten below: a price
        // the operator said outranks a price the model inferred.
        apply_prices(&folded.amounts, &mut lines);
        let mut usage = Usage::default();
        let mut queued = false;

        // Only what is still unpriced goes to the model — it is cheaper, and
        // it removes any chance of the pass contradicting the operator.
        let unpriced: Vec<CapturedItem> = folded
            .items
            .iter()
            .filter(|i| !folded.amounts.contains_key(&i.id))
            .cloned()
            .collect();
        if priced && !unpriced.is_empty() {
            // The stated grand total is deliberately NOT sent (Plan 13 D5a's
            // hint, retired 2026-08-10). Its only job was allocation — "spread
            // line prices consistent with this" — and allocation is exactly
            // the invention Isaac ruled out: *"the prices should only get
            // filled in if it's spoken by the user or we have the price the
            // user has used in the past."*
            //
            // Rewording it to "sanity check only" was not enough: across four
            // real runs of the same walk it still put the entire $1,250 total
            // onto a line the operator never priced, once. A number that wrong
            // on a client's estimate one time in four is not a hint, and no
            // prompt sentence outranks a number sitting in the context.
            //
            // The capture itself stays (`session_spoken_total` /
            // `session_meta`): it is honest data, and a future ceiling check —
            // "this line alone exceeds the whole quoted job" — is a real use
            // that does not create prices.
            let hint = None;
            let memory_prompt = self
                .memory
                .lock()
                .map_err(|_| CoreError::InvalidState("memory lock poisoned".into()))?
                .to_prompt();
            match price_items(
                &self.provider,
                &unpriced,
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

        // §4 step 5 — the compose pass: ONE call iff this document has prose
        // to write, i.e. ≥1 LLM-fillable (`fill: "walk"`) field OR a
        // `line_detail` style. A schema with neither — an operator's own bare
        // list — still makes zero calls. `manual` fields are never offered to
        // the model; they are always gaps in v1.
        let walk_fields: Vec<SchemaField> = schema
            .sections
            .iter()
            .filter(|s| s.kind == "filled")
            .flat_map(|s| s.fields.iter().filter(|f| f.fill == "walk").cloned())
            .collect();
        let line_detail = schema
            .sections
            .iter()
            .find(|s| s.kind == "line_items")
            .map(|s| s.line_detail.clone())
            .unwrap_or_default();
        let mut composed = Composition::default();
        if !walk_fields.is_empty() || line_brief(&line_detail).is_some() {
            // The coordination notes the walk already produced — free
            // context: `summarize()` captured them at finish and they are
            // sitting in this session's `notes` artifact. Without them the
            // document is guaranteed to be thinner than the walk was.
            let notes = self.session_notes(session_id)?;
            match compose_document(
                &self.provider,
                &schema.label,
                &line_detail,
                &walk_fields,
                &folded.items,
                session.summary.as_deref().unwrap_or(""),
                &notes,
                self.max_tokens,
                &mut usage,
            )
            .await
            {
                Ok(c) => {
                    apply_composition(&c.lines, &mut lines);
                    composed = c;
                }
                // Mirrors the pricing degrade exactly (R7): a model call this
                // build needed didn't complete — regenerate to retry. Every
                // walk field then falls to a truthful gap below, and the lines
                // keep the operator's own words with no second line.
                Err(_) => queued = true,
            }
        }
        let fields = assemble_fields(&schema, &composed.fields);

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
    ///
    /// Caller-less since the pricing hint was retired (2026-08-10) — kept, not
    /// deleted, because the CAPTURE is still live: `process()` writes the
    /// artifact, and reading it back is the seam a future ceiling check would
    /// use ("this one line exceeds the whole quoted job"). That is a real use
    /// that does not create prices, unlike the hint that was removed.
    #[allow(dead_code)]
    fn session_spoken_total(&self, session_id: &str) -> Result<Option<i64>, CoreError> {
        let artifacts = self.locked()?.list_artifacts_for_session(session_id)?;
        let meta: Option<&Artifact> = artifacts.iter().rev().find(|a| a.kind == "session_meta");
        Ok(meta
            .and_then(|a| serde_json::from_str::<serde_json::Value>(&a.body).ok())
            .and_then(|v| v.get("spoken_total_cents").and_then(|n| n.as_i64())))
    }

    /// The coordination notes `process()` wrote at finish, as compose-pass
    /// context. `[]` when the walk produced none, or the artifact is garbled
    /// — `parse_notes_artifact` is deliberately tolerant (R7), and a document
    /// written without this context is thinner, never wrong.
    fn session_notes(&self, session_id: &str) -> Result<Vec<NotesEntry>, CoreError> {
        let artifacts = self.locked()?.list_artifacts_for_session(session_id)?;
        Ok(artifacts
            .iter()
            .rev()
            .find(|a| a.kind == "notes")
            .map(|a| parse_notes_artifact(&a.body))
            .unwrap_or_default())
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
            usage: Usage { input_tokens: 80, output_tokens: 15, ..Default::default() },
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
        assert_eq!(usage, Usage { input_tokens: 80, output_tokens: 15, ..Default::default() });
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

    /// The decoded document body for an artifact id.
    fn v_of(store: &Arc<Mutex<Store>>, artifact_id: &str) -> serde_json::Value {
        let store = store.lock().unwrap();
        serde_json::from_str(&store.get_artifact(artifact_id).unwrap().body).unwrap()
    }

    #[tokio::test]
    async fn build_happy_path_prices_and_mints_a_document() {
        let (store, sid) = processed_session_with_items(&[("todo", "mulch"), ("safety", "loose railing")]);
        // Re-read the real minted ids so the scripted response can echo one.
        let a1 = store.list_items_for_session(&sid).unwrap()[0].id.clone();
        let store = Arc::new(Mutex::new(store));

        let provider = Arc::new(MockProvider::new(vec![
            tool_use(
                "price_items",
                serde_json::json!({"prices": [{"item_id": a1, "amount_cents": 28500}]}),
            ),
            compose_response(
                serde_json::json!([{"key": "scope_summary", "value": "Mulch the front beds."}]),
                serde_json::json!([{"item_id": a1, "detail": "3 cu yd, delivered and installed"}]),
            ),
        ]));
        let b = builder(store.clone(), provider);

        let outcome = b.build(&sid, "estimate").await.unwrap();
        assert!(!outcome.queued);
        assert_eq!(
            outcome.usage,
            Usage { input_tokens: 160, output_tokens: 30, ..Default::default() },
            "two calls on an estimate: the money, then the prose"
        );
        assert_eq!(
            v_of(&store, &outcome.document_artifact_id)["lines"][0]["detail"],
            "3 cu yd, delivered and installed",
            "the line says what it covers, so the number beside it is defensible"
        );

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

    /// Isaac's field report, 2026-08-09, end to end: the estimate came out
    /// with six lines — three unpriced scope lines and three bare price lines
    /// under them. The fold happens before the render, so the DOCUMENT is
    /// four lines with the money in the amount column.
    #[tokio::test]
    async fn build_folds_spoken_prices_onto_the_work_they_describe() {
        let (store, sid) = processed_session_with_items(&[
            ("part", "5 yards mulch"),
            ("part", "2 bags compost"),
            ("todo", "redo all garden beds"),
            ("price", "$500 labor"),
            ("price", "$200 mulch"),
            ("price", "$100 compost"),
        ]);
        let ids: Vec<String> =
            store.list_items_for_session(&sid).unwrap().iter().map(|i| i.id.clone()).collect();
        let store = Arc::new(Mutex::new(store));

        // The model prices nothing extra — the beds stay an honest gap.
        let provider =
            Arc::new(MockProvider::new(vec![tool_use("price_items", serde_json::json!({"prices": []}))]));
        let b = builder(store.clone(), provider.clone());

        let outcome = b.build(&sid, "estimate").await.unwrap();
        let store_guard = store.lock().unwrap();
        let art = store_guard.get_artifact(&outcome.document_artifact_id).unwrap();
        let v: serde_json::Value = serde_json::from_str(&art.body).unwrap();
        let lines = v["lines"].as_array().unwrap();

        let titles: Vec<&str> = lines.iter().map(|l| l["title"].as_str().unwrap()).collect();
        assert_eq!(
            titles,
            vec!["5 yards mulch", "2 bags compost", "redo all garden beds", "Labor"],
            "the two price statements that named work folded into it"
        );
        let amount = |item_id: &str| {
            lines.iter().find(|l| l["item_id"] == item_id).unwrap()["amount_cents"].clone()
        };
        assert_eq!(amount(&ids[0]), serde_json::json!(20000), "mulch carries its spoken price");
        assert_eq!(amount(&ids[1]), serde_json::json!(10000), "compost too");
        assert_eq!(amount(&ids[2]), serde_json::Value::Null, "the beds were never priced");
        assert_eq!(
            lines.iter().find(|l| l["item_id"] == ids[2]).unwrap()["is_gap"],
            true,
            "and say so — an unheard price is a gap, never a guess (R4)"
        );
        assert_eq!(amount(&ids[3]), serde_json::json!(50000), "labor prices itself");

        // Only the beds were still open, so only the beds were sent to be
        // priced — the pass can neither cost tokens on settled lines nor
        // contradict the operator.
        let asked = &provider.requests()[0].messages[0].content;
        let asked = format!("{asked:?}");
        assert!(asked.contains("redo all garden beds"));
        assert!(!asked.contains("$200 mulch"), "a spoken price is never re-priced");
        assert!(!asked.contains("Labor"));
    }

    /// The same walk as an INSPECTION (no amount column). Lifting a price out
    /// of a title would delete it from the only place it can appear, so the
    /// fold sits out entirely.
    #[tokio::test]
    async fn an_unpriced_kind_keeps_every_line_and_every_dollar_in_its_title() {
        let (store, sid) =
            processed_session_with_items(&[("part", "5 yards mulch"), ("price", "$200 mulch")]);
        let store = Arc::new(Mutex::new(store));
        let provider =
            Arc::new(MockProvider::new(vec![compose_response(serde_json::json!([]), serde_json::json!([]))]));
        let b = builder(store.clone(), provider.clone());

        let outcome = b.build(&sid, "work_order").await.unwrap();
        let store = store.lock().unwrap();
        let art = store.get_artifact(&outcome.document_artifact_id).unwrap();
        let v: serde_json::Value = serde_json::from_str(&art.body).unwrap();
        let titles: Vec<&str> =
            v["lines"].as_array().unwrap().iter().map(|l| l["title"].as_str().unwrap()).collect();
        assert_eq!(titles, vec!["5 yards mulch", "$200 mulch"]);
        assert_eq!(provider.requests().len(), 1, "the prose pass, never a pricing one");
    }

    /// A walk whose every item priced itself needs no model call at all.
    #[tokio::test]
    async fn a_fully_spoken_walk_skips_the_pricing_call() {
        let (store, sid) = processed_session_with_items(&[
            ("part", "5 yards mulch"),
            ("price", "$200 mulch"),
        ]);
        let store = Arc::new(Mutex::new(store));
        let provider =
            Arc::new(MockProvider::new(vec![compose_response(serde_json::json!([]), serde_json::json!([]))]));
        let b = builder(store.clone(), provider.clone());

        let outcome = b.build(&sid, "estimate").await.unwrap();
        assert!(!outcome.queued, "nothing to ask is not a degrade");
        assert_eq!(
            provider.requests().len(),
            1,
            "the prose pass still runs; the PRICING call is what the operator saved"
        );
        assert!(
            !format!("{:?}", provider.requests()[0].tools).contains("price_items"),
            "every line was priced by the operator"
        );

        let store = store.lock().unwrap();
        let art = store.get_artifact(&outcome.document_artifact_id).unwrap();
        let v: serde_json::Value = serde_json::from_str(&art.body).unwrap();
        let lines = v["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["amount_cents"], 20000);
        assert_eq!(lines[0]["is_gap"], false);
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

    /// A non-pricing kind still never makes a PRICING call — the invariant
    /// that matters, and the one that keeps money off a work order. (It does
    /// now make a prose call: a work order's whole value is the directives,
    /// so "zero calls" stopped being the goal the moment it started carrying
    /// them. The genuinely call-free case is pinned below.)
    #[tokio::test]
    async fn build_non_pricing_kind_never_makes_a_pricing_call() {
        let (store, sid) = processed_session_with_items(&[("todo", "mulch")]);
        let store = Arc::new(Mutex::new(store));
        let provider = Arc::new(MockProvider::new(vec![compose_response(
            serde_json::json!([{"key": "crew", "value": "Jose"}]),
            serde_json::json!([]),
        )]));
        let b = builder(store.clone(), provider.clone());

        let outcome = b.build(&sid, "work_order").await.unwrap();
        assert!(!outcome.queued);
        let tools = format!("{:?}", provider.requests()[0].tools);
        assert!(tools.contains("compose_document"));
        assert!(!tools.contains("price_items"), "a work order is never priced");
        assert_eq!(provider.requests().len(), 1, "one call, not two");
    }

    /// The call-free path: a schema with no walk fields and no `line_detail`
    /// — an operator's own bare list — still costs nothing to build.
    #[tokio::test]
    async fn a_schema_with_no_prose_to_write_makes_zero_calls() {
        let (store, sid) = processed_session_with_items(&[("todo", "mulch")]);
        let mut bare = hoa_schema();
        bare.sections.retain(|s| s.kind == "line_items");
        store.save_document_schema(&bare).unwrap();
        let store = Arc::new(Mutex::new(store));
        let provider = Arc::new(MockProvider::new(vec![]));
        let b = builder(store.clone(), provider.clone());

        let outcome = b.build(&sid, "hoa_addendum").await.unwrap();
        assert!(!outcome.queued);
        assert_eq!(outcome.usage, Usage::default());
        assert!(provider.requests().is_empty(), "nothing to write, nothing to spend");

        let store = store.lock().unwrap();
        assert!(
            store.list_llm_usage_for_session(&sid).unwrap().iter().all(|r| r.purpose != "document"),
            "and no 'document'-purpose usage row"
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

    /// Empty-walk guard (field fix, jefe-2026-07-24): a Processed session with
    /// ZERO items must NOT mint a document — it produced the blank "work order"
    /// ghost. `build` now skips truthfully (InvalidState) and, crucially, mints
    /// NO artifact and makes NO LLM call. (Replaces the prior test that treated
    /// the zero-item path as a silent success minting an all-dashes document.)
    #[tokio::test]
    async fn build_empty_processed_session_is_skipped_without_minting_an_artifact() {
        let (store, sid) = processed_session_with_items(&[]);
        let store = Arc::new(Mutex::new(store));
        let provider = Arc::new(MockProvider::new(vec![]));
        let b = builder(store.clone(), provider.clone());

        let err = b.build(&sid, "estimate").await.unwrap_err();
        assert!(
            matches!(err, CoreError::InvalidState(_)),
            "an empty walk skips the build instead of minting a blank document: {err:?}"
        );
        assert!(provider.requests().is_empty(), "no items -> no pricing/fill call at all");

        // Regression: the guard mints NOTHING — no ghost document artifact, and
        // no document number was burned.
        let store = store.lock().unwrap();
        let docs: Vec<_> = store
            .list_artifacts_for_session(&sid)
            .unwrap()
            .into_iter()
            .filter(|a| a.kind == "document")
            .collect();
        assert!(docs.is_empty(), "empty walk must not persist a document artifact");
    }

    /// The stated grand total never reaches the pricing prompt (Plan 13 D5a's
    /// hint, retired 2026-08-10 — see the call site). It was an instruction to
    /// allocate, and allocation is invention: across four real runs of the
    /// same walk it put the operator's whole $1,250 quote onto a line he never
    /// priced, once. The capture itself is untouched and still readable.
    #[tokio::test]
    async fn the_stated_total_never_reaches_the_pricing_prompt() {
        let (store, sid) =
            processed_session_with_items(&[("todo", "mulch"), ("safety", "loose railing")]);
        let a1 = store.list_items_for_session(&sid).unwrap()[0].id.clone();
        store
            .add_artifact(
                &sid,
                "session_meta",
                "meta",
                &serde_json::json!({"spoken_total_cents": 120000}).to_string(),
            )
            .unwrap();
        let store = Arc::new(Mutex::new(store));

        let provider = Arc::new(MockProvider::new(vec![
            tool_use(
                "price_items",
                serde_json::json!({"prices": [{"item_id": a1, "amount_cents": 95000}]}),
            ),
            compose_response(serde_json::json!([]), serde_json::json!([])),
        ]));
        let b = builder(store.clone(), provider.clone());
        b.build(&sid, "estimate").await.unwrap();

        let reqs = provider.requests();
        let ContentBlock::Text { text } = &reqs[0].messages[0].content[0] else {
            panic!("expected text content");
        };
        assert!(!text.contains("1200.00"), "the total must not reach the pricing prompt: {text}");
        assert!(!text.to_lowercase().contains("total"), "nor any framing of it: {text}");

        // The capture still works — it is honest data, and a future ceiling
        // check is a real use that does not create prices.
        let guard = store.lock().unwrap();
        let meta = guard
            .list_artifacts_for_session(&sid)
            .unwrap()
            .into_iter()
            .find(|a| a.kind == "session_meta")
            .expect("the artifact is still written");
        assert!(meta.body.contains("120000"));
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

    /// The golden: for each of the 7 built-ins, the decoded payload's shared
    /// fields equal the values `is_pricing_kind`/`total_shape` declare — the
    /// hardcoded parity reference in `pipeline/mod.rs` that the seeded rows
    /// must never silently drift from. `id`/`doc_number` excluded (they are
    /// non-deterministic by design).
    ///
    /// Since 2026-08-09 every built-in also writes prose, so each build here
    /// scripts one compose response; the per-line `detail` it returns is
    /// asserted too, because a document whose lines lost their second line
    /// would still pass every other check in this test.
    #[tokio::test]
    async fn builtin_output_matches_the_pricing_and_total_reference() {
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
            // Pricing kinds get a scripted price on item 1; every built-in
            // then gets its one compose call, writing a second line onto
            // item 1 only — so the test also proves a line the model said
            // nothing about keeps an empty detail rather than inventing one.
            let mut responses = Vec::new();
            if pricing {
                responses.push(tool_use(
                    "price_items",
                    serde_json::json!({"prices": [{"item_id": ids[0], "amount_cents": 28500}]}),
                ));
            }
            responses.push(compose_response(
                serde_json::json!([]),
                serde_json::json!([{"item_id": ids[0], "detail": "from the yard"}]),
            ));
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
            let expected: Vec<(&str, &str, Option<i64>, bool, &str)> = if pricing {
                vec![
                    ("order lumber", "", Some(28500), false, "from the yard"),
                    ("bark mulch", "", None, true, ""),
                ]
            } else {
                vec![
                    ("order lumber", "", None, false, "from the yard"),
                    ("bark mulch", "", None, false, ""),
                ]
            };
            for ((line, item_id), (title, qty, amount, is_gap, detail)) in
                lines.iter().zip(&ids).zip(expected)
            {
                assert_eq!(line["title"], title, "{kind}");
                assert_eq!(line["detail"], detail, "{kind}: written, or honestly empty");
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
            hint: None,
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
                    line_detail: String::new(),
                    fields: vec![],
                },
                SchemaSection {
                    key: "approvals".into(),
                    kind: "filled".into(),
                    label: "Approvals".into(),
                    priced: false,
                    line_detail: String::new(),
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
                    line_detail: String::new(),
                    fields: vec![SchemaField {
                        key: "terms_body".into(),
                        kind: "static".into(),
                        label: "Terms".into(),
                        fill: "static".into(),
                        static_value: Some("Valid for 30 days.".into()),
                        hint: None,
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
        tool_use("compose_document", serde_json::json!({ "fields": fields }))
    }

    /// A compose response that also writes the lines.
    fn compose_response(
        fields: serde_json::Value,
        lines: serde_json::Value,
    ) -> harness::CompletionResponse {
        tool_use("compose_document", serde_json::json!({ "fields": fields, "lines": lines }))
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
    async fn compose_echoes_and_validates_and_drops_unknown_keys() {
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
        let map = compose_document(
            &provider, "HOA Addendum", "", &fields, &[], "summary", &[], 512, &mut usage,
        )
        .await
        .unwrap()
        .fields;
        assert_eq!(map.get("hoa_no").map(String::as_str), Some("41827"), "first-wins dedup");
        assert_eq!(map.get("reviewed_by").map(String::as_str), Some("Dana"));
        assert_eq!(map.get("gate_code"), None, "hallucinated key dropped");
        assert_eq!(map.len(), 2);
        assert_eq!(usage, Usage { input_tokens: 80, output_tokens: 15, ..Default::default() }, "R9: usage accumulated");
    }

    /// R6 + the exact compose prompt: items (via `format_pricing_items`
    /// verbatim — no `right_text`), the coordination notes, and the summary
    /// reach the request; the transcript never does.
    #[tokio::test]
    async fn compose_fed_items_notes_and_summary_never_the_transcript() {
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
        compose_document(
            &dyn_provider,
            "HOA Addendum",
            "",
            &fields,
            &items,
            "Walked the front yard; HOA approval 41827 on file.",
            &[NotesEntry {
                bucket: "constraints".into(),
                label: "Gate".into(),
                detail: "Code 4412, park on the street.".into(),
            }],
            512,
            &mut usage,
        )
        .await
        .unwrap();

        let reqs = provider.requests();
        let ContentBlock::Text { text } = &reqs[0].messages[0].content[0] else {
            panic!("expected text content");
        };
        let expected = "Write this HOA Addendum.\n\
                        \n\
                        Fields to write:\n\
                        - [hoa_no] HOA approval #\n\
                        - [reviewed_by] Reviewed by\n\
                        \n\
                        The items on it:\n\
                        - [todo] Install boxwood hedge (item_id: item-A)\n\
                        - [part] bark mulch (item_id: item-B)\n\
                        \n\
                        What was said around the work:\n\
                        constraints:\n\
                        - Gate: Code 4412, park on the street.\n\
                        \n\
                        Session summary:\n\
                        Walked the front yard; HOA approval 41827 on file.";
        assert_eq!(
            text, expected,
            "the exact compose user message — right_text is absent (format_pricing_items \
             omits it), and the coordination notes ride along at no extra call"
        );
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
            hint: None,
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

    // ---- What the compose pass is FOR ------------------------------------

    /// The work order Isaac asked for: "if during my walk I say Jose is gonna
    /// do X, Michael is gonna do Y, that should be mentioned."
    #[tokio::test]
    async fn a_work_order_carries_directives_and_the_names_against_them() {
        let (store, sid) = processed_session_with_items(&[
            ("todo", "mulch the front beds"),
            ("todo", "rebuild the back fence panel"),
        ]);
        let ids: Vec<String> =
            store.list_items_for_session(&sid).unwrap().into_iter().map(|i| i.id).collect();
        let store = Arc::new(Mutex::new(store));
        let provider = Arc::new(MockProvider::new(vec![compose_response(
            serde_json::json!([{"key": "crew", "value": "Jose, Michael"}]),
            serde_json::json!([
                {"item_id": ids[0], "detail": "Strip the old bark first. Watch the irrigation heads.",
                 "assignee": "Jose"},
                {"item_id": ids[1], "detail": "Pull the cracked panel; match the existing pickets.",
                 "assignee": "Michael"}
            ]),
        )]));
        let b = builder(store.clone(), provider);

        let outcome = b.build(&sid, "work_order").await.unwrap();
        let v = decoded_document(&store, &outcome.document_artifact_id);
        let lines = v["lines"].as_array().unwrap();
        assert_eq!(lines[0]["detail"], "Strip the old bark first. Watch the irrigation heads.");
        assert_eq!(lines[0]["assignee"], "Jose");
        assert_eq!(lines[1]["assignee"], "Michael");
        assert_eq!(lines[0]["amount_cents"], serde_json::Value::Null, "still no money");
        assert_eq!(v["fields"][0]["value"], "Jose, Michael");
    }

    /// An assignee is only ever read on a document written FOR a crew. A
    /// model that volunteers one on an estimate is answering a question
    /// nobody asked, and a name in the price column would be a rendering bug
    /// wearing a data bug's clothes.
    #[tokio::test]
    async fn an_estimate_never_takes_an_assignee() {
        let (store, sid) = processed_session_with_items(&[("todo", "mulch the front beds")]);
        let id = store.list_items_for_session(&sid).unwrap()[0].id.clone();
        let store = Arc::new(Mutex::new(store));
        let provider = Arc::new(MockProvider::new(vec![
            tool_use("price_items", serde_json::json!({"prices": []})),
            compose_response(
                serde_json::json!([]),
                serde_json::json!([
                    {"item_id": id, "detail": "3 cu yd, delivered", "assignee": "Jose"}
                ]),
            ),
        ]));
        let b = builder(store.clone(), provider.clone());

        let outcome = b.build(&sid, "estimate").await.unwrap();
        let v = decoded_document(&store, &outcome.document_artifact_id);
        assert_eq!(v["lines"][0]["detail"], "3 cu yd, delivered", "the inclusion still lands");
        assert_eq!(v["lines"][0]["assignee"], serde_json::Value::Null, "the name does not");
        assert!(
            !format!("{:?}", provider.requests()[1].tools).contains("assignee"),
            "and the tool never offered the field, so there was nothing to volunteer"
        );
    }

    /// A line the pass had nothing real to say about keeps the operator's own
    /// words and no second line. Padding every line is how a document starts
    /// padding with something false.
    #[tokio::test]
    async fn a_line_the_pass_declined_gets_no_invented_detail() {
        let (store, sid) =
            processed_session_with_items(&[("todo", "mulch the beds"), ("todo", "check the gate")]);
        let ids: Vec<String> =
            store.list_items_for_session(&sid).unwrap().into_iter().map(|i| i.id).collect();
        let store = Arc::new(Mutex::new(store));
        let provider = Arc::new(MockProvider::new(vec![compose_response(
            serde_json::json!([]),
            serde_json::json!([
                {"item_id": ids[0], "detail": "Front beds only."},
                {"item_id": "hallucinated-id", "detail": "invented"},
                {"item_id": ids[0], "detail": "a second bite"}
            ]),
        )]));
        let b = builder(store.clone(), provider);

        let outcome = b.build(&sid, "work_order").await.unwrap();
        let v = decoded_document(&store, &outcome.document_artifact_id);
        assert_eq!(v["lines"][0]["detail"], "Front beds only.", "first-wins dedup");
        assert_eq!(v["lines"][1]["detail"], "", "declined -> empty, never filler");
        assert_eq!(v["lines"].as_array().unwrap().len(), 2, "and no line was added");
    }

    /// The coordination notes the walk already produced reach the pass — the
    /// gate code a crew needs is in `constraints`, not in the item list, and
    /// it costs nothing to carry.
    #[tokio::test]
    async fn the_walks_notes_reach_the_compose_pass() {
        let (store, sid) = processed_session_with_items(&[("todo", "mulch the beds")]);
        store
            .add_artifact(
                &sid,
                "notes",
                "notes",
                &crate::pipeline::notes::serialize_buckets(&[NotesEntry {
                    bucket: "constraints".into(),
                    label: "Access".into(),
                    detail: "Gate code 4412; dog in the back yard until eight.".into(),
                }]),
            )
            .unwrap();
        let store = Arc::new(Mutex::new(store));
        let provider = Arc::new(MockProvider::new(vec![compose_response(
            serde_json::json!([]),
            serde_json::json!([]),
        )]));
        let b = builder(store.clone(), provider.clone());
        b.build(&sid, "work_order").await.unwrap();

        let asked = format!("{:?}", provider.requests()[0].messages[0].content);
        assert!(asked.contains("Gate code 4412"), "the crew's own gate code reached the writer");
        assert!(asked.contains("dog in the back yard"));
    }

    /// A move-out report is a MONEY document: the deduction total is the
    /// whole point, and the label says what the sum means.
    #[tokio::test]
    async fn a_move_out_report_prices_its_deductions() {
        let (store, sid) = processed_session_with_template(
            Some("property"),
            &[("todo", "carpet stain, bedroom 1"), ("todo", "blinds, kitchen")],
        );
        let ids: Vec<String> =
            store.list_items_for_session(&sid).unwrap().into_iter().map(|i| i.id).collect();
        let store = Arc::new(Mutex::new(store));
        let provider = Arc::new(MockProvider::new(vec![
            tool_use(
                "price_items",
                serde_json::json!({"prices": [{"item_id": ids[0], "amount_cents": 14000}]}),
            ),
            compose_response(serde_json::json!([]), serde_json::json!([])),
        ]));
        let b = builder(store.clone(), provider);

        let outcome = b.build(&sid, "move_out").await.unwrap();
        let v = decoded_document(&store, &outcome.document_artifact_id);
        assert_eq!(v["total_label_key"], "deposit_deduction", "the sum says what it is");
        assert_eq!(v["lines"][0]["amount_cents"], 14000);
        assert_eq!(
            v["lines"][1]["is_gap"], true,
            "an unpriced deduction is an open question, not a free pass"
        );
    }

    /// "$0" on a client's estimate states a price nobody committed to.
    #[tokio::test]
    async fn a_zero_price_is_no_price_not_a_free_line() {
        let (store, sid) =
            processed_session_with_items(&[("todo", "mulch three beds"), ("todo", "prune")]);
        let ids: Vec<String> =
            store.list_items_for_session(&sid).unwrap().into_iter().map(|i| i.id).collect();
        let store = Arc::new(Mutex::new(store));
        let provider = Arc::new(MockProvider::new(vec![
            tool_use(
                "price_items",
                serde_json::json!({"prices": [
                    {"item_id": ids[0], "amount_cents": 30000},
                    {"item_id": ids[1], "amount_cents": 0}
                ]}),
            ),
            compose_response(serde_json::json!([]), serde_json::json!([])),
        ]));
        let b = builder(store.clone(), provider);

        let outcome = b.build(&sid, "estimate").await.unwrap();
        let v = decoded_document(&store, &outcome.document_artifact_id);
        assert_eq!(v["lines"][0]["amount_cents"], 30000);
        assert_eq!(
            v["lines"][1]["amount_cents"],
            serde_json::Value::Null,
            "a zero is the model having no price — not a free line"
        );
        assert_eq!(v["lines"][1]["is_gap"], true, "so it stays an honest gap");
    }

    /// A flat output budget is a cliff for a pass that writes prose: the
    /// model runs out mid-tool-call, the JSON truncates, and the document
    /// degrades to all-gaps — silently, and only on the longest walks.
    #[test]
    fn the_compose_budget_grows_with_the_document() {
        // A four-item walk with one field is small; the caller's floor holds.
        assert_eq!(compose_budget(1024, 1, 4, true), 1024);
        // A thirty-item work order with four fields needs real room — under a
        // flat 1024 this is the walk that would have come back empty.
        assert!(compose_budget(1024, 4, 30, true) > 2000);
        // No per-line writing, no per-line budget.
        assert!(compose_budget(256, 1, 30, false) < compose_budget(256, 1, 30, true));
        // Bounded: a runaway item list cannot authorize an unbounded spend.
        assert_eq!(compose_budget(1024, 4, 100_000, true), 8192);
        // And never LESS room than the caller asked for.
        assert!(compose_budget(4096, 1, 1, false) >= 4096);
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
        // One item so the empty-walk guard (jefe-2026-07-24) doesn't skip the
        // build — the trace here is about NUMBERING only. Fill fields are
        // cleared below; pricing degrades harmlessly on the empty provider
        // (R7 queued), which does not affect doc_number/number_prefix.
        let (store, sid) = processed_session_with_items(&[("todo", "mulch")]);
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
    async fn a_work_order_emits_its_assignment_block_and_todays_prefix() {
        let (store, sid) = processed_session_with_items(&[("todo", "mulch")]);
        let store = Arc::new(Mutex::new(store));
        let provider = Arc::new(MockProvider::new(vec![compose_response(
            serde_json::json!([
                {"key": "crew", "value": "Jose, Michael"},
                {"key": "access", "value": "Gate code 4412."}
            ]),
            serde_json::json!([]),
        )]));
        let b = builder(store.clone(), provider.clone());
        let outcome = b.build(&sid, "work_order").await.unwrap();
        let v = decoded_document(&store, &outcome.document_artifact_id);

        // The four fields a crew standing at the gate actually needs, in
        // schema order — two written, two honestly blank.
        let keys: Vec<&str> =
            v["fields"].as_array().unwrap().iter().map(|f| f["key"].as_str().unwrap()).collect();
        assert_eq!(keys, vec!["crew", "schedule", "access", "safety"]);
        assert_eq!(v["fields"][0]["value"], "Jose, Michael");
        assert_eq!(v["fields"][0]["label"], "Assigned to");
        assert_eq!(v["fields"][1]["is_gap"], true, "no date was said — never invented");
        assert_eq!(v["fields"][2]["value"], "Gate code 4412.");
        assert_eq!(v["fields"][3]["is_gap"], true, "no hazards were said");
        assert_eq!(v["number_prefix"], "WO", "today's Swift-side prefix, now also in the body");
    }
}
