# Rust Core 19 — the DocumentSchema seam

**Owner:** dam (murmur-core storage/pipeline + FFI) · **Ratified frame:** #234
(`docs/design/2026-07-16-paperwork-structure-v2-plan.md`) + dam's §7 answers on
#207 (`docs/design/2026-07-12-customizable-paperwork.md`). **Status:** the ONE
serial core dependency of Paperwork Structure v2. sac builds the authoring UI +
schema-driven rendering over dam's absence, ON TOP of this seam.

Plan numbers **17** (corrections-learning) and **18** (bucket-edit) are RESERVED
for other queued work — do not renumber.

**Rev 2 (adversarial review):** seeding moved out of the migration's inline SQL
into a Rust `seed_builtin_schemas` called from `from_connection` after migrate
(ONE source of truth = `builtin_schemas()`, sentinel `device_id`, guard live on
every open — folds in NIT 2); fill provider-`Err` now sets `queued` to mirror the
pricing degrade (SHOULD-FIX 3); WE-C + a named sac follow-up list pin the Swift
prefix-map divergence (SHOULD-FIX 2); WE-B's fill prompt reuses the real
`format_pricing_items` item shape (NIT 1).

---

## 1. Thinking

The whole customization moat ("voice → *your exact document*") rests on one
load-bearing invariant, and this plan exists to make it real without moving a
single user-visible byte at launch:

> **The LLM only ever fills a *named schema*.** A custom field is safe to fill
> because it is a named slot the fill prompt knows about — never a freeform
> guess. That is what keeps "customizable documents" from becoming "silently
> wrong paperwork."

Today's document path already IS a fixed schema, hardcoded in three Rust
functions: `render_structure_document` (one line per item), `is_pricing_kind`
(which kinds price), and `total_shape` (sum-vs-static total). This plan lifts
those three hardcoded facts into a **seeded `document_schemas` row per built-in
(trade × kind)**, routes `build_document` through the row, and adds CRUD +
per-schema numbering + a one-call fill pass for authored fields. Nothing else.

The design pin that makes this shippable across dam's month away: **v1 launches
on the seeded built-ins only** — the authoring editor is sac's, not wired at
launch. So the refactor is invisible: with only built-ins present, the document
is byte-for-byte what it is today. That is not a hope; it is **Stage 4's gated
acceptance criterion** — every existing characterization test passes UNMODIFIED
(Δ=0), pinned per trade × kind.

Four §7 recommendations were accepted on #234 (`gh pr view 234`), and this plan
honors each as a hard constraint:

1. **Fill = ONE focused structured-output call per doc-build.** `write_notes`
   (the finish-time notes pass) is untouched. Doc-fill is a build-time,
   per-tap, R9-metered pass — the same shape as the existing `price_items` pass.
2. **Exactly one `line_items` section per doc v1 — as a section KIND, not a
   singleton field.** The labor-vs-materials multi-section split is the *named*
   future relaxation; the schema shape must not foreclose it.
3. **Schemas scoped per `trade_key`.** Matches how doc kinds already key off
   template. Cross-trade sharing later = copy, not reference.
4. **Built-ins seeded from `from_connection` (after migrate) WITH the
   resurrection pin.** The v7 migration creates the TABLE only; a Rust
   `seed_builtin_schemas(conn, ...)` runs the INSERTs, iterating the ONE source
   `builtin_schemas()` (the migration cannot: `migrate_with` is pure SQL strings
   with no Rust hook, and it runs BEFORE `device_id` exists — `store/mod.rs`:47-50).
   Fixed UUIDs + skip-if-tombstoned: an operator who deletes a built-in must not
   see it resurrect on the next app-update. Because the seed runs on **every
   open** (guarded by `NOT EXISTS` incl. tombstoned rows), the resurrection guard
   is LIVE on every launch, not inert-after-v7. This is the one place the design
   can silently betray the user, so it is an explicit Stage-1 acceptance
   criterion (WE-A).

Out of scope (named so the seam's shape accounts for them, but no code):
authoring UI + schema-driven rendering (sac's), folding the app-side
`DocumentLayout` terms/signature into `static` fields (sac's, once the seam
lands), new section kinds beyond `static | filled | line_items`, upload→infer
comprehension (v3 / Premium), per-item price book, and confirm-once (Premium/v3,
dam's §7 answer: OUT).

---

## 2. Current reality (verified end-to-end, 2026-07-19)

- **Schema version is at v6** (`store/items.rs::fresh_store_is_at_schema_v6`;
  `MIGRATIONS` has 6 entries). This plan appends **v7**.
- **`build_document` today** (`pipeline/document.rs::DocumentBuilder::build`):
  validates the session is `Processed` + the kind is in
  `doc_kinds_for_template(template)`; renders one line per item via
  `render_structure_document(kind, items, PerPricingKind)` (is_gap =
  `is_pricing_kind(kind)`); if `is_pricing_kind(kind) && !items.is_empty()`,
  runs the `price_items` focused pass (items-only, R6, degrades to `queued` on
  failure, R7); stamps `total_shape(kind)` → `(total_kind, total_label_key)`;
  mints + writes the `document` artifact via
  `mint_document_number_and_add_artifact`.
- **Kind vocabulary** (`pipeline/mod.rs`): `doc_kinds_for_template` —
  landscape → `[estimate, invoice, work_order]`, property → `[condition,
  move_out]`, inspection → `[inspection]`, None → `[report]`.
  `is_pricing_kind` → only `estimate | invoice`. `total_shape` →
  `inspection → (static, findings)`, else `(sum, total)`.
- **Numbering** (`store/documents.rs`): `document_sequences(doc_kind PK, next,
  device_id)` — per-kind monotonic integer, minted core-side, stamped as
  `doc_number` into the artifact body inside ONE transaction (a failed write
  rolls the sequence bump back). The `<PREFIX>-NNNN` *string* is assembled
  **Swift-side** today (`MurmurEngineFormatting.swift`:
  estimate→EST, invoice→INV, work_order→WO, inspection→IR, condition→COND,
  move_out→MO, default→DOC; `%04d`).
- **The payload** (`ffi/document.rs`): `DocumentPayload { doc_kind, doc_number,
  job_date_unix, total_kind, total_label_key, static_total_cents, lines[],
  queued }`; `DocLine { id, title, detail, qty, amount_cents, section, is_gap,
  item_id }`. `section` is **always `null`** today. `ffi/convert.rs::
  document_payload` decodes the artifact body tolerantly (missing keys default);
  additive body keys are already precedent (Plan 12 added `item_id`).
- **CRUD precedent** to mirror: `ffi/vocabulary.rs` (lock → mutate → snapshot →
  release → persist; idempotent; the `_seeds` tombstone-respecting marker) and
  `ffi/items.rs` (engine-keyed, `Processed`-gate under ONE lock, R6 kind
  allowlist reject-never-coerce, `EngineError::Item`). `store/items.rs` is the
  sync-ready row discipline (created_at/updated_at/device_id/deleted_at,
  COALESCE partial update, tombstone-guarded reads).
- **Evals** (`crates/evals`): the grader scores *extraction* items/contacts/
  summary only — it does not grade documents. A document-fill eval is therefore
  a new hermetic characterization test driving `DocumentBuilder` with a
  `MockProvider` (Stage 6), not a grader change.
- **iOS seam:** `WalkEngine.buildDocument`, `DocumentLayout` (#214). sac folds
  `DocumentLayout` into `static` schema fields LATER — NOT this plan.

---

## 3. Data model — `document_schemas` (sync-ready)

A doc type is an ordered list of named sections; sections carry ordered fields.
Row is sync-shaped exactly like `items`; the flexible structural part is a JSON
envelope column (the `artifacts.body` precedent — no per-field table, keeps the
future multi-section relaxation a JSON change, not a migration).

```
document_schemas (
  id             TEXT PRIMARY KEY,   -- UUIDv7 (custom) or a FIXED built-in UUID
  kind           TEXT NOT NULL,      -- "estimate" | "hoa_addendum" | …
  label          TEXT NOT NULL,      -- "Estimate", "HOA Addendum"
  number_prefix  TEXT NOT NULL,      -- "EST", "HOA" — core mints <prefix>-NNNN
  trade_key      TEXT,               -- "landscape" | … | NULL (template-agnostic, e.g. report)
  sections       TEXT NOT NULL,      -- JSON envelope (below)
  schema_version INTEGER NOT NULL,   -- shape version of the `sections` JSON (starts 1)
  created_at     INTEGER NOT NULL,
  updated_at     INTEGER NOT NULL,
  device_id      TEXT NOT NULL,
  deleted_at     INTEGER
)
CREATE INDEX idx_document_schemas_kind ON document_schemas(kind) WHERE deleted_at IS NULL;
```

`sections` JSON envelope (validated at SAVE, never coerced at build):

```jsonc
{
  "total_kind": "sum",           // "sum" | "static"
  "total_label_key": "total",    // free label key (Swift owns copy)
  "sections": [
    { "key": "line_items", "kind": "line_items", "label": "Items",
      "priced": true, "fields": [] },
    { "key": "approvals", "kind": "filled", "label": "Approvals", "fields": [
        { "key": "hoa_no", "kind": "text", "label": "HOA approval #", "fill": "walk" }
    ]},
    { "key": "terms", "kind": "static", "label": "Terms", "fields": [
        { "key": "terms_body", "kind": "static", "label": "Terms",
          "fill": "static", "static_value": "Valid for 30 days." }
    ]}
  ]
}
```

Validation sets (Stage 3, R6 reject-unknown at save):

- `Section.kind ∈ { line_items, static, filled }` — the three NON-GOAL-bounded
  kinds; **exactly one `line_items` section** per schema in v1 (reject 0 and
  2+). Named future relaxations: 0 line_items (item-less docs), 2+ line_items
  (labor-vs-materials).
- `Field.kind ∈ { line_items, text, long_text, currency, quantity, date,
  static }`
- `Field.fill ∈ { walk, manual, static }`

**Built-in schema shape** — one `line_items` section only (no `filled`/`static`
fields; those fold in later as sac's `DocumentLayout` migration). This is what
makes launch-safety trivial: built-ins have zero `filled` fields, so the fill
pass makes **zero** LLM calls beyond today's pricing pass.

| id (FIXED) | kind | trade_key | number_prefix | line_items.priced | total_kind / label |
|---|---|---|---|---|---|
| `…0001` | estimate | landscape | EST | **true** | sum / total |
| `…0002` | invoice | landscape | INV | **true** | sum / total |
| `…0003` | work_order | landscape | WO | false | sum / total |
| `…0004` | condition | property | COND | false | sum / total |
| `…0005` | move_out | property | MO | false | sum / total |
| `…0006` | inspection | inspection | IR | false | **static / findings** |
| `…0007` | report | *NULL* | DOC | false | sum / total |

Fixed ids are UUIDv7-shaped constants `00000000-0000-7000-8000-0000000000NN`
(version nibble `7`, variant `8`) so they sort first and read as built-in.
`priced` / `total_*` per built-in reproduce `is_pricing_kind` / `total_shape`
EXACTLY — pinned in Stage 1.

---

## 4. Resolution & the legality gate (byte-identical by construction)

`build_document(session_id, kind)` keeps its signature. Internally:

1. **Legality gate (UNCHANGED for built-ins).** A kind is legal iff
   `doc_kinds_for_template(template).contains(kind)` **OR**
   `store.has_active_schema(kind, template)` (a custom, trade-matched schema).
   For built-ins the first clause answers identically to today, so every
   existing "illegal kind" test is preserved verbatim (e.g. `condition` stays
   illegal for a landscape session — its schema is `trade_key=property`, so the
   second clause is also false).
2. **Resolve the active schema** for `(kind, template)`:
   ```sql
   SELECT … FROM document_schemas
   WHERE kind = ?kind
     AND (trade_key = ?template OR (trade_key IS NULL AND ?template IS NULL))
     AND deleted_at IS NULL
   ORDER BY updated_at DESC LIMIT 1
   ```
   `report` (trade NULL) resolves ONLY for a None-template session — so it stays
   illegal on a landscape session, matching today. A legal kind with no
   resolvable schema (an operator tombstoned a built-in) → `CoreError::
   InvalidState` (truthful failure, R7) — **never** a silent hardcoded fallback
   (a fallback would resurrect a deleted built-in). At launch this branch is
   unreachable (all built-ins live, editor unshipped).
3. **Render `line_items`** deterministically from the resolved schema's
   `line_items.priced` (replaces the `is_pricing_kind` call). One line per item,
   `section` stays `null` (exactly one line_items section in v1 → no grouping).
4. **Price** iff `line_items.priced && !items.is_empty()` — today's `price_items`
   pass, unchanged (degrade → `queued`, R7).
5. **Fill** iff the schema has ≥1 `filled` field — ONE focused `fill_fields`
   pass (Stage 5). Built-ins have none → skipped → byte-identical. A provider
   `Err` on this pass sets `queued = true`, mirroring the pricing degrade
   (`document.rs`:337) — ONE consistent meaning: "a model call this build needed
   didn't complete; regenerate to retry." A model-*declined* field (the call
   succeeded, the field is simply absent) is a truthful gap and does NOT set
   `queued`.
6. **Total** from the schema envelope `total_kind`/`total_label_key` (replaces
   `total_shape`).
7. **Number** — mint the per-kind integer (unchanged `document_sequences`
   keyed by `doc_kind`) and stamp `number_prefix` from the resolved schema row
   into the payload (additive).

`is_pricing_kind` / `total_shape` / `doc_kinds_for_template` stay in
`pipeline/mod.rs` — they still serve Swift button chrome + the legality gate's
first clause. The build path stops *calling* `is_pricing_kind`/`total_shape`;
the schema row is the source of truth.

---

## 5. Staged TDD plan

Each stage is independently green (`cargo test --workspace` + `cargo clippy
--workspace --all-targets -- -D warnings`). Every stage is red-first: write the
named test, watch it fail, implement.

### Stage 1 — domain type + `document_schemas` table + seeded built-ins (v7)

New: `crates/murmur-core/src/domain.rs` — `DocumentSchema`, `SchemaSection`,
`SchemaField` structs (+ `serde` derive), the const validation arrays
`VALID_SECTION_KINDS` / `VALID_FIELD_KINDS` / `VALID_FILL_KINDS`, the seven fixed
built-in id consts, and `builtin_schemas() -> Vec<DocumentSchema>` (the ONE
source of the table above). Export via `lib.rs`.

New: `crates/murmur-core/src/store/schemas.rs` (module wired into
`store/mod.rs`). Contains `seed_builtin_schemas(conn, ...)`.

Edit: `store/migrations.rs` — append v7, **the TABLE only** (no inline seed
SQL — the migration framework is pure SQL strings, `migrate_with` iterates
`&[&str]` and `execute_batch`es each, and it runs BEFORE `device_id`/clock exist
at `store/mod.rs`:47-50, so there is no Rust hook at v7):
```sql
CREATE TABLE document_schemas ( … as §3 … );
CREATE INDEX idx_document_schemas_kind ON document_schemas(kind) WHERE deleted_at IS NULL;
```

Edit: `store/mod.rs::from_connection` — after `migrations::migrate(&conn)?`, call
`schemas::seed_builtin_schemas(&conn)?`. This is where the seed belongs, not the
migration: it runs after the table exists, and the seed uses **pinned literals**
(no clock/device_id needed) so its position ahead of the `with_clock` override is
irrelevant. `seed_builtin_schemas` iterates `builtin_schemas()` — the ONE source
of truth, no inline-SQL duplicate — and for each does a **parameterized**,
tombstone-respecting insert:
```sql
INSERT INTO document_schemas (id, kind, …, created_at, updated_at, device_id, deleted_at)
SELECT ?, ?, …, ?, ?, ?, NULL
WHERE NOT EXISTS (SELECT 1 FROM document_schemas WHERE id = ?);
```
Every seeded row uses a **SENTINEL `device_id` identical on every device** (pin
the literal: `"builtin"`) so built-ins share fixed UUIDs *and* a fixed origin —
together the stable sync merge key that lets two devices converge on "the same
built-in" rather than duplicating it. `created_at`/`updated_at` are **fixed
literals** (pin: `0`) so a seeded row is byte-identical on every device.

The `WHERE NOT EXISTS` checks **every** row incl. tombstoned — a fixed-id row
that was soft-deleted blocks re-seed. Because `seed_builtin_schemas` runs on
**every** `Store::open` (not once per `user_version`), the resurrection guard is
LIVE on every launch: a new built-in added to `builtin_schemas()` seeds naturally
on the next open, and a deleted built-in stays deleted forever (WE-A). No future
migration is needed to add a built-in.

Tests (`store/schemas.rs` + `store/migrations.rs`):
- `fresh_store_is_at_schema_v7` — replaces the v6 pin.
- `v7_seeds_exactly_the_seven_builtins` — ids + kinds + trade_keys + prefixes.
- `seeded_rows_deep_equal_builtin_schemas` — read every seeded row back and
  assert it deep-equals the corresponding `builtin_schemas()` element (the guard
  that the parameterized INSERT and the `Vec` source never drift; catches the
  SQL/Vec divergence an inline-SQL duplicate would have risked).
- `builtin_schemas_reproduce_todays_pricing_and_total_shape` — for every kind,
  `line_items.priced == is_pricing_kind(kind)` and the envelope
  `total_kind/label == total_shape(kind)`. (The parity net between the old
  hardcoded functions and the seeds.)
- `tombstoned_builtin_survives_a_fresh_seed_call` — insert, tombstone `…0001`,
  re-run `seed_builtin_schemas`; `…0001` stays tombstoned, not resurrected
  (WE-A core — the guard exercised the way every real launch exercises it).

### Stage 2 — store CRUD + resolution

`store/schemas.rs` methods (mirror `store/items.rs` discipline):
`list_document_schemas(trade_key: Option<&str>)`, `get_document_schema(id)`,
`save_document_schema(&DocumentSchema)` (upsert by id: insert or COALESCE-update,
bump `updated_at`; validation is Stage 3), `remove_document_schema(id)`
(tombstone, `NotFound` on a second remove), `resolve_active_schema(kind,
template)` (§4 query), `has_active_schema(kind, template)`.

`list_document_schemas(Some(t))` returns rows with `trade_key = t OR trade_key
IS NULL`, live only, ordered by id (built-ins first). `None` → all live.

Tests:
- `list_filters_by_trade_and_includes_null_trade_and_hides_tombstones`
- `save_upserts_by_id_and_bumps_updated_at_preserving_created_at`
- `remove_tombstones_then_second_remove_is_not_found`
- `resolve_prefers_newest_and_matches_kind_plus_trade`
- `resolve_report_only_for_none_template_not_for_landscape` (parity guard)
- `resolve_returns_none_for_a_tombstoned_builtin` (resurrection consequence)
- `we_a_reopen_over_a_tombstoned_builtin_yields_the_pinned_surviving_set`
  (WE-A end-to-end: tombstone a built-in, reopen the store — re-running the seed
  — assert the exact surviving live set; see §6).

### Stage 3 — save-time validation (R6, reject-never-coerce)

`store/schemas.rs::validate_schema(&DocumentSchema) -> Result<(), CoreError>`
called at the top of `save_document_schema`, BEFORE any write. Rejects: unknown
section kind, unknown field kind, unknown fill, ≠1 `line_items` section, empty
`kind`/`label`/`number_prefix`. Error text mirrors `items.rs`'s allowlist
message ("invalid field kind 'X'; must be one of: …"). Nothing is written on
rejection (validate precedes the INSERT; no partial state).

Tests:
- `reject_unknown_section_kind_nothing_persisted`
- `reject_unknown_field_kind_nothing_persisted` (WE-D core)
- `reject_unknown_fill`
- `reject_zero_line_items_sections` / `reject_two_line_items_sections`
- `valid_custom_schema_saves_and_round_trips`

### Stage 4 — schema-driven `build_document` + THE LAUNCH-SAFETY INVARIANT

Edit `pipeline/document.rs::DocumentBuilder::build` to §4 (resolve schema; render
from `line_items.priced`; total from envelope). Edit `pipeline/mod.rs` legality
gate to the §4 union. Add `render_structure_document` overload/param so `is_gap`
comes from `priced` (keep the `GapPolicy` type + its N2 parity tests intact).

**Gated acceptance criterion (launch-safety, Δ=0):** every existing test in
`pipeline/document.rs`, `ffi/document_build.rs`, and `ffi/convert.rs` passes
**UNMODIFIED**. Do not touch their assertions. If one needs editing, the seam
changed built-in output and the stage FAILS.

New tests:
- `build_resolves_the_seeded_schema_for_every_trade_kind` — drives a processed
  session per template and asserts a document lands for each legal kind.
- `builtin_output_is_byte_identical_per_trade_kind` — a golden: for each of the
  7 built-ins, build over a fixed 2-item set and assert the decoded
  `DocumentPayload`'s today-existing fields (doc_kind, total_kind,
  total_label_key, static_total_cents, queued, and per-line title/detail/qty/
  amount_cents/section/is_gap/item_id) equal the pre-refactor values. (`id`/
  `doc_number` excluded — already non-deterministic pre-refactor.)
- `build_errors_when_the_builtin_schema_was_tombstoned` — tombstone the estimate
  schema, `build_document(estimate)` → `EngineError::Document`, no resurrection.

### Stage 5 — `fill_fields` pass + payload `number_prefix` + `fields[]`

`pipeline/document.rs::fill_fields(provider, filled_fields, items, summary,
max_tokens, usage) -> Result<HashMap<String,String>, HarnessError>` — the exact
`price_items` twin (including its degrade contract): ONE forced structured-output
call; input is the **items + session summary only (never the transcript, R6)**;
the items block reuses `format_pricing_items` (ONE item-formatting helper, no
divergent shape); forced tool `fill_fields` with `{ fields: [{ key, value }] }`.
Echo-and-validate against the schema's `filled` field keys, first-wins dedup,
drop unknown keys. Accumulate usage before judging (R9). Two failure modes,
mirroring pricing: a provider `Err` carries no usage and returns `Err`; a
completed response whose tool block is **unparseable/absent** returns
`Err(HarnessError::Provider(…))` after `usage.add` (R9 — it cost tokens), exactly
as `price_items` does (`document.rs`:186-188). A tool block that IS present but
simply omits a field is NOT an error — that field is a truthful gap.

`build`: after pricing, if the schema has ≥1 `filled` field, run `fill_fields`,
matching on its result exactly like the pricing pass (`document.rs`:334-337):
`Ok(map)` → use the values; **`Err(_)` → `queued = true`** (a model call this
build needed didn't complete — regenerate to retry), and every `walk`/`manual`
field with no value degrades to a gap. Then assemble the `fields[]` output — one
entry per authored `filled`/`static` field in schema order: `static` →
`static_value` (is_gap false); `walk`/`manual` → `fill_fields` value if present
(is_gap false) else `value=null, is_gap=true`. `manual` fields are always gaps in
v1 (operator completes at review — no LLM). Note the two distinct meanings: a
**model-declined** `walk` field (call succeeded, field omitted) is a gap with
`queued` UNCHANGED; a **fill-call failure** (provider `Err` or unparseable) sets
`queued=true` AND its fields fall to gaps.

FFI surface (`ffi/document.rs`): add `DocumentPayload.number_prefix:
Option<String>` and `DocumentPayload.fields: Vec<DocField>` where `DocField {
section_key, key, label, kind, fill, value: Option<String>, is_gap }`. Both are
**additive**; built-ins emit `fields: []` and today's prefix, so Stage 4's
byte-identical net still holds on the shared fields. `ffi/convert.rs` decodes
them (default: absent `number_prefix` → None, absent `fields` → []). Body stamps
`number_prefix` + `fields` in `build`; the pre-existing keys are untouched.

Tests:
- `fill_fields_echoes_and_validates_and_drops_unknown_keys`
- `fill_fields_fed_items_and_summary_never_the_transcript` (R6 — assert the
  request text contains the summary, never a transcript header)
- `omitted_field_renders_as_a_gap_row` (R6 — WE-B F2)
- `static_field_passes_through_its_value`
- `manual_field_is_always_a_gap_in_v1`
- `fill_call_failure_sets_queued_and_degrades_fields_to_gaps` (R7 — provider
  `Err`; assert `queued == true` and the affected fields are gaps, never a hard
  build failure — the pricing-degrade posture)
- `model_declined_field_is_a_gap_without_queued` (the call succeeded but omitted
  the field; assert the field is a gap and `queued == false`)
- `custom_schema_full_render` (WE-B — lines + fields + static, exact)
- `number_prefix_comes_from_the_schema_row_across_interleaved_builds` (WE-C)
- `builtins_emit_empty_fields_and_todays_prefix` (byte-identical guard)

### Stage 6 — the custom-schema fill-quality eval

New hermetic test `crates/evals/tests/document_fill_quality.rs` (mirrors
`carried_scenarios.rs`: `MockProvider`, drives the real `DocumentBuilder`). A
processed session from a small corpus transcript + a saved custom schema whose
`filled` field is **not** mentioned in the transcript. Measures fill quality =
gap-row posture under R6 under-extraction bias:

- `custom_field_absent_from_the_transcript_renders_as_a_gap` — the mock returns
  the tool with NO entry for the unmentioned field; assert the rendered
  `fields[]` row is `value=null, is_gap=true` — the model declined, the seam did
  not fabricate.
- `custom_field_stated_in_the_transcript_is_filled` — the positive control: a
  mentioned field's mock value renders `value=…, is_gap=false`.

(No grader change — the grader scores extraction, not documents; this is a
characterization pin of the fill contract, the honest analog of the R6
distractor-FP signal.)

### Stage 7 — FFI CRUD + real-core gate (dam)

New `crates/ffi/src/schemas.rs` (mirror `ffi/vocabulary.rs` + `ffi/items.rs`):
```
list_document_schemas(trade_key: Option<String>) -> Vec<DocumentSchema>
save_document_schema(schema: DocumentSchema) -> DocumentSchema   // upsert by id
remove_document_schema(id: String)                               // tombstone
```
`DocumentSchema`/`SchemaSection`/`SchemaField` become `uniffi::Record`s with
**String** kinds (deliberate: strings let the app send a bad kind and get the
exact R6 error — an enum would make an unknown unrepresentable and untestable).
New `EngineError::Schema(String)` in `ffi/engine.rs`. Wire `ffi/lib.rs`.

Tests (`ffi/schemas.rs`):
- `ffi_save_list_remove_round_trip`
- `ffi_save_rejects_unknown_field_kind_nothing_persisted` (WE-D end-to-end:
  count schemas before/after == equal)
- `ffi_build_document_unchanged_for_builtins` (payload parity through the FFI)

**Real-core gate (dam — FFI surface changed):**
- `cargo test --workspace` + `cargo clippy --workspace --all-targets -- -D
  warnings` green.
- `cd apps/ios && ./build-ffi.sh` (regenerates the UniFFI bindgen for the 3 new
  methods + new records + the 2 additive payload fields), `./generate.sh`,
  then `xcodebuild … -scheme SitewalkGallery … build` **outside** the Nix shell.
- Demo-path parity: a clean `xcodegen generate` (no `MurmurCoreFFI` dep) still
  builds — `#if canImport(MurmurCoreFFI)` compiles the new surface out, so the
  fresh-clone demo build is unaffected.

---

## 6. Arithmetic-pinned worked examples (hand-recompute against real code)

### WE-A — reopen (re-seed) over a tombstoned built-in → exact surviving set

Pin ids: estimate=`…0001`, invoice=`…0002`, work_order=`…0003`,
condition=`…0004`, move_out=`…0005`, inspection=`…0006`, report=`…0007`.

The guard is NOT a migration-only event: `seed_builtin_schemas` runs on **every**
`Store::open` (from `from_connection`, after migrate), so this trace is what
every launch does — adding a built-in to `builtin_schemas()` needs no migration,
and a deleted built-in stays deleted across every subsequent open.

1. First open → v7 creates the table, `seed_builtin_schemas` seeds `{0001..0007}`
   live (each `INSERT … SELECT … WHERE NOT EXISTS(id)` fires once).
   `list_document_schemas(Some("landscape"))` = trade ∈ {landscape, NULL} =
   **`[0001 estimate, 0002 invoice, 0003 work_order, 0007 report]`**.
2. `remove_document_schema("…0001")` → `0001.deleted_at` set.
3. A later app-update adds built-in `…0008` punch_list (landscape, prefix PUN) to
   `builtin_schemas()`. On the **next open**, `seed_builtin_schemas` re-runs over
   the full (now eight-element) `builtin_schemas()`:
   - `…0001`: a row with that id EXISTS (tombstoned) → `WHERE NOT EXISTS` is
     false → **not re-inserted, stays tombstoned** (resurrection defused).
   - `…0002..0007`: exist live → skipped.
   - `…0008`: no such id → inserted live.
4. Post-reopen live set = **`{0002,0003,0004,0005,0006,0007,0008}`**; `0001`
   remains tombstoned. `list_document_schemas(Some("landscape"))` =
   **`[0002 invoice, 0003 work_order, 0007 report, 0008 punch_list]`** —
   estimate ABSENT. This is the acceptance criterion the review demanded.

### WE-B — custom schema (mixed sections) → exact fill prompt + rendered doc

Custom schema: kind `hoa_addendum`, trade `landscape`, prefix `HOA`,
`schema_version 1`, envelope `total_kind="sum"`, `total_label_key="total"`,
sections:
- S1 `line_items` (`priced=false`)
- S2 `filled` "Approvals": F_a `hoa_no` (text/walk, label "HOA approval #"),
  F_b `reviewed_by` (text/walk, label "Reviewed by")
- S3 `static` "Terms": F_c `terms_body` (static, value "Valid for 30 days.")

Pinned processed session (landscape), items in order:
- I1 `todo` "Install boxwood hedge" (id `item-A`, right "")
- I2 `part` "bark mulch" (id `item-B`, right "3 CU YD")

Summary artifact: `"Walked the front yard; HOA approval 41827 on file."`

**Assembled `fill_fields` user message (exact):**
```
Fill these document fields from the session. Put a value only on a field whose
answer was clearly stated — omit any field you are unsure about; a blank field
is cheaper than a wrong one.

Fields:
- [hoa_no] HOA approval #
- [reviewed_by] Reviewed by

Session items:
- [todo] Install boxwood hedge (item_id: item-A)
- [part] bark mulch (item_id: item-B)

Session summary:
Walked the front yard; HOA approval 41827 on file.
```
The `Session items` block is `format_pricing_items(items)` verbatim (the same
helper the pricing pass uses — `document.rs`:116-122: `- [{kind}] {text} (item_id:
{id})`), NOT a fill-specific shape; only the surrounding `Fields:` / `Session
summary:` framing is the fill prompt's own (pinned here). Note `right_text`
("3 CU YD") is deliberately absent — `format_pricing_items` omits it, and the
fill pass does not re-add it.

Model returns `fill_fields { "fields": [ {"key":"hoa_no","value":"41827"} ] }`
— `reviewed_by` omitted (not stated → R6).

**Rendered document (exact):**
- `line_items` (priced=false → is_gap false, today's non-pricing posture):
  - L1 `{ title:"Install boxwood hedge", detail:"", qty:"", amount_cents:null,
    section:null, is_gap:false, item_id:"item-A" }`
  - L2 `{ title:"bark mulch", detail:"", qty:"3 CU YD", amount_cents:null,
    section:null, is_gap:false, item_id:"item-B" }`
- `fields[]` (schema order):
  - F_a `{ section_key:"approvals", key:"hoa_no", label:"HOA approval #",
    kind:"text", fill:"walk", value:"41827", is_gap:false }`
  - F_b `{ section_key:"approvals", key:"reviewed_by", label:"Reviewed by",
    kind:"text", fill:"walk", value:null, is_gap:true }`  ← **the gap row (R6)**
  - F_c `{ section_key:"terms", key:"terms_body", label:"Terms",
    kind:"static", fill:"static", value:"Valid for 30 days.", is_gap:false }`
- `total_kind:"sum"`, `total_label_key:"total"`, `number_prefix:"HOA"`,
  `doc_number:1` → rendered `HOA-0001`.

### WE-C — numbering across two custom kinds + an existing kind

Schemas present (all landscape): estimate (EST, built-in), hoa_addendum (HOA),
punch_list (PUN). `document_sequences` keyed by `doc_kind`, independent, start
at 0, mint = prev+1. Interleaved builds:

| # | build kind | seq before→after | minted int | `number_prefix` | rendered |
|---|---|---|---|---|---|
| 1 | estimate     | 0→1 | 1 | EST | EST-0001 |
| 2 | hoa_addendum | 0→1 | 1 | HOA | HOA-0001 |
| 3 | estimate     | 1→2 | 2 | EST | EST-0002 |
| 4 | punch_list   | 0→1 | 1 | PUN | PUN-0001 |
| 5 | hoa_addendum | 1→2 | 2 | HOA | HOA-0002 |
| 6 | estimate     | 2→3 | 3 | EST | EST-0003 |

Core-side, `number_prefix` is read from each resolved schema row (not a Swift
switch) and stamped into the payload; the integer is the unchanged per-kind
`document_sequences` counter; `%04d` render. **The rendered column is the state
once Swift consumes `payload.number_prefix`** — TODAY, `MurmurEngineFormatting.
docNumberLabel` (`apps/ios/…/MurmurEngineFormatting.swift`:63) is a hardcoded
`switch` over `docKind` with `default → "DOC"`, so a custom `hoa_addendum` would
render **`DOC-0001`, not `HOA-0001`**, even though the core already emits
`number_prefix:"HOA"`. Likewise `DocKinds.legalKinds/label/stamp` (`apps/ios/
Sources/Fixtures/Fixtures.swift`:96+) are hardcoded built-in mirrors, so the
core's legality UNION never surfaces a custom kind's button in the UI. This is
**launch-safe today**: every built-in maps correctly through those hardcoded
switches (`estimate→EST`, …), and v1 ships on built-ins only, so the payload's
`number_prefix` is redundant-but-correct for built-ins. Custom kinds render
correctly only after the sac follow-ups below.

**sac follow-ups (editor milestone — NOT this plan):**
- `MurmurEngineFormatting.docNumberLabel` → read `payload.number_prefix` instead
  of the hardcoded `docKind` switch (fall back to the switch only when the field
  is absent, for pre-Rev-2 payloads).
- `DocKinds.legalKinds` (and `label`/`stamp`) → drive the kind list from the seam
  (`list_document_schemas(trade)`), not the hardcoded per-template arrays, so
  custom kinds surface their buttons + copy.

### WE-D — save with an unknown field kind → exact error, nothing persisted

`save_document_schema` with S1 `line_items` (valid) + S2 `filled` containing a
field `{ key:"b", kind:"barcode", label:"B", fill:"walk" }`:
- `validate_schema` walks fields; `"barcode" ∉ VALID_FIELD_KINDS` →
  `Err(CoreError::InvalidState("invalid field kind 'barcode'; must be one of:
  line_items, text, long_text, currency, quantity, date, static"))`, surfaced by
  FFI as `EngineError::Schema(…)`.
- The INSERT is never reached → `list_document_schemas` count is **unchanged**
  (equal before and after). Same posture for an unknown section kind
  (`"gallery" ∉ VALID_SECTION_KINDS`). R6: rejected at SAVE, never coerced at
  build.

---

## 7. Risks & rollback

- **Launch-safety regression (highest).** Mitigation: Stage 4 forbids editing
  any existing document/build/convert test; `builtin_output_is_byte_identical_
  per_trade_kind` is the golden. If it moves, the seam is wrong — halt.
- **Resurrection of a deleted built-in.** Mitigation: WE-A test + the
  `WHERE NOT EXISTS(id)` seed guard covering tombstoned rows. Acceptance
  criterion, not prose.
- **Editing a schema after documents were built from it.** Per CANON, document
  artifacts are **derived snapshots (burn-per-tap)** — immutable once written.
  Editing a schema affects only FUTURE `build_document` calls; a prior
  document's stored body is untouched. Regenerate is explicit (the operator taps
  build again). No back-migration of old snapshots. This is a feature, not a
  bug: an invoice already sent must not silently reshape.
- **A `filled` field the model fabricates.** Mitigation: `fill_fields` echoes +
  validates keys, drops unknowns, and omission → gap (R6). Stage 6 eval pins
  the gap-row posture on an unmentioned field.
- **UniFFI surface drift.** New records/methods + 2 additive payload fields
  change the generated bindings. Mitigation: Stage 7 real-core gate
  (`build-ffi.sh` + `generate.sh` + xcodebuild) and the demo-path
  compile-out parity check.
- **Rollback:** the whole seam is additive behind seeded built-ins. Reverting
  the v7 migration is not safe once shipped (never edit a shipped migration),
  but the *behavior* is trivially revertible: point `build` back at
  `is_pricing_kind`/`total_shape` — the built-in rows become inert. The FFI CRUD
  methods are unreferenced by the demo path, so a partial revert leaves the app
  building on built-ins exactly as v1 intended.

---

## 8. Boundary

Core owns the schema record, resolution, the fill, numbering, sync, and R6
save-time validation (dam). The authoring editor, schema-driven rendering, and
the `DocumentLayout`→`static`-fields migration are sac's, built on this seam
during dam's absence. The seam is the one serial dependency; everything after it
is app-side. v1 ships on seeded built-ins with the app looking identical —
customization flips on when sac's editor lands.
