# Murmur Rust Core — Plan 13: notes-first core

> **For agentic workers:** REQUIRED SUB-SKILL: use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax. The Rust tasks (Stage 1 + Stage 2 core/ffi) are **hermetic**: in-memory `Store`, `MockProvider`, `with_providers`/`SessionProcessor`/`DocumentBuilder` — **no model, no `whisper` feature, no network, no camera, no real-photo filesystem**. `cargo test --workspace` must NEVER require the `whisper` feature or a model file (the load-bearing CI invariant). The Swift tasks are **not CI-gated** (real-core-only, needs the gitignored `MurmurCoreFFI` xcframework) and the **notes-screen visual/grouping design is explicitly sac's** — `// sac:` markers throughout. Run `cargo`/`xcodegen` **inside** the Nix dev shell; run `xcodebuild` **outside** it (Nix linker env breaks Xcode `ld`). Never read `.env` or `project.local.yml`.
>
> **⚠ SHIPPABILITY — merging any task on this plan auto-publishes the TestFlight internal lane on real-engine** (CANON 2026-07-10). main must build the **real-core archive** at every merge. That is why this plan is split into **two mergeable stages**: Stage 1 is purely **additive** (new `build_document` path, nothing removed — old `finish()` still returns a document); Stage 2 is the **atomic flip** (finish→notes across Rust *and* Swift in one PR). Never merge a task that leaves the real-core archive un-compilable. See **§Staging**.
>
> **Plan review (2026-07-10) — APPROVE WITH CONDITIONS (applied).** Adversarial review recomputed the D7 terminal-state claim (documents build only from `Processed`, `Processed` is terminal → `clear_authoritative_outputs` never races a live document) — **HELD**; staging boundary clean; evals clean. Three conditions folded in: **C1** — the structure render's `is_gap` default must NOT naively delegate to `is_pricing_kind`: that would flip a *degraded/offline* non-pricing document to looks-confirmed. Gap policy is now an **explicit parameter** (`GapPolicy::PerPricingKind` for the on-demand build; `GapPolicy::AllGap` for the offline fallback — "amount not yet priced" ≠ "nothing has been through the LLM"). **N2** — the on-demand render uses a fresh `new_id()` per line (the `build_document`-tool convention, `line.id != item_id`); the offline fallback keeps its legacy `line.id = item.id`; the cross-check waives the `id` field. **N3** — `doc_kind_for_template(t)` is redefined as `doc_kinds_for_template(t)[0]` (property now → `condition`, pinned by test); `NotesPayload.doc_kind` is **advisory**, and Swift button wiring keys off the **client-known template**, not the payload field. dam's three answers folded: doc numbers **burn per tap** (confirmed); `session_transcript` getter **deferred** (future work, not an open question); the **spoken grand total IS threaded** into the pricing pass as one scalar hint captured at summary time (D5a) — never by reopening transcript access in the pricing prompt.

**Goal.** Implement the ADOPTED notes-first pivot (`meta/CANON.md` 2026-07-10; `docs/design/2026-07-10-decisions-notes-first.md`; sac's #189 design `docs/design/2026-07-09-notes-first-output.md` + `docs/design/notes-mockup.html`; dam's ruling on PR #189). A walk's finish output becomes **notes** (items + summary — the payload `finish()` already computes); the finished document stops being auto-built at DONE and becomes an **on-demand** `build_document(kind)` engine call powering the trade's action buttons.

**What lands:**

1. **murmur-core (pipeline) — drop phase B from `process()`.** `SessionProcessor::process()` stops making the forced `build_document` call. `ProcessOutcome.document_artifact_id` is removed. Extraction + summary + the finish swap are unchanged; a successful `process()` now costs one fewer LLM call. (Stage 2.)
2. **murmur-core (pipeline) — new `DocumentBuilder`.** An on-demand builder: validate the session is `Processed` + the `kind` is legal for the template; **deterministically render** document structure from the session's authoritative items (no LLM); for **pricing kinds** (estimate/invoice) run **one focused pricing pass** whose input is the items only (never the transcript — R6); attach validated per-item amounts (Plan 12's echo-and-validate, reused); mint the doc number and persist the document as an immutable **snapshot** artifact. LLM failure degrades to an unpriced structure-only document — never a hard failure (R7). (Stage 1, additive.)
3. **murmur-core — pricing seam.** A `price_items(...)` function (LLM impl now) behind which a future price-book lookup slots (lookup-first, LLM-fallback). Seam defined, price-book NOT built. (Stage 1.)
4. **ffi — new `MurmurEngine::build_document(session_id, kind)`** (async, throwing) returning the existing `DocumentPayload` (read back via `convert::document_payload`). **`WalkSession::finish()` changes its return type** from `DocumentPayload` to a new `NotesPayload` record (items + summary + queued). (build_document = Stage 1; finish flip = Stage 2.)
5. **apps/ios — seam + routing.** `WalkEngine.finish()` returns a new `NotesModel`; `WalkEngine.buildDocument(sessionId:kind:)` is added. `MurmurEngine` + `DemoWalkEngine` conform (scripted notes + canned document). `AppModel` routes DONE → a Notes screen; the action buttons call `buildDocument` → the **existing `ReviewView`** (unchanged, now reached deliberately). The Notes screen itself is a minimal seam behind `// sac:` markers — sac owns its visuals. (Stage 2.)

**What this plan is NOT (see Non-goals for the full list).** No price-book store (seam only). No PDF export changes. No notes **editing**. No per-trade button-set **content** or notes-screen visual design (sac's). No follow-ups system. No vision. No new SQLite migration (documents remain `artifacts.body` JSON — additive).

---

## Hard dependencies (all DONE, on `main`)

- **Plan 07** (`crates/ffi`, `crates/murmur-core`): the structured document is an `Artifact` with `kind="document"` + a **JSON body** (`store/documents.rs`, `mint_document_number_and_add_artifact`) — *no domain type, no migration*. `DocLine`/`DocumentPayload` uniffi records + `convert::document_payload` parse the body **defensively field-by-field** (`convert.rs:36–69`); `partial_document_from_items` (`convert.rs:79–116`) is the item→line fallback. Per-`doc_kind` number minting (`store/documents.rs`). The **empty-session contract**: empty/whitespace transcript → `process()` short-circuits, summary `"(empty session)"`, no document, no panic (`pipeline/mod.rs:134–143`).
- **Plan 06a** (`items.source`): `ItemSource { Live, Authoritative, Manual }`. After `finish_session_processed`, the surviving board = this-run **authoritative** + **manual** items (`sessions.rs:404–442`). Phase 0 `clear_authoritative_outputs` sweeps prior authoritative items + **all artifacts** before a (re)process (`sessions.rs:466–485`) — only reachable from `AwaitingProcessing|Failed`.
- **Plan 12** (`DocLine.item_id`): the document already carries an **optional per-line `item_id`** validated by **echo-and-validate / first-wins dedup / degrade-to-None** (`pipeline/tools.rs:338–388`); the offline fallback carries `item_id` trivially (it builds rows straight from items). `convert::document_payload` reads `item_id` (`convert.rs:53`). **The pricing pass in this plan reuses this exact echo-validate pattern for amounts.**
- **Plan 11** (`photos`): `count_live_photos_by_item_for_session` (batched per-item photo counts) already feeds the board snapshot (`ffi/src/session.rs:372–409`); the notes payload reuses it.

**Verified API facts (checked against source, not guessed):**
- **No migration.** Documents live in `artifacts.body` JSON; `document_payload` tolerates any missing field. Adding `NotesPayload` (a new uniffi record) and a new engine method is additive — `MIGRATIONS.len()` unchanged, no `user_version` bump.
- **`process()` phase B is self-contained.** `run_build_document` (`pipeline/mod.rs:292–397`) is the ONLY document-writing path in `process()`; `run_llm_phases` returns `(summary, Option<String> document_artifact_id)` (`mod.rs:217, 285`); `ProcessOutcome.document_artifact_id` (`mod.rs:47`) is read only by `WalkSession::finish` (`ffi/src/session.rs:674–694`). Removing phase B touches exactly these.
- **`finish()` return is `DocumentPayload`** (`ffi/src/session.rs:632`), mapped to `DocumentModel` by `MurmurEngine.document(_:)` (`MurmurEngine.swift:378`), stored on `AppModel.document` (`AppModel.swift:95`), rendered by `ReviewView`. The Swift seam is `WalkEngine.finish() async -> DocumentModel` (`WalkEngine.swift:89`).
- **Doc-number minting is per-`doc_kind`** and lazy (`store/documents.rs:53`); `mint_document_number_and_add_artifact` mints + writes in one tx (a number is consumed iff the artifact lands).
- **Evals do NOT pin finish-time document behavior.** Only live-prompt + swap-at-finish board plumbing pins exist (`crates/evals/tests/live_prompt_pins.rs`, `carried_scenarios.rs`) — verified: no transcript→document score test. Dropping phase B moves no eval.
- `doc_kind_for_template` (`pipeline/mod.rs:30`) maps template→a single default kind; this plan generalizes it to an allow-list.

**Spec basis:** R6 (under-extraction bias — the pricing pass must not author lines), R7 (never lose the artifact — a document always lands, pricing failure degrades), R9 (spend meter — per-tap cost is proportional and logged). Design: notes-first (#189), Q1/Q2/Q6/Q7 rulings on the PR thread.

---

## Architecture — decisions, justified (reviewers read these first)

### D1. `finish()` = notes only; the document build moves to an explicit `build_document(kind)`

`finish()` today extracts items + summary **and** forces a document build (phase B). The pivot removes phase B: `finish()` returns a `NotesPayload` (the items + summary it already computed) and nothing more. Documents are built later, deliberately, by a new **engine-keyed** method (not `WalkSession`-scoped — the walk is over and the `WalkSession` is dropped; documents are generated from the notes screen, possibly after relaunch/from history):

```
MurmurEngine::build_document(session_id: String, kind: String) async throws -> DocumentPayload
```

**Why engine-keyed, not session-keyed:** `MurmurEngine.finish()` nils out its `WalkSession` handle (`MurmurEngine.swift:197`); a later tap has no session object. Photos already solved this — `add_photo` is engine-keyed on `session_id` (`ffi/src/photos.rs`). `build_document` follows that precedent.

**Why plain async return, not a `DocumentReady` event:** the caller (a button tap) has a direct `await` result — the simplest shape. No new `WalkEvent` variant, no listener wiring. (An event flow would only pay off for background/batch generation, which is a non-goal.)

### D2. `NotesPayload` — the exact finish record

```rust
// crates/ffi/src/notes.rs  (new; uniffi::Record)
pub struct NotesPayload {
    pub session_id: String,
    pub doc_kind: String,        // template's DEFAULT kind (estimate|report|inspection) — for button curation only
    pub summary: String,         // session.summary; "(empty session)" for a silent walk
    pub items: Vec<BoardItem>,   // authoritative+manual board post-swap, with batched photo_count (reuse BoardItem)
    pub queued: bool,            // true = finish degraded offline (D9); session NOT Processed → buttons disabled
}
```

- **`items` reuses `BoardItem`** (`events.rs:10` — id/kind/text/right/photo_count). No new item record; the notes screen groups by `kind` client-side (sac's trade-aware buckets). Photo counts come from the same batched `count_live_photos_by_item_for_session` the board snapshot uses.
- **Transcript is NOT in the payload.** The Swift side already accumulates the full transcript from `transcriptCommitted` events during the walk (`AppModel`); the notes screen's "show what I heard" row reads that. Adding it to the payload would duplicate state. (Noted as an open question if the history screen — which has no live event stream — needs it; a `session_transcript(session_id)` getter is the trivial future add.)
- **`doc_kind`** is the template's default kind (`doc_kinds_for_template(template)[0]`), carried as an **advisory** hint only. **Swift button wiring keys off the client-known template** (`AppModel` already holds the `TradeFixture`/template it began the walk with), NOT off `NotesPayload.doc_kind` — so the property mismatch (default `condition` vs the legacy `report` copy) never drives UI. The real per-trade taxonomy is sac's.

### D3. Empty-notes contract (maps Plan 07's empty-session contract onto notes)

Plan 07: silent walk → truthful empty document (queued=false, doc_number=0, no panic). The notes equivalent:

| finish scenario | NotesPayload |
|---|---|
| empty/whitespace transcript (process short-circuits) | `summary:"(empty session)"`, `items: <manual items only, usually []>`, `queued:false` |
| offline / LLM-down (process `Err`) — D9 | `summary:""`, `items: <current live board>`, `queued:true` |
| double-`finish()` / post-`cancel()` degrade | `summary: <stored summary or "">`, `items: <current board>`, `queued:false` |
| normal | `summary:<computed>`, `items:<authoritative+manual>`, `queued:false` |

No branch panics across FFI (finish returns a bare `NotesPayload`, not a `Result` — same posture as today's `DocumentPayload`).

### D4. Deterministic structure render — items are the source of truth

The document **structure** is rendered in Rust with **no LLM**: one line per authoritative item, in item order.

```
line = { id: new_id(), title: item.text, detail:"", qty:"",
         amount_cents: None, section: None, is_gap: <GapPolicy>, item_id: Some(item.id) }
```

- **`item_id` is carried directly** from the item. Plan 12's fallback path `partial_document_from_items` is the *prototype* for the item→line mapping; the shared core render is `render_structure_document(doc_kind, items, gap: GapPolicy)`. The **on-demand build** uses it; the **FFI offline fallback** keeps its own `DocLine` construction (it sets `line.id = item.id`, a legacy shortcut) but must produce **identical field semantics except `id`** — the cross-check test (Task 2) waives the `id` field. The on-demand render uses a fresh `new_id()` per line, matching the `build_document`-tool convention (`line.id != item_id`, asserted in Plan 12).
- **`section: None`** — section/grouping copy is sac's (SCOPE / NEEDS ATTENTION / …); Swift groups by item `kind`. Core stays out of section-copy.
- **`is_gap` is an explicit `GapPolicy` parameter (C1)** — NOT a naive `is_pricing_kind` delegation:
  - `GapPolicy::PerPricingKind` (the on-demand build): `is_gap = is_pricing_kind(doc_kind)` — `estimate`/`invoice` lines are gaps until priced; `report`/`inspection`/`work_order`/`move_out`/`condition` lines are not gaps (a normal finding is not a gap). Reuses the D2a rule.
  - `GapPolicy::AllGap` (the offline/degraded fallback): `is_gap = true` **unconditionally**, for every kind. A degraded document has had **nothing** through the LLM — even an inspection finding is *wholly unconfirmed*, not merely unpriced. "Amount not yet priced" ≠ "nothing has been through the LLM." (This preserves today's `partial_document_from_items` behavior, which sets `is_gap:true` unconditionally by design.)
  - A line that the pricing pass (D5) later prices flips to `is_gap:false` regardless of policy.

**Consequence, stated honestly:** the document is now exactly as rich as the extracted items — no transcript re-derivation. This is the *point* of notes-first (items are authoritative; the document is a view of them) and is what CANON means by "structure re-renders deterministically from the structured items." It is a deliberate quality trade vs today's transcript-authored lines: fewer hallucinated lines, no synthesized rollups, tighter R6. (Richer per-trade line templates are sac's future button-content work — a non-goal here.)

### D5. Pricing pass — items only, amounts only, echo-validated (R6)

For pricing kinds (`estimate`, `invoice`), after the structure render, run **one** focused LLM call:

- **Input = the items only** (`item_id`, `kind`, `text`) + memory prompt (the operator's vocabulary/history, already injected by `build_document_prompt`). **NOT the transcript.** Argument: the pricing model has **no line-authoring power** — its output schema can only attach an amount to an `item_id` that already exists in the structure. Feeding the transcript would reintroduce exactly the transcript→line re-derivation Plan 12 moved away from and reopen the R6 line-inflation surface. Items-only ⇒ **line count is fixed by the deterministic render; the LLM can move only amounts.** (Price genuinely needs domain knowledge; v1 lets the LLM guess from item text + memory — the honest v1 per CANON; the price-book seam D6 slots in front later.)
- **Output schema** (forced tool `price_items`):
  ```json
  { "prices": [ { "item_id": "<id>", "amount_cents": <int> } ], "total_cents": <int, optional> }
  ```
- **Apply** with Plan 12's rules: for each returned price, keep `amount_cents` **iff** `item_id ∈ structure item-id set` and not already claimed (first-wins); unknown/duplicate/absent → the line stays unpriced (amount `None`, `is_gap` per-template). A priced line flips `is_gap:false`. `total_kind:"sum"`.
- **Degrade path (LLM `Err` — offline/down):** skip the pricing pass entirely. The structure-only document (all amounts `None`, `is_gap` per-template) still **mints a number and persists**, with `queued:true` (reused as "document incomplete — pricing did not run"; Swift's existing `queued` note copy — "SAVED OFFLINE — WILL FINISH WHEN YOU RECONNECT" — fits). **Never a hard failure** (R7).

Non-pricing kinds skip the LLM entirely — zero-cost, structure-only, `queued:false`.

### D5a. Spoken grand total — threaded as one scalar hint, captured at summary time (dam answer 3)

If the operator spoke a target total ("keep the whole thing under twelve hundred"), the pricing pass should honor it — **without** reopening transcript access in the pricing prompt (which would break D5's no-line-authoring guarantee). Mechanism, chosen to preserve the zero-migration invariant:

- **Capture at summary time.** The `summarize` pass already legitimately reads the transcript (`prompts::summarize(provider, assembled_transcript, ...)`). Extend its forced `write_summary` tool with an **optional** `spoken_total_cents` field; the model returns it only when a grand total was actually stated (R6: absent when unsure). So transcript access stays confined to the pass that already has it.
- **Persist with no migration.** Store the scalar as a tiny per-session artifact `kind = "session_meta"`, body `{"spoken_total_cents": N}`, written by `process()` on success (the `artifacts` table already exists; `kind` is free-form — "the store doesn't care what `kind` means", `artifacts.rs:24`). No column, no `user_version` bump. `clear_authoritative_outputs` already sweeps all artifacts on reprocess, so a retry re-captures cleanly. Absent field / no meta artifact → no hint.
- **Feed as one scalar.** `DocumentBuilder::build` reads the meta artifact (`session_spoken_total(session_id) -> Option<i64>`) and, for a pricing kind, passes it into `price_items` as a single scalar the prompt renders as one line: *"Operator's stated target total: $1200.00 — allocate line prices consistent with this."* The pricing prompt still receives **items only + this one scalar** — never the transcript. The output schema is unchanged (amounts on existing `item_id`s only), so the hint cannot author lines.

See Worked Example G.

### D6. Pricing seam for the future price book

`price_items` is a free function `async fn price_items(provider, items, memory_prompt, budget) -> Result<HashMap<String,i64>, HarnessError>` (LLM impl now). The future price book slots in as a pre-step: `lookup_prices(items) -> (resolved, unresolved)`, then LLM only for `unresolved`. **Seam = the `HashMap<item_id, cents>` return contract**; nothing about the caller changes. Price-book store is NOT built.

### D7. Doc numbers — mint per generate (burn per tap); documents are immutable snapshots

Each `build_document(kind)` call mints the **next** number for that `doc_kind` (per-kind sequences already independent) and writes a **new** document artifact. Regenerating an estimate ⇒ `EST-0002` (a fresh snapshot), leaving `EST-0001` intact. **0..N documents per session** (Q6/Q7 ruling); regenerate is explicit; a generated snapshot is **never silently mutated** (a sent estimate is a sent estimate).

- **R9 note — numbers burn per tap.** Two taps of Estimate ⇒ EST-0001, EST-0002. Accepted: a document number identifies a *snapshot*; a regenerate genuinely is a new document. Sequences are cheap local integers. (If burn-rate ever matters, a "replace last" that reuses the number is the seam — flagged as an open question, recommend burn-per-tap for v1.)
- **Snapshot vs `clear_authoritative_outputs` on REPROCESS:** `clear_authoritative_outputs` tombstones **all** artifacts, but it runs **only** on a (re)process of an `AwaitingProcessing|Failed` session. Documents are built **only** from a `Processed` session (D8), and `mark_session_processed` makes `Processed` **terminal** (`sessions.rs:194–198`) — there is no user path that reprocesses a Processed session, so **`clear_authoritative_outputs` never races a live document.** The dangling-`item_id` concern (a future "recover this walk" flow re-extracting items under an existing document) is handled by design regardless: **snapshot text is the truth; `item_id` links resolve best-effort at render** (a link to a tombstoned item simply yields no jump/photo-group — `document_payload` already reads `item_id` tolerantly, and the render never fails on a stale id). 
- **Old pre-13 auto-built documents** (built at finish before this plan) remain valid artifacts and **render unchanged** through the same tolerant `document_payload` parser. No migration; the history/notes screen surfaces them via `latest_document_artifact` if present.

### D8. `build_document` validation

- **Session must be `Processed`** — its items are final/authoritative. A non-Processed session (offline notes, `queued:true`) has no authoritative board; `build_document` returns `EngineError` and the Swift buttons stay disabled while `queued`. (An empty-but-Processed session is allowed — it yields a truthful **zero-line** document, D3-parallel; still mints a number, still lands.)
- **`kind` must be legal for the session's template.** New `doc_kinds_for_template(template) -> &'static [&'static str]` allow-list; `is_pricing_kind(kind) -> bool`. Unknown/mismatched kind → `EngineError`.

| template | legal kinds | pricing kinds |
|---|---|---|
| `landscape` | `estimate`, `invoice`, `work_order` | `estimate`, `invoice` |
| `property` | `condition`, `move_out` | — |
| `inspection` | `inspection` | — |

(These are the **kind vocabulary + pricing flags** — core's concern. Which button *leads* and its label copy are sac's.)

### D9. Per-tap cost is logged (R9)

`build_document`'s pricing call logs one `llm_usage` row with `purpose = "document"` (distinct from `"processing"`), so per-tap spend is measured and attributable. Non-pricing kinds log nothing (no call). A degraded (pricing-failed) build logs whatever partial usage the failed call incurred, same posture as `process()`.

---

## Worked examples (reviewers: hand-recompute against the code)

**A — finish() notes payload (happy path, Stage 2).** Template `landscape`. Transcript: *"bark mulch, three cubic yards. call Dev the framer. loose railing on the back deck."*
`process()`: extraction adds A1=`todo "order bark mulch 3 cu yd"`, A2=`safety "loose railing back deck"`; contact Dev; summary `"Ordered mulch; flagged loose railing."`; **no phase B**. `finish_session_processed` swaps → A1,A2 survive.
`finish()` → `NotesPayload{ session_id:S, doc_kind:"estimate", summary:"Ordered mulch; flagged loose railing.", items:[{A1,todo,"order bark mulch 3 cu yd",pc0},{A2,safety,"loose railing back deck",pc0}], queued:false }`.
**Cost:** one folded `"processing"` usage row = extraction + summary tokens **only**; assert `provider.requests()` contains **no** `build_document` request (phase B gone). Finish is strictly cheaper than today.

**B — build_document("estimate") deterministic + pricing (Stage 1).** Session S `Processed`, items A1,A2 as above.
1. validate: `landscape` allows `estimate` ✓; `estimate` is a pricing kind.
2. structure render → `L1{title:"order bark mulch 3 cu yd", item_id:A1, amount:None, is_gap:true}`, `L2{title:"loose railing back deck", item_id:A2, amount:None, is_gap:true}`.
3. pricing pass input = `[{A1,todo,"order bark mulch 3 cu yd"},{A2,safety,"loose railing back deck"}]`; model returns `{prices:[{A1,28500}]}`.
4. echo-validate vs `{A1,A2}`: A1 valid → `L1.amount=28500, is_gap:false`; A2 unpriced → stays gap. `total_kind:"sum"` → total `28500`.
5. mint `estimate` #1 → `doc_number:1`; persist snapshot; `queued:false`. Usage → one `"document"` row.
`DocumentPayload{ doc_kind:"estimate", doc_number:1, lines:[L1 $285.00 item_id A1, L2 gap item_id A2], total sum }`. Swift renders `EST-0001` in the **existing ReviewView**.

**C — pricing LLM fails → structure-only degrade (Stage 1).** As B, pricing provider returns `Err`. Steps 1–2 as above; step 3 fails → skip. Document lands: L1,L2 both amount `None`, `is_gap:true`, **still minted `estimate` #1**, `queued:true`. No error thrown (R7). Any partial pricing-call usage logged.

**D — build_document("work_order") non-pricing (Stage 1).** `landscape` allows `work_order` (non-pricing). Structure render only, **no LLM call**: L1,L2 amount `None`, `is_gap:false` (non-pricing default), mint `work_order` #1, `queued:false`, zero usage rows.

**E — doc numbers burn per tap; independent sequences (Stage 1).** `build_document(S,"estimate")`→EST #1. Again→EST #2 (new snapshot; #1 intact; 2 estimate artifacts for S). `build_document(S,"invoice")`→INV #1 (independent sequence). `latest_document_artifact(S)` returns the newest by `id DESC` (INV #1).

**F — empty-but-Processed session (Stage 1 + D3).** Silent walk: `finish()`→`NotesPayload{summary:"(empty session)", items:[], queued:false}`. `build_document(S,"estimate")`: structure render over **zero** items → **zero lines**; pricing pass over zero items (skip the call — nothing to price); mint EST #1; truthful empty document lands, `queued:false`. No panic, no error.

**G — spoken grand total threaded (D5a).** Template `landscape`. Transcript includes *"keep the whole thing under twelve hundred bucks."* plus the mulch/railing content of Ex B.
- `process()`: extraction adds A1,A2 (as B); the `summarize` call returns `summary:"…" , spoken_total_cents:120000`; `process()` writes a `session_meta` artifact `{"spoken_total_cents":120000}`. `finish()` notes are unchanged (`queued:false`) — the scalar does NOT appear in the notes payload.
- `build_document(S,"estimate")`: structure render L1(A1),L2(A2); `session_spoken_total(S)` → `120000`; `price_items` prompt = items `[{A1,…},{A2,…}]` + the scalar line *"stated target total: $1200.00"* (no transcript). Model returns `{prices:[{A1,95000},{A2,25000}]}` summing 120000. Echo-validate vs `{A1,A2}` → both kept; total `sum` = 120000. Mint EST #1; `queued:false`.
- **`is_gap` check (C1):** both priced → `is_gap:false`. Contrast Ex C's degraded path, where the *offline* `AllGap` policy would have set every line `is_gap:true` even for a non-pricing kind.

---

## Staging (main stays shippable at every merge)

Because a merge auto-publishes the **real-core** TestFlight lane, the real-core archive must compile after every PR. The FFI signature of `finish()` cannot change until `MurmurEngine.swift` matches it, so:

- **Stage 1 — additive Rust + FFI (PR-1).** Everything for `build_document`: `DocumentBuilder`, `render_structure_document`, `price_items`, `doc_kinds_for_template`/`is_pricing_kind`, the FFI `MurmurEngine::build_document` export. **`process()`/`finish()` untouched** — old finish still returns `DocumentPayload`. `MurmurEngine.swift` needs no edit (the new FFI method simply goes uncalled). Real-core archive compiles; behavior unchanged; TestFlight ships the identical old flow. Gated by `cargo test`/`clippy` + iOS demo build. *(Tasks 1–4.)*
- **Stage 2 — the atomic flip (PR-2), Rust + Swift in ONE PR.** Drop phase B from `process()`; add `NotesPayload`; change `WalkSession::finish()` to return it; update `MurmurEngine.swift` (`finish→NotesModel`, `buildDocument` mapping), the `WalkEngine` protocol, `DemoWalkEngine` parity, `AppModel` routing (DONE→Notes screen; button→`buildDocument`→ReviewView), and a minimal `NotesView` seam (`// sac:`). The finish-signature change and its Swift consumer land together — the only way the real-core archive stays compilable. Gated by `cargo test`/`clippy` + iOS **demo** build (green because DemoWalkEngine + protocol + AppModel are internally consistent) + **dam's local real-core compile** (`./build-ffi.sh && ./generate.sh && xcodebuild`, since CI can't build real-core) + bindings-drift check. *(Tasks 5–8.)*

Within Stage 1 the Rust tasks are independently `cargo`-testable; Stage 2's Rust tasks (5) are testable before the Swift tasks (6–8) but must **merge together**.

---

## Tasks

### Stage 1 — additive `build_document` (PR-1)

#### Task 1 — `doc_kinds_for_template` + `is_pricing_kind` (core)
- [ ] **RED:** in `crates/murmur-core/src/pipeline/mod.rs` tests, assert `doc_kinds_for_template(Some("landscape"))` == `["estimate","invoice","work_order"]`, `Some("property")`==`["condition","move_out"]`, `Some("inspection")`==`["inspection"]`, `None`→`["report"]`; `is_pricing_kind("estimate")`==true, `is_pricing_kind("work_order")`==false, `is_pricing_kind("inspection")`==false.
- [ ] **RED (N3):** pin `doc_kind_for_template(Some("landscape"))=="estimate"`, `Some("inspection"))=="inspection"`, `None)=="report"`, and **`Some("property"))=="condition"`** — the property value CHANGES from today's `"report"` to `"condition"` (property's own legal-kind list starts with `condition`, not `report`). This only affects the offline fallback's `doc_kind` + the advisory `NotesPayload.doc_kind`; both `condition` and `report` map to `sum`/`total` in `partial_document_from_items` (no total-shape regression), and Swift keys buttons off the template (D2), so the copy switch is unaffected.
- [ ] **GREEN:** add `pub fn doc_kinds_for_template(template: Option<&str>) -> &'static [&'static str]` and `pub fn is_pricing_kind(kind: &str) -> bool`. **Redefine `doc_kind_for_template(t)` as `doc_kinds_for_template(t)[0]`** (N3 — no more `_ => "report"` arm that lands outside property's own list).
- [ ] **Gate:** `nix develop -c cargo test -p murmur-core`.

#### Task 2 — `render_structure_document` with explicit `GapPolicy` (core; C1)
- [ ] **RED (PerPricingKind):** given `[A1 todo, A2 safety]` and `doc_kind:"estimate"`, `GapPolicy::PerPricingKind` → two lines, each `amount_cents:None`, **`is_gap:true`**, `item_id:Some(item.id)`, `section:None`, `title==item.text`; same items + `doc_kind:"inspection"` + `PerPricingKind` → **`is_gap:false`**. Assert line `id != item_id` (fresh `new_id`).
- [ ] **RED (AllGap, C1):** same items + `doc_kind:"inspection"` + **`GapPolicy::AllGap`** → **`is_gap:true`** for every line (a degraded inspection is wholly unconfirmed, not merely unpriced). This is the assertion that stops the naive `is_pricing_kind` delegation from flipping degraded non-pricing docs to looks-confirmed.
- [ ] **RED (cross-check, N2):** `render_structure_document("estimate", items, AllGap)` and today's `partial_document_from_items("estimate", items, true)` produce lines with identical `title`/`detail`/`qty`/`amount_cents`/`section`/`item_id`/`is_gap` — **waiving the `id` field** (on-demand render uses fresh `new_id()`; the offline fallback uses `line.id = item.id`). Assert field-by-field except `id`.
- [ ] **GREEN:** add `pub(crate) enum GapPolicy { PerPricingKind, AllGap }` and `pub(crate) fn render_structure_document(doc_kind: &str, items: &[CapturedItem], gap: GapPolicy) -> Vec<serde_json::Value>` (same field shape `BuildDocumentTool` emits; `is_gap` per the policy). Leave `convert::partial_document_from_items` (ffi) as-is (it keeps `line.id = item.id` and passes the equivalent of `AllGap`) — do NOT force delegation across the crate boundary; the cross-check test is the contract that keeps the two in lockstep.
- [ ] **Gate:** `nix develop -c cargo test -p murmur-core`.

#### Task 3 — `price_items` + `DocumentBuilder::build` (core)
- [ ] **RED (pricing echo-validate):** with a `MockProvider` scripted to return `price_items {prices:[{A1,28500},{bogus,999},{A2,...}], ...}` where only A1,A2 are real and A2 is returned twice, assert the applied prices keep A1=28500, first A2 wins, bogus degrades — mirror Plan 12's `build_document_echoes_and_validates_item_ids` matrix but for amounts.
- [ ] **RED (build happy path B):** construct a store, add A1,A2 as `Authoritative`, `end_and_record_session` + `mark_session_processed`. `DocumentBuilder::build(S,"estimate")` with a scripted pricing response → the persisted `document` artifact has `doc_number:1`, L1 priced/`is_gap:false` with `item_id:A1`, L2 gap. Assert one `"document"` usage row.
- [ ] **RED (degrade C):** pricing provider `Err` → document still lands (mint #1), all amounts `None`, `queued:true`, `build` returns `Ok`. 
- [ ] **RED (non-pricing D):** `build(S,"work_order")` makes **zero** provider calls, lands a structure-only doc, `queued:false`.
- [ ] **RED (validation D8):** `build` on a non-`Processed` session → `CoreError`; `build(S,"estimate")` where template is `inspection` → `CoreError` (illegal kind); empty-but-Processed → zero-line doc, `Ok` (F).
- [ ] **RED (spoken total, D5a / Ex G):** write a `session_meta` artifact `{"spoken_total_cents":120000}` for a Processed session with A1,A2; `build(S,"estimate")` with a scripted pricing response → assert the `price_items` request's user message contains the scalar hint (e.g. `"1200"` / `"120000"`) and does **NOT** contain the transcript. With no meta artifact, the hint line is absent.
- [ ] **GREEN:** add `crates/murmur-core/src/pipeline/document.rs`:
  - `async fn price_items(provider, items: &[CapturedItem], spoken_total_cents: Option<i64>, memory_prompt, max_tokens) -> Result<HashMap<String,i64>, HarnessError>` — one forced `price_items` tool call. User block = the items (id/kind/text) **only** + (if `Some`) one scalar hint line (D5a); NEVER the transcript. Parse `prices[]`, drop entries whose `item_id ∉ items`, first-wins dedup, ignore returned `total_cents` (sum is derived). Provider `Err` propagates.
  - `pub struct DocumentBuilder { provider, store, memory, memory_store, budgets }` with `pub async fn build(&self, session_id: &str, doc_kind: &str) -> Result<BuildDocumentOutcome, CoreError>`:
    1. lock/read session; require `status == Processed` (else `InvalidState`); validate `doc_kind ∈ doc_kinds_for_template(session.template)` (else `InvalidState`).
    2. `items = list_items_for_session(session_id)` (authoritative+manual survivors).
    3. `lines = render_structure_document(doc_kind, &items, GapPolicy::PerPricingKind)`.
    4. if `is_pricing_kind(doc_kind)` and `!items.is_empty()`: `let hint = self.session_spoken_total(session_id)?;` then `match price_items(.., hint, ..).await { Ok(map) => apply(map, &mut lines), Err(_) => queued=true }`; else `queued=false`.
    5. build payload (`doc_kind`, `job_date_unix=session.started_at`, `total_kind:"sum"`/label per kind, `lines`, `queued`) **without** `doc_number`; `mint_document_number_and_add_artifact(session_id, doc_kind, None /* burn per tap */, payload)`.
    6. `record_llm_usage(Some(session_id), "document", &usage)` (only if a call was made).
    7. return `BuildDocumentOutcome { document_artifact_id, usage, queued }`.
  - `apply(map, lines)`: for each line with `item_id` present in `map` (and unclaimed), set `amount_cents`, `is_gap=false`; first-wins.
  - `session_spoken_total(session_id) -> Result<Option<i64>, CoreError>`: read the `session_meta` artifact (kind-scoped), parse `spoken_total_cents`. `None` when absent.
- [ ] **Note (R9/D9):** the pricing prompt is fed **items only + the one optional scalar** (D5/D5a) — the transcript-absence assertion above is load-bearing. Reuse `build_document_prompt`'s memory injection but a new items-only user block + a pricing-specific system prompt in `prompts.rs`.
- [ ] **Gate:** `nix develop -c cargo test -p murmur-core` + `nix develop -c cargo clippy -p murmur-core --all-targets -- -D warnings`.

#### Task 4 — FFI `MurmurEngine::build_document` (ffi, additive)
- [ ] **RED:** in `crates/ffi/src/session.rs` (or a new `document_build.rs`), a `#[tokio::test]` using `with_providers`: begin a walk, append transcript, `finish()` (old path still returns a document here in Stage 1 — that's fine), then `engine.build_document(session_id, "estimate")` returns a `DocumentPayload` with the expected lines; an illegal kind returns `EngineError`.

  ⚠ ordering note: in Stage 1, `finish()` still runs phase B, so a `finish()`ed session already has a document artifact. `build_document` **appends a new** one (burn-per-tap) — assert it reads back **its own** newly-written artifact, not the phase-B one. (After Stage 2 removes phase B, this ambiguity disappears; the test stays valid either way because `build_document` returns the id it just wrote.)
- [ ] **GREEN:** add to `#[uniffi::export(async_runtime = "tokio")] impl MurmurEngine`:
  ```rust
  pub async fn build_document(&self, session_id: String, kind: String) -> Result<DocumentPayload, EngineError> {
      let builder = DocumentBuilder::new(self.providers.processing.clone(), self.store.clone(),
                                         self.memory.clone(), self.memory_store.clone());
      let outcome = builder.build(&session_id, &kind).await.map_err(|e| EngineError::Document(e.to_string()))?;
      let art = { self.store.lock()?.get_artifact(&outcome.document_artifact_id) };
      convert::document_payload(&art?).map_err(|e| EngineError::Document(e.to_string()))
  }
  ```
  Add an `EngineError::Document(String)` variant. Return the payload by reading back exactly the artifact id the builder wrote (never `latest_document_artifact` — burn-per-tap means multiple docs).
- [ ] **Gate:** `nix develop -c cargo test -p ffi` + `clippy`. Regenerate bindings locally (dam) to confirm the new method + no drift on existing records. **Merge PR-1** — real-core archive still compiles (MurmurEngine.swift unchanged).

### Stage 2 — the atomic flip (PR-2, Rust + Swift together)

#### Task 5 — drop phase B; `finish()` returns `NotesPayload` (core + ffi)
- [ ] **RED (core):** update `pipeline/mod.rs` tests: `processes_a_session_end_to_end` no longer scripts a `document_response()`; assert `provider.requests()` has **no** `build_document` request; usage row = extraction + summary only. Remove/retarget `processes_and_builds_a_document_artifact`, `build_document_echoes_real_id_...`, `failed_attempt_does_not_burn_a_document_number` (these pinned phase B — move the still-relevant echo-validate coverage to Task 3's builder tests, which is where document building now lives). `ProcessOutcome` loses `document_artifact_id`.
- [ ] **RED (spoken total capture, D5a):** a session whose transcript states a total → after `process()`, `session_spoken_total(session_id)` == `Some(N)` (a `session_meta` artifact was written); a session with no stated total → `None` (no meta artifact). A `summarize` response omitting `spoken_total_cents` → `None`.
- [ ] **GREEN (core):** delete `run_build_document`; `run_llm_phases` returns just `summary`; `process()` returns `ProcessOutcome { session, usage }`. Drop the `existing_doc_number` read-back + `doc_kind` threading in `process()` (dead once phase B is gone). Extend `prompts::summarize` to also return `Option<i64> spoken_total_cents` (an additive optional field on the `write_summary` tool schema, R6: absent unless clearly stated); `process()` writes it as a `session_meta` artifact **only when `Some`**, inside the success path before/with `finish_session_processed`. Keep `clear_authoritative_outputs` as-is (it already sweeps the `session_meta` artifact on reprocess).
- [ ] **RED (ffi):** in `crates/ffi/src/session.rs`, `finish()` returns `NotesPayload`. Tests: happy path (Worked Ex A) — items+summary+`queued:false`, no `build_document` request; empty transcript → `summary:"(empty session)", items:[], queued:false` (retarget the existing empty-session finish test); offline degrade (process `Err`) → `queued:true` with the live board; double-finish → degraded notes, no panic.
- [ ] **GREEN (ffi):** add `crates/ffi/src/notes.rs` (`NotesPayload` uniffi record + a `convert::notes_payload(session, items, photo_counts, queued)` builder). Rewrite `WalkSession::finish()` to return `NotesPayload`: on `Ok(outcome)` build notes from `list_items_for_session` + the batched photo counts + `outcome.session.summary`; on the empty/degrade branches follow the D3 table. Replace the `partial_document`/`degraded_document` document helpers with notes equivalents (`partial_notes(queued)`, `degraded_notes()`), reusing `list_items_for_session`. Keep `cancel()` unchanged.
- [ ] **Gate:** `nix develop -c cargo test --workspace` + `clippy --workspace --all-targets -- -D warnings`.

#### Task 6 — Swift seam: `WalkEngine` protocol + models (apps/ios)
- [ ] Add `NotesModel` (summary, items `[CapturedFixture]`, docKind, queued) and change the protocol:
  ```swift
  func finish() async -> NotesModel
  func buildDocument(sessionId: String, kind: String) async throws -> DocumentModel
  ```
  Keep `DocumentModel`/`DocRowFixture` exactly as-is (ReviewView unchanged). `// sac:` marker on `NotesModel`'s grouping expectations.
- [ ] **`MurmurEngine`** (real, behind `#if canImport`): `finish()` maps `NotesPayload → NotesModel` (reuse the `board(_:)` item mapping for `items`); add `buildDocument` calling `engine.buildDocument(sessionId:kind:)` and mapping `DocumentPayload → DocumentModel` via the **existing** `document(_:)`. Update `lastDocument`/re-entrancy notes to a `lastNotes` cache.
- [ ] **`DemoWalkEngine`** parity: `finish()` returns scripted notes (`trade` fixtures → `NotesModel`); `buildDocument(sessionId:kind:)` returns the canned `DocumentModel` it returns today (Worked Ex B shape). No-op/echo for unknown kinds.
- [ ] **Gate:** iOS **demo** build (xcodebuild OUTSIDE nix) green.

#### Task 7 — `AppModel` routing + Notes screen seam (apps/ios)
- [ ] Add a `.notes` phase (or reuse `.review` for the document). `finishWalk()` sets `self.notes = await engine.finish()` → `phase = .notes`. The action buttons call `Task { let doc = try await engine.buildDocument(sessionId:kind:); self.document = doc; phase = .review }`; disable buttons when `notes.queued`.
- [ ] Minimal `NotesView` (`// sac:` — visuals/grouping/action-button taxonomy are sac's): summary card, items grouped by `kind`, a "TURN THESE NOTES INTO" action row wired to `buildDocument(kind:)` per `doc_kinds_for_template`'s Swift mirror, an Export/utility row (stub), a collapsed transcript row (from the accumulated transcript). Keep it deliberately thin — sac replaces the chrome; the **wiring** (which button → which kind → ReviewView) is what this task guarantees.
- [ ] `ReviewView` unchanged — verify it still renders a `DocumentModel` reached via the button path.
- [ ] **Gate:** iOS demo build green; manual walk-through of the demo flow (DONE→Notes→button→ReviewView).

#### Task 8 — real-core compile + bindings drift (dam-manual) + merge
- [ ] dam runs `cd apps/ios && ./build-ffi.sh && ./generate.sh && xcodebuild -project SitewalkGallery.xcodeproj -scheme SitewalkGallery -destination 'platform=iOS Simulator,name=iPhone 17' build` — confirm the real-core archive compiles against the flipped `finish()` + new `build_document` bindings (CI cannot do this).
- [ ] Bindings-drift check: regenerate the Swift bindings from `crates/ffi`; confirm `MurmurEngine.swift`'s referenced symbols (`NotesPayload`, `build_document`, unchanged `DocumentPayload`/`DocLine`) all resolve.
- [ ] **Merge PR-2** — this is the ship. TestFlight internal lane publishes the notes-first flow on real-engine.

---

## Gates (every task)
- `nix develop -c cargo test --workspace` — exit code 0 (never grep counts; run under the Nix shell so the toolchain resolves).
- `nix develop -c cargo clippy --workspace --all-targets -- -D warnings`.
- iOS **demo** build (CI-gated): `xcodebuild -project SitewalkGallery.xcodeproj -scheme SitewalkGallery -destination 'platform=iOS Simulator,name=iPhone 17' build` — **outside** the Nix shell.
- Stage 2 only: real-core compile + bindings-drift (dam-manual, Task 8).

## Acceptance criteria
1. A successful `process()` makes **no** `build_document` call; `finish()` returns notes (items + summary); one folded `"processing"` usage row.
2. `MurmurEngine::build_document(session_id, kind)` renders structure deterministically from the session's authoritative items, prices estimate/invoice via one items-only LLM pass, and **always lands a numbered document snapshot** — pricing failure degrades to unpriced + `queued:true`, never an error.
3. Illegal `kind`-for-template and non-`Processed` sessions return `EngineError`; empty-but-Processed yields a truthful zero-line document.
4. Doc numbers mint per generate (burn-per-tap), per-kind independent sequences; regenerate leaves prior snapshots intact.
5. Empty/offline/double-finish notes follow the D3 table; no panic across FFI.
6. Pre-13 documents and `convert::document_payload` still parse unchanged (no migration).
7. main builds the real-core archive at every merge (Stage 1 additive; Stage 2 atomic).
8. `ReviewView` is reached via the button path and renders unchanged; the Notes screen wiring routes each action button → the correct `kind` → ReviewView.

## Non-goals (explicit)
- Price-book store (D6 seam only).
- Notes **editing**; follow-ups system; per-trade button-set **content** + notes-screen **visuals** (sac's).
- PDF export changes; vision; QuickBooks/Jobber integrations.
- A `session_transcript(session_id)` getter (deferred — future work when the history screen needs it; dam 2026-07-10).
- "Replace last document" / doc-number reuse (v1 burns per tap — dam confirmed).
- A price-book-backed spoken-total (D5a captures the *stated* total only; reconciling it against a price book is future work).
- Reprocess-after-Processed / "recover this walk" UX (the dangling-`item_id` render rule is defined for it, but the flow is out of scope).

## Resolved by dam (2026-07-10)
- **Doc numbers burn per tap (D7)** — CONFIRMED. A sent estimate keeps its number; regenerate = a new number/snapshot. No reuse.
- **Spoken grand total (D5a)** — CONFIRMED threaded, captured at summary time and fed as one scalar hint; the pricing prompt never regains transcript access. Mechanism is the plan's call (see D5a).
- **`session_transcript` getter** — DEFERRED. Notes-screen transcript stays client-accumulated; add a core getter only when the history screen (no live event stream) needs it. Future work, not a blocker.

## Open questions for sac
1. **`doc_kinds_for_template` taxonomy (D8)** — the kind vocabulary + pricing flags are core's; confirm the landscape/property/inspection kind lists match sac's button taxonomy (labels/lead-button are sac's). — **sac**
2. **`queued` reuse for pricing-failed documents (D5/C)** — reusing the offline `queued` flag + its "SAVED OFFLINE…" note copy for a pricing-unavailable document — acceptable, or does sac want a distinct "unpriced" state/copy? — **sac**
