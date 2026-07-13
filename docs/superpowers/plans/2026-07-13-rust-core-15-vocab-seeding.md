# Murmur Rust Core — Plan 15: vocabulary seeding

> **For agentic workers:** REQUIRED SUB-SKILL: use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax. The Rust tasks (1–3) are **hermetic**: in-memory `Store`, `MockProvider`, `MurmurEngine::with_providers`, a `SpyStore` for memory persistence — **no model, no `whisper` feature, no network, no mic**. `cargo test --workspace` must NEVER require the `whisper` feature or a model file (the load-bearing CI invariant). The Swift task (4) is **not CI-gated for logic** (the iOS **demo** build IS CI-gated; real-core is dam-manual) and the **vocab-card visual design / chip layout / onboarding placement is explicitly sac's** — `// sac:` markers throughout. Run `cargo`/`xcodegen` **inside** the Nix dev shell; run `xcodebuild` **outside** it (Nix linker env breaks Xcode `ld`). Never read `.env` or `project.local.yml`.
>
> **⚠ SHIPPABILITY — merging this plan auto-publishes the TestFlight internal lane on real-engine** (CANON 2026-07-10). main must build the **real-core archive** at every merge. Plan 15 is **additive** (a new `seed_vocabulary` FFI method + a new `SeedReport` record + a non-rendered internal memory section), so it is **ONE PR** — but the real-core compile is a **MANDATORY manual gate** (Task 5; CI cannot build real-core). See **§Staging**.
>
> **Design source (AGREED — do not relitigate):** `docs/design/2026-07-07-onboarding-vocabulary-seeding.md` (merged, PR #181). The six open questions there are **closed** by the 2026-07-10 decision round (`docs/design/2026-07-10-decisions-notes-first.md` §"Vocab seeding", carried in `meta/CANON.md`): **JSON packs are sac-curated & bundled on-device** (CI schema test), **the demo walk comes BEFORE the vocab card in onboarding**, **type-only interview v1**, **SEED_MAX ≈ 60**, all writes through the existing Plan 12 funnel, scheduled **after Plan 13** (done) — Plan 14 (done) landed in between. This plan **implements** that agreed design; it does not reopen it.
>
> **Plan review pending.** The adversarial reviewer must hand-recompute WE-A…WE-E below against the real funnel (`Memory::add_vocabulary_term`, `crates/harness/src/memory/mod.rs`) and confirm every count.

**Goal.** A brand-new user's `vocabulary` section is empty, so their **first** walk gets zero STT biasing — exactly when transcription is worst (unknown crew names, local place names, trade jargon) and a bad transcript costs the most trust. Plan 15 lets onboarding **seed** the section from a **sac-curated per-trade pack** (confirmed by the user, never silent) plus free-form terms, so the biasing loop starts **warm** instead of cold. Every seeded term flows through the **existing** `add_vocabulary_term` funnel and lands in the **one** memory section that both STT biasing (`collect_bias_terms` at `begin_walk`) and the LLM context (`Memory::to_prompt`) already read — so a single write reaches both consumers with **zero new plumbing**.

**What lands (all in ONE PR):**

1. **harness (`Memory`) — an internal, non-rendered section for seed bookkeeping.** A `_`-prefixed section convention (excluded from `to_prompt`/`render`, `word_count`, `clamp_to_cap`, `prune_stale`) plus `mark_pack_seeded`/`is_pack_seeded` on a `_seeds` marker section. This makes seeding **idempotent** and **deletion-durable** without touching `MemoryEntry` or the 100-term cap. (Task 1.)
2. **ffi — `seed_vocabulary(trade, version, terms) -> SeedReport`.** A thin batch orchestrator over the **existing** `add_vocabulary_term(term, now, Stated)` funnel: idempotent per `trade:version` (marker-guarded), bounded by `SEED_MAX = 60` per pass, backstopped by the untouched 100-cap, `Stated` provenance, persisted under the same lock discipline as the CRUD methods. Returns exact counts. (Task 2.)
3. **ffi/core — the dual-flow invariant, pinned.** A test proves a seeded term appears in **both** the `begin_walk` STT bias terms **and** the agent's `Memory::to_prompt`, while the `_seeds` marker appears in **neither**. (Task 3.)
4. **apps/ios — the sac seam.** `WalkEngine.seedVocabulary(...)` on both engines; bundled per-trade JSON packs (`// sac:` schema + curation); a Swift `VocabPackTests` schema gate; the onboarding **vocab card after the demo walk** (chips confirm/deselect + the existing free-form add bar). **Card visuals / chip layout / exact placement are sac's** (`// sac:`); this task guarantees the data path. (Task 4.)
5. **Real-core compile + bindings drift (dam-manual) + merge.** (Task 5, MANDATORY gate.)

**What Plan 15 is NOT (see Non-goals for the full list).** No new write path (seeding *only* calls `add_vocabulary_term`). No change to normalize/dedup/cap semantics. No `MemoryEntry` schema change, no new `FactSource` variant, no `Pinned`/protected tier (deferred, Plan 12 D3 escalation). No migration. No auto-harvest of proper nouns from live extraction. No contact/notes import. No server anything (packs are on-device). No voice-capture interview (type-only v1).

---

## Hard dependencies (all DONE, on `main`)

- **Plan 12 — the vocabulary write funnel (LANDED):** `Memory::add_vocabulary_term(term, now, source) -> VocabAdd` (`crates/harness/src/memory/mod.rs:251`) normalizes (`normalize_term`: trim + collapse internal whitespace, **case preserved**), dedups **case-insensitively across both sides keeping first-seen stored casing**, rejects `Empty`/`TooLong` (`MAX_VOCABULARY_TERM_WORDS = 6`), and enforces `MAX_VOCABULARY_TERMS = 100` at write time (`Full` — **reject, never silent-evict**). `remove_vocabulary_term` (`:275`) is case-insensitive. The FFI CRUD (`crates/ffi/src/vocabulary.rs`) wraps this under a lock-then-clamp(`DEFAULT_WORD_CAP`)-then-save discipline, `Stated` provenance, panic-free across FFI. `VocabularyView` (sac) is the editor + revisit surface.
- **Plan 05/06 — the read half (LANDED):** `collect_bias_terms(memory, _template)` (`crates/ffi/src/session.rs:100`) reads `section_texts(VOCABULARY_SECTION)`, `take(max = SttConfig::max_bias_terms = 100)`; `build_bias_prompt(terms, max)` (`crates/stt/src/bias.rs`) makes the whisper `initial_prompt`. `Memory::to_prompt`/`render` (`memory/mod.rs:159`) renders **all** sections into the agent/reflection context. **Both read the same `vocabulary` section** — the seam Plan 15 exploits (D8-15).
- **Plan 13/14 (LANDED):** notes-first core + comprehensive notes. Irrelevant to seeding except that finish()/process() are stable.
- **iOS onboarding (LANDED):** `OnboardingFlow.swift` (WELCOME → YOUR BUSINESS[trade] → MIC), `BusinessProfile.swift` (`tradeKey ∈ {landscape, property, inspection}`, UserDefaults), `WalkEngine.swift` protocol with `listVocabulary`/`addVocabularyTerm`/`removeVocabularyTerm`, `DemoWalkEngine` in-memory vocab, `AppModel.vocabulary`. `OnboardingFlow.swift` carries a `TODO(#181)` placeholder in the MIC step — **superseded** by D9-15 (card moves to *after* the demo walk).

**Verified API facts (checked against source, not guessed):**
- `add_vocabulary_term` returns `VocabAdd { Added | Duplicate | Full | Empty | TooLong }` (`memory/mod.rs:30`). A `Duplicate` **touches provenance on the stored casing and consumes no slot** (`:259–264`); it never adds a second variant. **Case of an existing term is not overwritten** by a later add (both `Stated` ⇒ equal `rank()` ⇒ `remember_from` does not downgrade/rewrite, `:96–110`). This is load-bearing for WE-A ("french drain" stays lowercase).
- `MAX_VOCABULARY_TERMS = 100` and `stt::SttConfig::default().max_bias_terms = 100` are pinned equal by `read_side_cap_matches_the_write_side_constant` (`vocabulary.rs:135`). SEED_MAX must **not** fork either.
- `Memory::render` iterates `self.sections` (BTreeMap, alphabetical) and emits `## name` + `- text` for **every** non-empty section (`memory/mod.rs:159–186`). A marker section would leak into the prompt unless excluded — hence D5-15.
- `clamp_to_cap` (`:210`) evicts across **all** sections by `(source.rank(), last_touched, name)`; `prune_stale` (`:196`) drops stale non-`Corrected` entries across all sections; `word_count` (`:143`) sums all sections. A marker entry must be excluded from all three or idempotency breaks under cap/age pressure — hence D5-15.
- The FFI CRUD lock discipline (`vocabulary.rs:29–58`): mutate under `self.memory.lock()`, `clamp_to_cap(DEFAULT_WORD_CAP)`, `.clone()` the snapshot, drop the lock, `self.memory_store.save(&snapshot)`. `seed_vocabulary` reuses this verbatim.
- `SttConfig`/`build_bias_prompt` read terms **as-is** in insertion order — so seed insertion order is observable in the bias prompt (WE-A pins it).

**Spec basis:** R6 (under-extraction bias — seeding never fabricates terms the user didn't confirm; chips are opt-out, free-form is opt-in), R7 (inspectable & undoable — a `Full`/over-budget seed degrades to a partial add with exact counts, never a crash or a silent eviction; every seeded term is visible and removable in `VocabularyView`), R9 (spend — seeding adds **no** LLM call). Design: `2026-07-07-onboarding-vocabulary-seeding.md`; decisions `2026-07-10-decisions-notes-first.md`; `meta/CANON.md`.

---

## Architecture — decisions, justified (reviewers read these first)

### D1-15. Seeding reuses the existing funnel — no new write path

Every seeded term is written through **`Memory::add_vocabulary_term(term, now, FactSource::Stated)`** — the *same* call the editor uses. `seed_vocabulary` is a thin orchestrator: it does **not** re-implement normalize/dedup/cap/word-guard. This is the design's non-negotiable ("no new write path"), and it means the seeding path inherits, for free, the case-insensitive dedup, the whitespace normalization, the `TooLong` guard, and the hard 100-cap. The only things `seed_vocabulary` adds *around* the funnel are: a per-pass budget (D2), an idempotency marker (D4), and the lock/persist wrapper (already the CRUD pattern).

### D2-15. `SEED_MAX = 60` is a per-pass batch bound, **not** a Memory invariant

The 100-term cap stays the **single hard Memory invariant** (unforked, unchanged — honoring the design's "don't fork the 100-cap"). `SEED_MAX = 60` is a bound on **how many terms one `seed_vocabulary` call may add**, enforced **in the FFI seeding funnel** (the "flow" side of the boundary), not in `Memory`. Rationale: reserve ~40 slots so the reflection loop (Plan 02/12) can keep learning without immediately hitting `Full` (design Q4). Because curated packs are ≤~20 terms (D7), the budget is a **guardrail**, not a normal path — WE-D exercises it deliberately with an oversized pack. The UI also stops offering chips past the budget (a cosmetic mirror, `// sac:`), but the **core enforcement is authoritative** so the invariant is hermetically testable and independent of app state. `const SEED_MAX: usize = 60;` lives in `crates/ffi/src/vocabulary.rs` (the seeding funnel), **not** in `harness` (where `MAX_VOCABULARY_TERMS` lives) — keeping the two constants in separate crates makes it structurally impossible to accidentally fork the cap.

### D3-15. Provenance: seeded terms are `Stated`, indistinguishable from user-typed once confirmed

Seeded terms are written with `FactSource::Stated` — the user **confirmed** them (chips are opt-out; nothing is silent, design Q2), so they *are* user-asserted facts. **No `MemoryEntry` schema change, no per-term "seeded" flag, no new `FactSource` variant.** Consequences, pinned:
- Under cap/age pressure, seeded terms behave **exactly** like any `Stated` term: evicted after all `Inferred`, before `Corrected` (`clamp_to_cap`); a `Stated` seed **can** be pruned by age like any other `Stated` term (only `Corrected` is never auto-pruned). Seeding does **not** change eviction semantics (design Q4).
- "Which terms were seeded" is deliberately **not** tracked per-term. The budget (D2) is per-pass, not "count existing seeds", so no per-term provenance is needed. Deletion durability (D4) uses a **pack-level** marker, not per-term state.
- A `Pinned`/protected tier for seeds (if device testing shows erosion) is the **Plan 12 D3 escalation — deferred, measure first** (Open Question 3).

### D4-15. Idempotency & tombstone: a pack-level applied-marker, not per-term presence

`seed_vocabulary(trade, version, terms)` records an applied-marker `"{trade}:{version}"` in the internal `_seeds` section (D5). Semantics, pinned:
- **Same `trade:version` seeded twice ⇒ no-op** (marker present ⇒ return early, `added:0`, `already_seeded:true`). Seeding is idempotent.
- **Deleting a seeded term is durable — re-seeding does NOT resurrect it** (WE-B). Because the re-seed is **marker-guarded**, not presence-guarded: the second call short-circuits before touching the funnel, so a term the user removed via `VocabularyView` stays gone. This is the "tombstone" answer without a tombstone table: the *pack* is the unit of idempotency, not the term.
- **A `version` bump re-runs** (marker `trade:2` ≠ `trade:1`), applying the new pack as a delta through the funnel (existing terms dedup; genuinely-new curated terms add). This is intended: a pack-content revision by sac is a deliberate re-seed. (A user-deleted term that is still curated in the new version *may* re-appear on a version bump — acceptable, since only user-confirmed chips reach `seed_vocabulary` and the funnel dedups; noted in Open Question 2.)

### D5-15. Internal `_`-prefixed sections — the marker never leaks and never evicts

The `_seeds` marker rides the **existing** `Memory.sections` map (no new storage, no migration — section names are free-form strings, exactly as `session_meta` rode the artifact table in Plan 14). To keep it out of the agent context and safe from eviction, a one-line convention: **a section whose name starts with `_` is internal** — excluded from `render`/`to_prompt`, `word_count`, `clamp_to_cap` (never an eviction candidate), and `prune_stale` (never aged out). `vocabulary_terms()` already reads only `VOCABULARY_SECTION`, so it is unaffected. **No existing section is named with a `_` prefix**, so this is behavior-preserving for all current data (asserted by the unchanged existing tests staying green). One helper `fn is_internal_section(name: &str) -> bool { name.starts_with('_') }` gates all four sites.

### D6-15. Trade change is a **union**, never a replace

Changing trade (landscape→property) calls `seed_vocabulary("property", 1, <property pack>)`, which **adds** the new pack's confirmed terms and **preserves everything already present** — prior-trade seeds *and* user terms (WE-C). We do **not** delete the old trade's seeds. Rationale: once seeded and surviving, a term may be one the user now relies on; we cannot distinguish "stale seed" from "load-bearing term", and destructive removal violates R7. The union grows the section, but packs are small and the 100-cap + reflection headroom absorb it. (If a user churns through all three trades: 3 packs × ~15 ≈ 45 terms — still under 60 and well under 100.)

### D7-15. Packs live in the app bundle (sac-curated JSON); core receives terms via FFI

Per the 2026-07-10 ruling: **bundled JSON packs, sac-curated**, one per trade, in the app bundle. Core owns **no** pack content — `seed_vocabulary` receives the **user-confirmed term subset** (already run through sac's chip UI) as a `Vec<String>` and a `(trade, version)` identity. This keeps trade-jargon curation in sac's lane (copy iteration without a Rust rebuild), keeps the privacy invariant (nothing but LLM calls leaves the device — packs are static on-device assets), and keeps core's surface a small, testable funnel. The JSON **schema is a `// sac:` contract** (D9); a Swift `VocabPackTests` gate validates every bundled pack against it (the "CI schema test" from the ruling; it rides the iOS test target — sac's content, sac's test).

### D8-15. One write, two consumers — the automatic dual-flow

Seeded terms land in the **`vocabulary`** memory section. That section is read by **both**:
- **STT biasing:** `begin_walk` → `collect_bias_terms` → `section_texts(VOCABULARY_SECTION)` → `build_bias_prompt` → whisper `initial_prompt`.
- **LLM context:** `Memory::to_prompt`/`render` emits `## vocabulary` into the agent + reflection prompts.

So a single `add_vocabulary_term` reaches **both** with zero extra plumbing — the seeding value proposition. The `_seeds` marker, being internal (D5), reaches **neither**. Task 3 pins this with an end-to-end test.

### D9-15. Flow: **demo walk BEFORE the vocab card** — sac owns the surface

Per the ruling, the vocab card appears **after** the user's first (demo/guided) walk, not as a linear onboarding wall (design Q1 rationale: the user then has concrete context for "what did it get wrong?"). The card = **suggestion chips** from the trade pack (default-on, tap to deselect — never silent, design Q2) + the **existing** `VocabularyView` free-form add bar. **Done** writes the confirmed chips via `seed_vocabulary(trade, version, confirmedChips)` and the free-form terms via the existing `addVocabularyTerm` (one at a time). **Skip writes nothing** (cold-start fallback = today's behavior exactly). Card **visuals, chip layout, and exact placement in the flow are sac's** (`// sac:`); Task 4 supplies the data path + a minimal reference wiring and removes the stale `TODO(#181)`.

---

## Worked examples (reviewers: hand-recompute against `Memory::add_vocabulary_term`)

Conventions: `normalize_term` = trim + collapse internal whitespace runs to one space, **case preserved**; dedup is **case-insensitive on the normalized form**, keeping **first-seen stored casing**; `Added` consumes a slot, `Duplicate` does not. `SEED_MAX = 60`, cap `= 100`.

**WE-A — landscape seed with a collision + normalization (happy path).**
Pre-state — the user has already typed 3 free-form terms (insertion order): `["Hollis", "Boxwood Lane", "french drain"]` (all `Stated`, count = 3). Trade = `landscape`, version 1. Confirmed pack chips (in order), 12 entries:
`["bark mulch", "French Drain", "boxwood", "zone 2", "  drip  irrigation ", "sod", "retaining wall", "paver", "boxwood", "hardscape", "downspout", "swale"]`

Funnel trace (`added` counter starts 0):

| # | input | normalized | outcome | added | note |
|---|---|---|---|---|---|
| 1 | `bark mulch` | `bark mulch` | **Added** | 1 | new |
| 2 | `French Drain` | `French Drain` | **Duplicate** | 1 | ci-matches user `french drain`; casing **not** overwritten |
| 3 | `boxwood` | `boxwood` | **Added** | 2 | `Boxwood Lane` is different text |
| 4 | `zone 2` | `zone 2` | **Added** | 3 | |
| 5 | `  drip  irrigation ` | `drip irrigation` | **Added** | 4 | whitespace collapsed |
| 6 | `sod` | `sod` | **Added** | 5 | |
| 7 | `retaining wall` | `retaining wall` | **Added** | 6 | |
| 8 | `paver` | `paver` | **Added** | 7 | |
| 9 | `boxwood` | `boxwood` | **Duplicate** | 7 | intra-pack dup of #3 |
| 10 | `hardscape` | `hardscape` | **Added** | 8 | |
| 11 | `downspout` | `downspout` | **Added** | 9 | |
| 12 | `swale` | `swale` | **Added** | 10 | |

`SeedReport = { added: 10, duplicates: 2, skipped_over_budget: 0, skipped_full: 0, already_seeded: false, total: 13 }`. Resulting vocabulary (insertion order, **13 terms**):
`["Hollis", "Boxwood Lane", "french drain", "bark mulch", "boxwood", "zone 2", "drip irrigation", "sod", "retaining wall", "paver", "hardscape", "downspout", "swale"]`
`13 ≤ SEED_MAX(60)` and `13 ≤ 100`. Marker `_seeds` gains `"landscape:1"`. **Dual-flow (D8):** `begin_walk`'s bias prompt = `"Terms used in this session: Hollis, Boxwood Lane, french drain, bark mulch, boxwood, zone 2, drip irrigation, sod, retaining wall, paver, hardscape, downspout, swale."`; `to_prompt` renders `## vocabulary` with the same 13; **`_seeds` appears in neither**.

**WE-B — idempotency + deletion durability (no resurrection).**
Continue from WE-A (13 terms, `landscape:1` marked). User deletes `boxwood` via `remove_vocabulary_term` → **12 terms**. App re-invokes `seed_vocabulary("landscape", 1, <same 12 chips>)`:
- `is_pack_seeded("landscape:1") == true` ⇒ **early return, no funnel calls**. `SeedReport = { added: 0, duplicates: 0, skipped_over_budget: 0, skipped_full: 0, already_seeded: true, total: 12 }`.
- `boxwood` is **NOT** resurrected. Vocabulary stays **12**. Seeding twice = no-op; deletion is durable. ✓

**WE-C — trade change (union, preserve everything).**
Continue from WE-B (12 terms). User switches trade → `property`. App calls `seed_vocabulary("property", 1, ["carpet","blinds","water heater","GFCI","baseboard","drywall","HVAC","walkthrough"])` (8 terms, none collide):
- `is_pack_seeded("property:1") == false` ⇒ apply. All 8 `Added`. `SeedReport = { added: 8, duplicates: 0, skipped_over_budget: 0, skipped_full: 0, already_seeded: false, total: 20 }`.
- Landscape seeds + user terms **preserved** (union). `_seeds = ["landscape:1", "property:1"]`. Total **20** `≤ 60 ≤ 100`.

**WE-D — `SEED_MAX` budget bound (guardrail).**
Empty vocabulary. `seed_vocabulary("inspection", 1, <70 distinct terms>)`:
- Terms 1–60 → `Added` (`added` reaches 60). Terms 61–70 → `added == SEED_MAX` ⇒ **`skipped_over_budget`** (funnel **not** called). `SeedReport = { added: 60, duplicates: 0, skipped_over_budget: 10, skipped_full: 0, total: 60 }`.
- `60 < 100` — the hard cap is never touched; ~40 slots stay free for reflection. Marker `inspection:1` recorded. (Real curated packs are ≤~20, so this path is a test-only guardrail.)

**WE-E — cap backstop + `Full` tolerance (R7).**
Vocabulary already at **98** (e.g. via reflection). `seed_vocabulary("landscape", 1, [5 new terms])`:
- Terms 1–2 → `Added` (count → 100). Terms 3–5 → `add_vocabulary_term` returns `Full` ⇒ **`skipped_full`**. `SeedReport = { added: 2, duplicates: 0, skipped_over_budget: 0, skipped_full: 3, total: 100 }`.
- **No error, no eviction** (R7: never silent-evict; the funnel rejects, seeding tallies). Marker `landscape:1` still recorded (the pack is considered applied). No throw across FFI.

---

## Staging (main stays shippable)

**ONE PR** (`pr/dam/plan-15-vocab-seeding` → main). All-additive: an internal memory section + two `Memory` methods, a new FFI method + `SeedReport` record, a Swift protocol method + pack assets + demo wiring. Gated by `cargo test --workspace` + `clippy --workspace --all-targets -- -D warnings` + iOS **demo** build + **the mandatory dam-manual real-core compile + bindings-drift check (Task 5)**. Rust tasks (1–3) are independently `cargo`-testable before the Swift task (4). A two-stage split buys nothing (the FFI growth is additive; existing Swift ignores the new method until sac wires the card) — one atomic PR keeps main from holding a half-wired seeding contract.

---

## Tasks

### Task 1 — internal `_`-section convention + seeded-pack marker (harness; D4-15/D5-15)
- [ ] **RED** (`crates/harness/src/memory/mod.rs` tests): (a) a section named `"_seeds"` with an entry is **absent** from `to_prompt()` output; (b) its words are **not** counted by `word_count()`; (c) `clamp_to_cap(tiny_cap)` with an over-cap `vocabulary` section **evicts vocabulary entries but never the `_seeds` entry** (assert the marker survives to zero remaining budget); (d) `prune_stale(now, small_age)` with a very old `_seeds` entry **keeps** it (never aged out); (e) `mark_pack_seeded("landscape:1")` then `is_pack_seeded("landscape:1") == true` and `is_pack_seeded("property:1") == false`; marking twice is idempotent (one entry). Add a **regression pin**: a normal section (`"vocabulary"`, `"people"`) is **still** rendered/counted/evictable exactly as before (guards against over-broad exclusion).
- [ ] **GREEN:** add `pub(crate) const SEED_MARKER_SECTION: &str = "_seeds";` and `fn is_internal_section(name: &str) -> bool { name.starts_with('_') }`. Gate the four sites on it: `render` (skip internal), `word_count` (skip internal), `clamp_to_cap` (exclude internal from the candidate iterator), `prune_stale` (skip internal sections in `retain`). Add `pub fn mark_pack_seeded(&mut self, key: &str)` (idempotent `remember_from(SEED_MARKER_SECTION, key, 0, FactSource::Stated, None)` — `now=0` is fine, markers are never aged) and `pub fn is_pack_seeded(&self, key: &str) -> bool` (`section_texts(SEED_MARKER_SECTION).contains(&key)`).
- [ ] **Gate:** `nix develop -c cargo test -p harness` + `nix develop -c cargo clippy -p harness --all-targets -- -D warnings`. (All **pre-existing** memory tests must stay green — proof the exclusion is behavior-preserving for non-`_` sections.)

### Task 2 — `seed_vocabulary` + `SeedReport` (ffi; D1-15/D2-15/D3-15/D4-15/D6-15)
- [ ] **RED** (`crates/ffi/src/vocabulary.rs` tests, reusing the existing `SpyStore` + `engine(...)` harness): encode **WE-A** (`added:10, duplicates:2, total:13`, exact resulting list + order), **WE-B** (re-seed after a delete ⇒ `already_seeded:true`, `boxwood` not resurrected, total 12), **WE-C** (trade change unions to 20, landscape terms preserved), **WE-D** (70-term pack ⇒ `added:60, skipped_over_budget:10`), **WE-E** (fill to 98 first, then `added:2, skipped_full:3`, no throw). Assert each applying seed **persists** (last `SpyStore` save carries the terms) and the `_seeds` marker is set; assert the WE-B no-op path **does not save** (no funnel write). Assert `list_vocabulary()` after WE-A returns the 13-term list unchanged (casing preserved: `french drain` stays lowercase).
- [ ] **GREEN:** add `const SEED_MAX: usize = 60;` (in `vocabulary.rs`, **not** harness — D2). Add `#[derive(uniffi::Record)] pub struct SeedReport { pub added: u32, pub duplicates: u32, pub skipped_over_budget: u32, pub skipped_full: u32, pub already_seeded: bool, pub terms: Vec<String> }` (`total` = `terms.len()`; expose `terms` so the card updates in one round-trip, mirroring the CRUD methods). Add `#[uniffi::export] pub fn seed_vocabulary(&self, trade: String, version: u32, terms: Vec<String>) -> Result<SeedReport, EngineError>`:
  - lock `self.memory`; compute `key = format!("{trade}:{version}")`; if `mem.is_pack_seeded(&key)` ⇒ build `SeedReport{ already_seeded: true, added: 0, .., terms: mem.vocabulary_terms()… }`, drop lock, **return without saving** (idempotent, no write).
  - else iterate `terms`: for each, if `added == SEED_MAX` ⇒ `skipped_over_budget += 1`; else `match mem.add_vocabulary_term(&t, now, FactSource::Stated) { Added => added += 1, Duplicate => duplicates += 1, Full => skipped_full += 1, Empty | TooLong => { /* curated packs are pre-validated by VocabPackTests (Task 4) → unreachable in practice; drop silently, do NOT tally as skipped_full (not a cap failure) */ } } }`.
  - after the loop: `mem.mark_pack_seeded(&key)`; `mem.clamp_to_cap(DEFAULT_WORD_CAP)`; `snapshot = mem.clone()`; drop lock; `self.memory_store.save(&snapshot)?`; return `SeedReport`.
  - **Reviewer:** confirm the Task-4 pack gate makes `Empty`/`TooLong` unreachable, so the silent-drop (no tally) is safe; if a raw/un-gated caller could pass blanks, add a `skipped_invalid` field instead.
- [ ] **Gate:** `nix develop -c cargo test -p ffi` + `nix develop -c cargo clippy -p ffi --all-targets -- -D warnings`.

### Task 3 — the dual-flow invariant, pinned end-to-end (ffi; D8-15)
- [ ] **RED:** a hermetic test (in `crates/ffi/src/session.rs` or `vocabulary.rs` tests): seed one distinctive term (e.g. `"boxwood"`) via `seed_vocabulary`, then (a) assert `collect_bias_terms(&memory, Some("landscape"))` contains `"boxwood"` and `build_bias_prompt(...)` renders it; (b) assert `memory.to_prompt()` contains `## vocabulary` and `- boxwood`; (c) assert `to_prompt()` does **NOT** contain `_seeds` or `landscape:1`. One test, three asserts — the whole D8 claim.
- [ ] **Gate:** `nix develop -c cargo test -p ffi` + `nix develop -c cargo test --workspace` (whole-workspace green; no `whisper` feature).

### Task 4 — Swift seam: `seedVocabulary` + packs + card (apps/ios; `// sac:` visuals deferred)
- [ ] **Protocol** (`WalkEngine.swift`): add `func seedVocabulary(trade: String, version: UInt32, terms: [String]) throws -> SeedReport` (mirror the uniffi record as a Swift-visible `SeedReport`). `// sac:` note that the **card UI reads `SeedReport.terms`** to refresh.
- [ ] **`MurmurEngine`** (real, `#if canImport`): forward to `engine.seedVocabulary(trade:version:terms:)`.
- [ ] **`DemoWalkEngine`** parity: an in-memory implementation mirroring the Rust semantics loosely (normalize + ci-dedup + `SEED_MAX=60` per-pass bound + a `seededPacks: Set<String>` marker keyed `"trade:version"` for idempotency + the 100-cap) so the demo build exercises the card with no backend. Keep the existing demo vocab (`["french drain","ledger board"]`) unchanged.
- [ ] **Packs** (`// sac:` — sac curates content): `apps/ios/Sources/Resources/VocabPacks/{landscape,property,inspection}.json`, each `{ "trade": "landscape", "version": 1, "terms": ["bark mulch", …] }`. Provide a `VocabPack: Codable` struct + a loader that decodes the bundled pack for `BusinessProfile.tradeKey`.
- [ ] **Schema gate** (`VocabPackTests` in the iOS test target — the ruling's "CI schema test"): for **every** bundled pack assert: decodes; `version >= 1`; `terms` non-empty and `count <= 60` (SEED_MAX); no case-insensitive duplicates; each term `1…6` whitespace words (mirrors `MAX_VOCABULARY_TERM_WORDS`), non-blank. **This gate is what makes `Empty`/`TooLong` unreachable in Task 2.**
- [ ] **Card wiring** (`// sac:` owns visuals/placement — D9): a minimal reference `VocabSeedCard` shown **after the demo walk** (not in the linear MIC step) presenting the pack terms as default-on deselectable chips + the existing free-form add bar; **Done** calls `seedVocabulary(trade, version, confirmedChips)` then `addVocabularyTerm(each free-form)`; **Skip** writes nothing. **Remove the stale `TODO(#181)`** in `OnboardingFlow.swift`'s MIC step. Persist an app-side `onboardingVocabSeeded` flag (UserDefaults, alongside `BusinessProfile`) so the card is not re-shown — a UX mirror of the core marker (core stays authoritative for idempotency).
- [ ] **Gate:** iOS **demo** build (xcodebuild OUTSIDE nix): `xcodebuild -project SitewalkGallery.xcodeproj -scheme SitewalkGallery -destination 'platform=iOS Simulator,name=iPhone 17' build`. Run `VocabPackTests` in the app test target.

### Task 5 — real-core compile + bindings drift (dam-manual) + merge  **[MANDATORY GATE]**
- [ ] dam runs `cd apps/ios && ./build-ffi.sh && ./generate.sh && xcodebuild -project SitewalkGallery.xcodeproj -scheme SitewalkGallery -destination 'platform=iOS Simulator,name=iPhone 17' build` — confirm the real-core archive compiles against the new `seed_vocabulary` + `SeedReport` bindings (CI cannot do this — Plan 13 lesson).
- [ ] Bindings-drift check: regenerate the Swift bindings from `crates/ffi`; confirm `MurmurEngine.swift`'s `seedVocabulary` + `SeedReport` symbols resolve and no unrelated record drifted.
- [ ] **Merge the PR** — TestFlight internal lane publishes the seeding path on real-engine. (sac's card-visuals PR may follow independently.)

---

## Gates (every task)
- `nix develop -c cargo test --workspace` — exit code 0 (never grep counts; run under the Nix shell).
- `nix develop -c cargo clippy --workspace --all-targets -- -D warnings`.
- **All pre-existing harness/memory + ffi/vocabulary tests stay green** (Task 1's `_`-exclusion must not change any non-`_` section behavior; Task 2 must not change the CRUD funnel).
- iOS **demo** build (CI-gated) + `VocabPackTests` — **outside** the Nix shell.
- **MANDATORY:** real-core compile + bindings-drift (dam-manual, Task 5) — before merge.

## Acceptance criteria
1. `seed_vocabulary(trade, version, terms)` writes each term through the **existing** `add_vocabulary_term(_, _, Stated)` funnel — no new normalize/dedup/cap path — and returns exact `SeedReport` counts matching WE-A…WE-E.
2. Seeding is **idempotent** per `trade:version` (second call = no-op, `already_seeded:true`, no save) and **deletion is durable** (a removed seeded term is not resurrected by re-seed) — WE-B.
3. `SEED_MAX = 60` bounds terms added per pass (WE-D); the **100-cap is unchanged and unforked** and backstops without eviction (WE-E, R7); the constant lives in the ffi seeding funnel, not harness.
4. Trade change **unions** (WE-C): new pack added, prior seeds + user terms preserved; never a replace/delete.
5. Seeded terms reach **both** the `begin_walk` STT bias prompt **and** `Memory::to_prompt`; the `_seeds` marker reaches **neither** (D8, Task 3).
6. Internal `_`-prefixed sections are excluded from render/word_count/clamp/prune; **no existing (non-`_`) section behavior changes** (Task 1 regression pins green).
7. Every bundled JSON pack passes `VocabPackTests` (version ≥ 1, ≤ 60 terms, no ci-dupes, 1–6 words each); the card seeds confirmed chips + free-form terms after the demo walk; **Skip writes nothing**; `NotesPayload`/finish untouched; real-core archive builds (Task 5).

## Non-goals (explicit)
- A **new write path** — seeding only calls `add_vocabulary_term`; no bypass of normalize/dedup/cap/word-guard.
- A `MemoryEntry` **schema change**, a new `FactSource` variant, or a per-term "seeded" flag; a **`Pinned`/protected tier** for seeds (Plan 12 D3 escalation — deferred, Open Question 3).
- **Auto-harvest** of proper nouns from live extraction (Plan 12 D9 seam — `Inferred` source ready; detection not built).
- **Contact-sync / previous-notes import** (a term source, but separate plumbing).
- **Voice-capture interview** (type-only v1, design Q3); **redesigning `VocabularyView`** (reused as-is).
- **Server anything** — packs are bundled on-device (privacy invariant holds).
- A **new SQLite migration** (the marker rides the in-memory `Memory.sections` map, persisted by the existing `MemoryStore`).
- The vocab-card **visual design / chip layout / exact flow placement** (sac's follow-up; this plan supplies the data path + reference wiring).
- Counting existing seeds toward the budget (SEED_MAX is per-pass, not a running seed total — D3); the offline eval before/after (design Q6 — run once, not wired as a standing gate here).

## Risks & rollback
- **Risk — over-broad `_` exclusion breaks an existing section.** Mitigation: no current section is `_`-prefixed; Task 1's regression pins assert `vocabulary`/`people` render/count/evict unchanged. Rollback: revert Task 1 (the marker is the only `_` user); FFI seeding then loses idempotency but the funnel still works.
- **Risk — `SEED_MAX` forks the cap.** Mitigation: the constant lives in a *different crate* (ffi) from `MAX_VOCABULARY_TERMS` (harness); `seed_vocabulary` never compares against 100 itself — the funnel's `Full` is the only cap check. Rollback: none needed; removing `seed_vocabulary` leaves the funnel untouched.
- **Risk — a pack version bump resurrects a user-deleted term.** Bounded: only user-confirmed chips reach core, and the funnel dedups; worst case the user re-deletes one term. Open Question 2 tracks a chip-suppression option.
- **Rollback of the whole plan:** delete `seed_vocabulary`/`SeedReport` (ffi) + the two `Memory` marker methods + the `_`-exclusion (harness) + the Swift protocol method/packs/card. All additive → clean revert; the pre-existing vocabulary CRUD + STT/LLM read paths are untouched, so main returns to cold-start seeding (today's behavior) with no data migration.

## Open questions
1. **Budget model (dam).** `SEED_MAX` is a **per-pass** bound (D2), so N trade-changes can seed up to N×60 (bounded by the 100-cap). Is per-pass enough, or should the reflection-headroom guarantee be a **running seed total** (needs per-term seed provenance, D3 — heavier)? — **recommend: per-pass; packs are small, the 100-cap backstops, measure erosion before adding provenance.**
2. **Version-bump resurrection (sac + dam).** A pack `version` bump re-runs the funnel (D4); a user-deleted term still curated in the new version can reappear (only if re-confirmed via chips). Acceptable, or should the card suppress chips the user previously deleted? — **recommend: acceptable for v1 (chips are opt-out and dedup-safe); revisit if users complain.**
3. **Protected `Pinned` tier (dam).** If device testing shows seeds eroding under cap/age pressure (they're `Stated`, so prunable), promote a protected tier (Plan 12 D3). — **ship `Stated` + measure first.**
4. **Marker persistence longevity (dam).** The `_seeds` marker lives in `Memory` (persisted by `MemoryStore`). If a user wipes memory (future "reset" affordance), seeding re-offers — correct behavior? — **recommend: yes, a memory wipe is a legitimate re-onboard signal.**
