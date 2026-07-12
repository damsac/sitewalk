# Murmur Rust Core — Plan 14: comprehensive notes

> **For agentic workers:** REQUIRED SUB-SKILL: use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax. The Rust tasks (1–5) are **hermetic**: in-memory `Store`, `MockProvider`, `SessionProcessor`/`with_providers` — **no model, no `whisper` feature, no network, no camera, no real-photo filesystem**. `cargo test --workspace` must NEVER require the `whisper` feature or a model file (the load-bearing CI invariant). The Swift task (6) is **not CI-gated** (real-core-only, needs the gitignored `MurmurCoreFFI` xcframework) and the **notes-screen visual/grouping/bucket rendering is explicitly sac's** — `// sac:` markers throughout. Run `cargo`/`xcodegen` **inside** the Nix dev shell; run `xcodebuild` **outside** it (Nix linker env breaks Xcode `ld`). Never read `.env` or `project.local.yml`.
>
> **⚠ SHIPPABILITY — merging this plan auto-publishes the TestFlight internal lane on real-engine** (CANON 2026-07-10). main must build the **real-core archive** at every merge. Plan 14 is a **single-stage, additive** change (a new `notes` artifact + a grown-but-additive `NotesPayload`), so it is **ONE PR** — but the real-core compile is a **MANDATORY manual gate** (Task 7; CI cannot build real-core). See **§Staging**.
>
> **Contract source (dam-confirmed on PR #199 — read before implementing):** Isaac's four buckets ARE the contract. Notes are a **client ↔ team coordination artifact**: **Summary** (narrative 2–4 sentences, replaces the one-liner) / **Scope of work** (directives with client detail baked in — "darker mulch than last year") / **Constraints** (budget, permits, deadline, site access/gate codes, client preferences) / **Conditions & issues** (site findings affecting the work). Each notes entry = `bucket` + terse `label` + full `detail`. **The live board stays terse** — detail is notes-only. Target render: `docs/design/notes-mockup.html` (landscape). Prior plan: `docs/superpowers/plans/2026-07-10-rust-core-13-notes-first.md` (LANDED — this plan builds on the notes-first core it shipped).
>
> **Plan review (2026-07-12) — APPROVE WITH CONDITIONS (applied).** All load-bearing invariants recomputed clean: the **Swift contract holds no collision** with #199's actual diff (`NotesPayload.notes` is additive, read by name; the one payload→`NotesModel` map edit is self-contained); the **evals Δ=0 is structural**, not hopeful (the notes pass writes only to the `notes` artifact, which the item-based grader never reads — WE-B); **uniffi additivity is safe** (new field + new `NotesEntry`/`NotesBucket` symbols regenerate cleanly); the **`clear_authoritative_outputs` sweep is free** (it already tombstones all artifacts on reprocess). Three conditions folded in: **C1** — Task 2's `write_summary`→`write_notes` rename must retarget the hardcoded tool-name literal in **6 additional MockProvider-script files** (beyond `prompts.rs`/`mod.rs`/`session.rs` which the plan already edits); an un-renamed script makes `write_notes` find no tool → the summary pass returns nothing → sessions go `Failed` → the workspace gate fails (enumerated in Task 2). **C2** — bump `summary_max_tokens` 512→1024 to fit summary+buckets, cap the entry count in the prompt (≤12, prefer fewer+denser), and pin **graceful degradation** on a truncated/malformed `write_notes` response (`notes:[]`, summary preserved if parseable, never a hard failure — R7); the R9 cost delta is stated honestly (Task 2). **N1** — the PR #199 hard-dep line reworded (OPEN, contract-via-comment; the code doesn't require the merge). dam ruled **HOLD** on the `item_id` link (Open Question 1).

**Goal.** The `finish()` notes screen today returns the terse board (`items`) + a one-line `summary` — glance-able, but it drops the detail that feeds the paperwork ("she wants a *darker* mulch", "gate code #1418", "before the event in 3 weeks"). Plan 14 makes the notes a **comprehensive, structured coordination artifact**: a narrative summary plus **bucketed detailed entries** (Scope of work / Constraints / Conditions & issues), captured **at summary time** from the transcript the summary pass already reads — **no new LLM call, extraction untouched, board provably unchanged**.

**What lands (all in ONE PR):**

1. **murmur-core — the summary pass grows into a notes pass.** `pipeline::summarize` becomes `write_notes`: the same single forced call that already reads the full transcript (and already returns `spoken_total_cents`, Plan 13 D5a) now *also* returns a narrative `summary` (2–4 sentences) + a `Vec<NotesEntry>` (bucket/label/detail). **Zero added LLM calls; extraction agent pass untouched.** (Tasks 1–2.)
2. **murmur-core — notes persist as a `notes` artifact.** `process()` writes a `kind="notes"` JSON artifact (`{"buckets":[…]}`) on success (additive, no migration — `kind` is free-form, mirrors `session_meta`/`document`). `clear_authoritative_outputs` already sweeps it on reprocess. (Task 3.)
3. **murmur-core — evals stay green (R6).** The notes pass writes to the notes artifact, **never to items** — the item-based F0.5 grader reads item rows only, so board precision/recall is byte-identical before/after. Pinned. (Task 4.)
4. **ffi — `NotesPayload` grows additively.** New `notes: Vec<NotesEntry>` field (+ `NotesEntry` uniffi record, `NotesBucket` enum). `finish()`/`partial_notes`/`degraded_notes` read the notes artifact via a new tolerant `session_notes()` helper. (Task 5.)
5. **apps/ios — minimal seam.** `NotesModel` gains `notes: [NotesEntryFixture]`; `MurmurEngine.finish()` maps `NotesPayload.notes`; `DemoWalkEngine` scripts sample buckets. **Bucket rendering / section grouping / visual design is sac's follow-up** (`// sac:`); this task only guarantees the data reaches Swift. (Task 6.)

**What Plan 14 is NOT (see Non-goals for the full list).** No notes **editing**. No change to `build_document` — the document structure still renders deterministically from **items**, not from notes buckets (Q4/D4-14). No new SQLite migration (notes ride in `artifacts.body` JSON). No per-trade bucket variations beyond the four. No price-book. No vision. No PDF.

---

## Hard dependencies (all DONE, on `main`)

- **Plan 13** (notes-first core — LANDED): `finish()` returns `NotesPayload{ session_id, doc_kind, summary, items, queued }` (`crates/ffi/src/notes.rs`); the on-demand `MurmurEngine::build_document(session_id, kind)` (`crates/murmur-core/src/pipeline/document.rs`) renders structure from items + prices estimate/invoice via one items-only pass. The **summary pass** (`pipeline::summarize`, `prompts.rs:75`) already reads the full assembled transcript and already returns `(Option<String> summary, Option<i64> spoken_total_cents, Usage)` — the D5a precedent that proves the tool schema can grow additively. `process()` already writes a `session_meta` artifact for the spoken total (`pipeline/mod.rs:193–200`) — the exact write pattern Plan 14 reuses for the notes artifact.
- **Plan 07** (artifact seam): a generated artifact hangs off a session with a free-form `kind` + JSON `body`; `store.add_artifact(session_id, kind, title, body)` (`store/artifacts.rs:28`); `list_artifacts_for_session`; `clear_authoritative_outputs` sweeps **all** artifacts on a (re)process (`sessions.rs`) — so the notes artifact is retry-clean with no extra code. **No migration** (`MIGRATIONS.len()` unchanged, no `user_version` bump).
- **Plan 06a** (`items.source`): the surviving board after `finish_session_processed` = this-run authoritative + manual items — the terse board the notes screen shows unchanged.
- **PR #199** (sac, **OPEN** — the four-bucket contract is confirmed via its comment thread, but the *code* here does **not** depend on #199 merging): the notes screen already groups whatever `finish()` returns; it will render the richer `notes` payload under sac's grouping once sac wires the bucket sections (a follow-up). Plan 14's Swift edits are self-contained (Task 6) and don't touch #199's diff.

**Verified API facts (checked against source, not guessed):**
- **The summary pass is a single forced `provider.complete` call** (`prompts.rs:80–90`), tool `write_summary`, `tool_choice` forced. Growing its schema + return tuple adds **no** call — the token accounting in `pipeline/mod.rs` (`usage.add(&summary_usage)`) is unchanged in *shape* (one summary call), only its output is richer.
- **`process()` writes `session_meta` only when `spoken_total_cents.is_some()`** (`mod.rs:193–200`), before `finish_session_processed`. The notes-artifact write slots in the same block (write only when `!buckets.is_empty()`).
- **`ProcessOutcome` is `{ session, usage }`** (`mod.rs:65`) and does NOT need to grow — FFI reads the notes artifact from the store at `finish()` time, exactly as it reads `session.summary` from the outcome. No new field threads across the FFI boundary via `ProcessOutcome`.
- **The evals grader reads item rows + contacts + `summary_present`** (`crates/evals/src/grade.rs:24–29`, `Observed`), built from the store by `run.rs`. It does **not** read artifacts. `summary_present` is a bool. So a notes artifact + a longer summary string cannot move the F0.5 score. (Confirm no summary-*length* pin exists — Task 4.)
- **`NotesPayload` is a `uniffi::Record`** (`crates/ffi/src/notes.rs:10`); adding a field regenerates bindings additively (Swift reads fields by name). New `NotesEntry`/`NotesBucket` are new symbols — purely additive.
- **`add_item`'s enum** (`pipeline/tools.rs:117`) is `todo|decision|note|safety|part|price` — the notes pass does NOT use `add_item` and cannot add to this set; the four buckets are an **orthogonal** taxonomy living only in the notes artifact.

**Spec basis:** R6 (under-extraction bias — the notes pass must not author board items and must not fabricate constraints), R7 (never lose the outcome — a notes-pass parse failure degrades to `notes:[]`, never a hard failure), R9 (spend meter — no new call, so no new cost). Design: notes-first (#189), the four-bucket contract (#199 thread).

---

## Architecture — decisions, justified (reviewers read these first)

### D1-14. Where the depth comes from: **grow the summary pass** (option b)

Three candidates were weighed:

- **(a) Extend the extraction pass** — give `add_item` a `detail`/`bucket`. **Rejected.** The extraction agent runs *live* (latency-sensitive) and its output IS the terse board; baking detail into it either bloats the board (breaks "the board stays terse") or forces a live/finish output split on a hot path. It also puts client/logistics/budget capture on the R6-graded item extractor, muddying the incentive the F0.5 grader pins.
- **(c) A third dedicated notes pass** — a new LLM call after summarize. **Rejected.** R9: it doubles the finish-time call count for content the summary pass could produce for free. No new information is available to a third call that the summary call doesn't already have (both read the same transcript).
- **(b) Enrich at summary time — ADOPTED.** The `summarize` pass **already legitimately reads the full assembled transcript** and **already** returns an extra scalar (`spoken_total_cents`, D5a) — the exact precedent. Grow its forced tool from `write_summary` → `write_notes`: one call returns `{ summary (narrative), spoken_total_cents (optional), notes[] (buckets) }`. **Net LLM calls unchanged. Extraction untouched → the board is provably identical → the F0.5 grader is provably unmoved (D3-14).**

The narrative summary and the buckets are two projections of the same transcript read; producing them in one structured call is strictly cheaper than any alternative and keeps transcript access confined to the one pass that already has it.

### D2-14. `NotesEntry` shape + the four buckets

The **Summary** bucket is the top-level narrative string (it replaces Plan 13's one-liner; the mockup renders it as the summary card). The other **three** buckets are entry lists:

```rust
// crates/murmur-core/src/pipeline/notes.rs  (new; plain serde — core-side)
pub struct NotesEntry {
    pub bucket: String,   // "scope_of_work" | "constraints" | "conditions_and_issues"
    pub label:  String,   // terse (mirrors a board label)
    pub detail: String,   // the full spoken context ("darker mulch than last year")
}
```

```rust
// crates/ffi/src/notes.rs  (new uniffi types)
#[derive(uniffi::Enum, …)] pub enum NotesBucket { ScopeOfWork, Constraints, ConditionsAndIssues }
#[derive(uniffi::Record, …)] pub struct NotesEntry { pub bucket: NotesBucket, pub label: String, pub detail: String }
```

- **`bucket` is a string in the artifact, an enum across FFI.** Core stores the wire string (tolerant of drift); FFI maps string→enum and **drops** any entry whose bucket isn't one of the three (R6 posture: never fabricate a bucket; a garbled row is dropped, not coerced). So Swift only ever sees valid variants — sac's exhaustive `switch` is safe.
- **`label` + `detail` are plain `String`s** (both required, `detail` may be empty). Not `Option<String>` — a uniform record is simpler for uniffi and Swift, and an empty detail is a truthful "terse label, no extra context."
- **Entry count is capped (C2)** — the prompt asks for **≤12 entries, prefer fewer + denser**, and the summary/notes call's output budget is bumped 512→1024 tokens (Task 2) so the full payload fits in one response. A truncated/malformed response degrades to `notes:[]` with the summary preserved (R7), never a hard failure.
- **No `item_id` link in v1.** The notes pass is fed the **transcript only** (exactly like `summarize` today) — it does NOT receive the extracted items, so its entries carry no board-item foreign key. This keeps the two passes fully decoupled, adds zero tokens, and means the notes pass has **no** board-authoring surface. The board and the notes are parallel views of the same transcript; reconciling them (jump-to-photo from a Scope entry) needs the item-linking seam and is **deferred** (Open Question 1). This is the deliberate v1 trade: cleaner and R6-provable over richer cross-links.

### D3-14. R6 / evals invariance — the load-bearing guarantee

The item-based F0.5 grader (`grade.rs`) scores `Observed.items`, built by `run.rs` from **store item rows**. Plan 14:
- **does not touch the extraction agent pass** (its prompt, its `add_item` tool, its enum) → the item rows are byte-identical to pre-14;
- **writes buckets only to the `notes` artifact** (never `add_item`) → zero new item rows;
- keeps `summary_present == true` (the notes pass still returns a summary).

Therefore, for **every** corpus scenario, `score₁₄ ≡ score₁₃`. This is not "we hope it stays green" — it is a structural invariant, pinned by a test asserting the item-row count/content is unchanged when the notes pass returns buckets (Task 3) plus the existing eval gate (Task 4). The notes prompt carries its **own** R6 clause ("capture only what was said; never invent a budget, deadline, or access detail — a missed note is cheaper than a fabricated constraint") so the *notes* content is conservative too, but that quality is not graded by the item F0.5 — the separation of incentives is the point.

### D4-14. `build_document` is unchanged — notes are display + export only

`DocumentBuilder::build` renders structure from **items** and prices from **items** (Plan 13 D4/D5). Plan 14 does **not** feed notes buckets into document lines. Reasons:
- Feeding Scope-of-work detail into `render_structure_document` would let the *notes pass* author document content — reopening exactly the transcript→line R6 inflation surface Plan 12/13 closed. The document must stay item-authoritative.
- The document and the notes are different artifacts for different audiences (priced deliverable vs. crew coordination read).
- **Future seam (deferred):** once notes entries carry an `item_id` (Open Question 1), a document line could pull its `detail` from the matching Scope entry (richer estimates). Named, not built.

**Constraints ⟷ spoken-total reconciliation (no duplication).** Plan 13 D5a already captures a machine-readable `spoken_total_cents` in the `session_meta` artifact and threads it into the pricing pass as one scalar. The **Constraints** bucket ALSO surfaces the budget — but as a **human-readable display string** ("Budget — keep the whole job under $1,200"), not a second pricing input. Both come from the **same** `write_notes` call (summary + `spoken_total_cents` + buckets in one response), so there is one capture, two projections: `session_meta.spoken_total_cents` drives pricing (unchanged); the Constraints entry is display-only. The pricing pass never reads the notes artifact.

### D5-14. Persistence — a `notes` artifact, tolerant parser, no migration

Buckets persist as a durable per-session artifact `kind="notes"`, body `{"buckets":[{bucket,label,detail},…]}`, written by `process()` on success **only when non-empty** (mirrors the `session_meta` write). Not item columns (buckets aren't items) and not a session-row column (no migration; the narrative summary already rides the existing `summary` column). The parser is **tolerant** field-by-field (like `convert::document_payload`): a missing/garbled entry is skipped, an unknown bucket string is dropped, a pre-14 session with **no** notes artifact yields `notes: []`. `clear_authoritative_outputs` already tombstones it on reprocess (retry re-captures cleanly).

The narrative **summary** stays on `session.summary` (the `write_notes` tool's `summary` field, now instructed to be 2–4 sentences) — no schema change, and the library/history list keeps reading `session.summary` (a slightly longer subtitle; truncation is a sac display concern, Open Question 2).

### D6-14. Empty / offline / double-finish — the D3 table extended

Plan 13's D3 table gains a `notes` column (FFI reads the notes artifact via `session_notes()`; absent → `[]`):

| finish scenario | summary | items | **notes** | queued |
|---|---|---|---|---|
| empty/whitespace transcript (process short-circuits) | `"(empty session)"` | manual only (usually `[]`) | **`[]`** (no notes artifact written) | `false` |
| offline / LLM-down (process `Err`) — degrade | `""` | current live board | **`[]`** (process never completed → no artifact) | `true` |
| double-`finish()` / post-`cancel()` | stored summary (or `""`) | current board | **stored notes artifact (or `[]`)** | `false` |
| normal | narrative (2–4 sentences) | authoritative+manual | **buckets from the notes artifact** | `false` |

No branch panics across FFI (`finish()` still returns a bare `NotesPayload`). `session_notes()` is fallible-tolerant: any store/parse error yields `[]`, never an unwind.

### D7-14. Compat / staging — ONE PR

Adding `notes: Vec<NotesEntry>` to the existing `NotesPayload` record + the new `NotesEntry`/`NotesBucket` symbols is **additive** (bindings regenerate; Swift reads the new field by name; existing Swift ignores it until sac renders it). The **one** required Swift edit is the payload→`NotesModel` mapping (set `notes`), co-located with a `NotesModel.notes` field defaulting to `[]` and a `DemoWalkEngine` sample — all trivial and in this PR. Bucket **rendering** (the grouped sections) is sac's follow-up; main is shippable in the meantime (buckets present in data, unrendered → no regression, sac's #199 grouping already tolerates extra payload). A two-stage split (persist first, payload+Swift later) buys nothing here because the payload growth is additive and the Swift mapping edit is tiny — one atomic PR keeps main from ever holding a half-wired notes contract. **Mandatory gate:** dam's local real-core compile + bindings-drift check (Task 7; CI can't build real-core — Plan 13 lesson).

---

## Worked examples (reviewers: hand-recompute against the code)

**WE-A — the notes pass, one call (happy path).** Template `landscape`. Transcript:
*"Susan wants fresh bark mulch on the front beds — darker than last year, the old stuff faded. Trim the four boxwoods along the walkway. Zone 2 irrigation head is broken, needs replacing. Keep the whole job under twelve hundred. She needs the quote by Friday. Gate code's 1418."*

- **Extraction agent pass (UNCHANGED):** `add_item todo "bark mulch, front beds"`, `todo "trim 4 boxwoods, walkway"`, `part "replace zone-2 irrigation head"` → **board = 3 terse items**. (This is the F0.5 domain — untouched.)
- **`write_notes` call (the grown summary pass) returns ONE response:**
  - `summary` = *"Estimate walk on Susan Hollis's front yard. Fresh (darker) bark mulch, boxwood trimming, and a broken zone-2 irrigation head to replace. Target ~$1,200; quote due Friday."* (narrative, 3 sentences)
  - `spoken_total_cents` = `120000`
  - `notes` = 6 entries:
    - `{scope_of_work, "Bark mulch — front beds", "Darker mulch than last year; the old mulch faded."}`
    - `{scope_of_work, "Trim boxwoods", "Four boxwoods along the walkway; shape + haul clippings."}`
    - `{conditions_and_issues, "Zone-2 irrigation head broken", "Replace — parts + labor."}`
    - `{constraints, "Budget", "Keep the whole job under $1,200."}`
    - `{constraints, "Deadline", "Quote due Friday."}`
    - `{constraints, "Site access", "Gate code 1418."}`
- **`process()` writes:** `session.summary` = the narrative; `session_meta` = `{"spoken_total_cents":120000}` (Plan 13, unchanged); **`notes` artifact** = `{"buckets":[…6…]}`.
- **`finish()` →** `NotesPayload{ doc_kind:"estimate", summary:<narrative>, items:[3 terse board items], notes:[6 entries], queued:false }`.
- **Cost (R9):** extraction-agent usage + **one** `write_notes` call — **exactly the call count of today's summarize**. Assert `provider.requests()` contains the extraction requests + one `write_notes` forced call and **no** third pass. The board (3 items) is byte-identical to a Plan-13 run of the same transcript.

**WE-B — evals invariance (D3-14).** Take any corpus scenario whose Plan-13 baseline is, say, `TP=3, FP=1, FN=1` → precision `3/4=0.75`, recall `3/4=0.75`, `F0.5 = (1+0.25)·0.75·0.75 / (0.25·0.75 + 0.75) = 0.703125/0.9375 = 0.75`. Under Plan 14, `run.rs` builds `Observed.items` from the **same item rows** (extraction untouched; notes wrote to a different artifact the grader never reads) → `TP/FP/FN` identical → `F0.5 = 0.75`. **Δ = 0** for every scenario. `summary_present` stays `true`. Pinned by the "notes pass adds zero item rows" test (Task 3) + the eval gate (Task 4).

**WE-C — `build_document` untouched by notes.** `build_document(S,"estimate")` on the WE-A session renders **3** lines from the **3 board items** (never the 6 notes entries). The pricing pass is fed those 3 items + the `spoken_total` scalar `$1,200` from `session_meta` (never the Constraints "Budget" entry, never the transcript). Model returns e.g. `{mulch:28500, boxwoods:14000, irrigation:12000}` → priced lines sum `$545.00` (≤ target). **The 6 notes entries appear nowhere in the document.** Proves notes are display/export-only and the document stays item-authoritative; the budget lives in two decoupled projections (pricing scalar vs. display note).

**WE-D — empty / offline / double-finish (D6-14).**
- *Empty:* silent walk → `finish()` short-circuits → `NotesPayload{ summary:"(empty session)", items:[], notes:[], queued:false }`; no notes artifact written.
- *Offline (process `Err`):* `NotesPayload{ summary:"", items:<live board>, notes:[], queued:true }`; process never completed, so no notes artifact exists.
- *Double-finish:* first `finish()` processed the session and wrote a notes artifact; the second `finish()` hits `degraded_notes()` → reads `session.summary` + `session_notes()` (the stored 6 entries) → `notes:[…6…]`, `queued:false`. No reprocess, no panic.

**WE-E — tolerant parse / pre-14 compat (D5-14).**
- A notes artifact body `{"buckets":[{"bucket":"logistics","label":"x","detail":"y"},{"bucket":"scope_of_work","label":"Mulch","detail":"…"}]}` → FFI parse **drops** the unknown `"logistics"` entry, keeps the one valid Scope entry → `notes:[1 entry]`. No error.
- A **pre-14** session (finished before this plan) has no notes artifact → `session_notes()` → `[]` → `NotesPayload.notes:[]`. The payload is valid; the notes screen shows summary + terse items exactly as it did in Plan 13. Old `document` artifacts still parse unchanged.

---

## Staging (main stays shippable)

**ONE PR** (`pr/dam/plan-14-comprehensive-notes` → main). All-additive: a new `notes` artifact, a grown `write_notes` tool, a new `NotesPayload.notes` field, new uniffi types, a minimal Swift mapping edit + demo sample. Gated by `cargo test --workspace` + `clippy --workspace --all-targets -- -D warnings` + iOS **demo** build + **the mandatory dam-manual real-core compile + bindings-drift check (Task 7)**. Rust tasks (1–5) are independently `cargo`-testable before the Swift task (6); they merge together. Bucket **rendering** is a **separate sac PR** (not in scope here) — main ships the data first, the chrome follows.

---

## Tasks

### Task 1 — `NotesEntry` core type + tolerant parser (murmur-core)
- [ ] **RED:** in a new `crates/murmur-core/src/pipeline/notes.rs`, tests for `parse_notes_artifact(body: &str) -> Vec<NotesEntry>`: a well-formed `{"buckets":[…]}` round-trips; an entry with an **unknown bucket** string is **dropped**; a malformed/missing-field entry is **skipped**; an empty/absent body → `[]`. Test `serialize_buckets(&[NotesEntry]) -> String` produces `{"buckets":[…]}` that `parse_notes_artifact` reads back identically.
- [ ] **GREEN:** add `pub struct NotesEntry { pub bucket: String, pub label: String, pub detail: String }` (`serde`, `Clone`, `PartialEq`); `const NOTE_BUCKETS: [&str;3] = ["scope_of_work","constraints","conditions_and_issues"]`; `pub fn parse_notes_artifact(body: &str) -> Vec<NotesEntry>` (tolerant: `serde_json::from_str` a `Value`, walk `buckets[]`, keep only rows with all three string fields whose `bucket ∈ NOTE_BUCKETS`); `pub fn serialize_buckets(entries: &[NotesEntry]) -> String`. Wire `pub mod notes;` in `pipeline/mod.rs`.
- [ ] **Gate:** `nix develop -c cargo test -p murmur-core`.

### Task 2 — grow the summary pass into `write_notes` (murmur-core; D1-14/D2-14)
- [ ] **RED (prompts.rs):** rename the forced tool to `write_notes`; extend its schema with an optional `notes` array (`bucket` enum `scope_of_work|constraints|conditions_and_issues`, `label`, `detail`) alongside the existing `summary` (now described "2–4 plain sentences: what, why, when") + `spoken_total_cents`. Change `summarize`'s return to `(Option<String> summary, Option<i64> spoken_total_cents, Vec<NotesEntry> buckets, Usage)`. Update the existing prompts.rs tests to the new tool name; add: a response with a `notes` array → parsed `Vec<NotesEntry>` (unknown buckets dropped via `parse`); a response with **no** `notes` → `buckets: []`; the R6 clause is present in the system prompt ("never invent a budget, deadline, or access detail"); **the entry-count cap clause is present** ("at most 12 entries; prefer fewer, denser entries"). Keep `spoken_total_cents` behavior pinned.
- [ ] **RED (C1 — rename blast radius):** the tool name `"write_summary"` is **hardcoded in MockProvider scripts across 6 additional files** beyond the three this task/plan already edits (`prompts.rs` defines it; `pipeline/mod.rs` + `ffi/src/session.rs` hold test helpers Tasks 2/3/5 touch). Each of the six holds **one** `"write_summary"` string literal inside a shared `summary_response`/`summary` helper that fans out across many tests — an un-renamed literal makes the forced `write_notes` call find no matching tool → `summarize` returns `None` summary → the session goes **`Failed`** → `cargo test --workspace` fails. Retarget **all six** to `"write_notes"`:
  - `crates/evals/tests/grader_hermetic.rs` (`:24`)
  - `crates/murmur-core/tests/source_swap_e2e.rs` (`:22`)
  - `crates/murmur-core/tests/live_extraction_e2e.rs`
  - `crates/murmur-core/tests/pipeline_e2e.rs`
  - `crates/ffi/tests/bridge_e2e.rs` (`:55`)
  - `crates/ffi/src/document_build.rs` (`:89`, `#[cfg(test)]`)

  (`grep -rn '"write_summary"' crates/` must return **zero** hits after this task — a single missed literal fails the workspace gate silently as a `Failed` session, not a compile error.)
- [ ] **RED (C2 — graceful degradation, R7):** a `write_notes` response whose `notes` array is **truncated/malformed** (e.g., the model hit `max_tokens` mid-array, or an entry is a bare string) must NOT hard-fail: `summarize` returns the **parseable summary** + `buckets: []` (drop the unparseable tail, never `Err`). Pin: a response with a valid `summary` but a garbled `notes` value → `(Some(summary), _, vec![], usage)`. A response missing `write_notes` entirely stays the existing `(None, None, [], usage)` "loggable spend" path (unchanged).
- [ ] **GREEN:** rename `WRITE_SUMMARY`→`WRITE_NOTES` (`"write_notes"`); grow `summary_tool_spec`→`notes_tool_spec`; parse the `notes` array through `notes::parse_notes_artifact`-style field-validation (reuse the Task 1 validator on the tool `input`, not just the artifact body — factor a `fn parse_notes_value(&serde_json::Value) -> Vec<NotesEntry>` both call; a non-array / garbled `notes` field yields `[]`, never a panic — C2). Rewrite the system prompt to state the coordination-artifact purpose + the four buckets + the R6 clause + the ≤12-entry "prefer fewer, denser" cap.
- [ ] **GREEN (C2 — token budget):** bump `SessionProcessor::summary_max_tokens` **512 → 1024** (`pipeline/mod.rs:99`) so summary + buckets fit in one response without truncation. **R9 cost delta, stated honestly:** the summary/notes call's *output* budget doubles (≈512 extra output tokens worst-case, ~one-third of a cent at current pricing); there is **no new call** (D1-14), so per-finish call count is unchanged — the delta is a modest output-token bump on the one pass that already ran, in exchange for the whole comprehensive-notes payload. Acceptable per the notes-first value trade.
- [ ] **Gate:** `nix develop -c cargo test -p murmur-core` (+ `grep -rn '"write_summary"' crates/` returns nothing).

### Task 3 — thread buckets through `process()` + persist the notes artifact (murmur-core; D5-14)
- [ ] **RED:** `run_llm_phases` returns `(String summary, Option<i64> spoken_total_cents, Vec<NotesEntry> buckets)`. In `pipeline/mod.rs` tests: a `write_notes` response carrying buckets → after `process()`, a `kind=="notes"` artifact exists with the serialized buckets (parse it back, assert entries); a response with **no** buckets → **no** notes artifact (mirrors the `session_meta` absence test). **R6 pin (D3-14):** a `process()` whose `write_notes` returns buckets adds **zero** item rows beyond what `add_item` created — assert `list_items_for_session(sid).len()` equals the count the extraction agent produced (buckets never become items). Update `processes_a_session_end_to_end`'s usage assertion — it is **unchanged** (one summary/notes call, same MockProvider token totals): `Usage { input_tokens: 350, output_tokens: 70 }` still holds because no call was added.
- [ ] **GREEN:** in `run_llm_phases`, capture the buckets from `summarize`/`write_notes` and return them. In `process()`'s success arm, after the `session_meta` write and **before** `finish_session_processed`, `if !buckets.is_empty() { store.add_artifact(session_id, "notes", "notes", &notes::serialize_buckets(&buckets))?; }`. `ProcessOutcome` stays `{ session, usage }` (buckets reach FFI via the artifact, not the outcome).
- [ ] **Gate:** `nix develop -c cargo test -p murmur-core` + `nix develop -c cargo clippy -p murmur-core --all-targets -- -D warnings`.

### Task 4 — evals stay green + no summary-length pin (murmur-core / evals; D3-14)
- [ ] **RED/verify:** grep `crates/evals` for any assertion on summary **text/length** (only `summary_present` should be pinned). If a length pin exists, retarget it to presence-only (the narrative is now 2–4 sentences). Confirm the grader's `Observed` is item/contact/`summary_present` only (no artifact read) — document the invariance in a one-line test comment.
- [ ] **Gate (the real check):** `nix develop -c cargo test -p evals` — the F0.5/precision pins are **byte-identical** to pre-14 (extraction untouched). Exit code 0. If any eval score moved, STOP — the notes pass leaked into item extraction (a design violation), not a test to "fix."

### Task 5 — `NotesPayload.notes` + FFI wiring (ffi; D2-14/D6-14/D7-14)
- [ ] **RED:** in `crates/ffi/src/notes.rs` + `session.rs` tests (`with_providers`): a happy-path `finish()` (WE-A) returns `NotesPayload.notes` with the mapped entries (bucket enum correct); an unknown-bucket artifact row is dropped (WE-E); a pre-14 / no-artifact session → `notes: []`; the D6-14 table — empty→`[]`, offline(`queued:true`)→`[]`, double-finish→stored buckets. Assert `items` (the terse board) is unchanged by any of this.
- [ ] **GREEN:** add `#[derive(uniffi::Enum)] NotesBucket { ScopeOfWork, Constraints, ConditionsAndIssues }` + `#[derive(uniffi::Record)] NotesEntry { bucket: NotesBucket, label: String, detail: String }`; add `pub notes: Vec<NotesEntry>` to `NotesPayload`; grow `notes_payload(...)` with a `notes: &[NotesEntry]` param. Add `convert::notes_entries(core: &[murmur_core::…NotesEntry]) -> Vec<NotesEntry>` (map bucket string→enum, **drop** unknowns). Add a `WalkSession::session_notes() -> Vec<NotesEntry>` helper: read the latest `kind=="notes"` artifact via `list_artifacts_for_session`, `parse_notes_artifact`, map; any error → `[]`. Thread `session_notes()` into `partial_notes`/`degraded_notes`/the happy-path `finish()` arms per the D6-14 table.
- [ ] **Gate:** `nix develop -c cargo test -p ffi` + `nix develop -c cargo clippy -p ffi --all-targets -- -D warnings` + `nix develop -c cargo test --workspace`.

### Task 6 — Swift seam: `NotesModel.notes` + demo sample (apps/ios; `// sac:` rendering deferred)
- [ ] Add `NotesEntryFixture { bucket: NotesBucket, label: String, detail: String }` + a `NotesBucket` Swift enum (mirrors the uniffi enum); add `notes: [NotesEntryFixture]` (default `[]`) to `NotesModel`. `// sac:` marker — **the grouped bucket sections + visuals are sac's follow-up**; this task only carries the data.
- [ ] **`MurmurEngine`** (real, `#if canImport`): `finish()` maps `NotesPayload.notes → NotesModel.notes`.
- [ ] **`DemoWalkEngine`** parity: scripted `finish()` returns a few sample bucket entries (WE-A shape) so the demo build exercises the field. Unknown/empty → `[]`.
- [ ] **Gate:** iOS **demo** build (xcodebuild OUTSIDE nix) green: `xcodebuild -project SitewalkGallery.xcodeproj -scheme SitewalkGallery -destination 'platform=iOS Simulator,name=iPhone 17' build`.

### Task 7 — real-core compile + bindings drift (dam-manual) + merge  **[MANDATORY GATE]**
- [ ] dam runs `cd apps/ios && ./build-ffi.sh && ./generate.sh && xcodebuild -project SitewalkGallery.xcodeproj -scheme SitewalkGallery -destination 'platform=iOS Simulator,name=iPhone 17' build` — confirm the real-core archive compiles against the grown `NotesPayload` + new `NotesEntry`/`NotesBucket` bindings (CI cannot do this — Plan 13 lesson).
- [ ] Bindings-drift check: regenerate the Swift bindings from `crates/ffi`; confirm `MurmurEngine.swift`'s referenced symbols resolve and no unrelated record drifted.
- [ ] **Merge the PR** — TestFlight internal lane publishes the comprehensive-notes payload on real-engine. (sac's bucket-rendering PR follows independently.)

---

## Gates (every task)
- `nix develop -c cargo test --workspace` — exit code 0 (never grep counts; run under the Nix shell so the toolchain resolves).
- `nix develop -c cargo clippy --workspace --all-targets -- -D warnings`.
- `nix develop -c cargo test -p evals` — F0.5/precision pins byte-identical to pre-14 (Task 4).
- iOS **demo** build (CI-gated): `xcodebuild … SitewalkGallery … build` — **outside** the Nix shell.
- **MANDATORY:** real-core compile + bindings-drift (dam-manual, Task 7) — before merge.

## Acceptance criteria
1. The summary pass makes **one** forced `write_notes` call returning `{ summary (2–4 sentences), spoken_total_cents (optional), notes[] }`; **no** LLM call is added vs. Plan 13; the extraction agent pass is untouched.
2. `process()` persists buckets as a `kind="notes"` JSON artifact (only when non-empty), no migration; `clear_authoritative_outputs` sweeps it on reprocess.
3. The evals F0.5/precision scores are **byte-identical** to pre-14 for every corpus scenario (the notes pass writes zero item rows).
4. `finish()` returns `NotesPayload` with a populated `notes: Vec<NotesEntry>` (bucket/label/detail), buckets grouped Scope/Constraints/Conditions; the terse `items` board is unchanged.
5. Empty / offline(`queued`) / double-finish / pre-14 sessions follow the D6-14 table (`notes:[]` or stored buckets); no panic across FFI; unknown-bucket rows dropped tolerantly; a truncated/malformed `write_notes` response degrades to `notes:[]` with the summary preserved (R7).
6. `build_document` is unchanged — document lines still render from **items**, priced from the `session_meta` spoken-total scalar; the Constraints "Budget" entry is display-only.
7. `NotesPayload` grows additively; one PR; main builds the real-core archive at merge (Task 7 gate green).

## Non-goals (explicit)
- Notes **editing** (the notes are read-only; edit is future).
- An `item_id` link between notes entries and board items (deferred seam — Open Question 1); and any document-line enrichment from notes detail (D4-14 future seam).
- Feeding notes buckets into `build_document` structure or pricing (document stays item-authoritative).
- Per-trade bucket variations beyond the four; a fifth "People/Logistics/Materials" bucket (folded into Constraints per Isaac, v1).
- The notes-screen **visual design / bucket section rendering / grouping chrome** (sac's follow-up PR).
- A price book; a price-book-backed budget reconciliation; PDF export; vision.
- A new SQLite migration; a `session_transcript(session_id)` getter (still deferred from Plan 13).

## Open questions
1. **`item_id` link (sac + dam).** v1 keeps notes entries free-text (no board-item foreign key) to keep the passes decoupled and R6-provable. Pulling the link forward would enable jump-to-photo from a Scope entry and document-line enrichment (D4-14) — worth it now, or hold for a later plan? — **recommend: hold.**
2. **Narrative summary length in the library list (sac).** `session.summary` is now 2–4 sentences (was a one-liner). The history/library subtitle reads it directly — does sac want core to also store a short derived one-liner, or will the list truncate client-side? — **recommend: client-side truncate, no core change.**
3. **Bucket taxonomy freeze (sac + dam).** The three entry buckets (`scope_of_work`/`constraints`/`conditions_and_issues`) + top-level Summary match Isaac's #199 contract exactly. Confirm before wiring sac's exhaustive Swift `switch`, since adding a fourth entry-bucket later is an additive-but-visible enum change.
