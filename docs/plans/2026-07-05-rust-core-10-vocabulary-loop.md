# Murmur Rust Core — Plan 10: The Vocabulary → STT Biasing Loop (write half)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. The Rust tasks (1–4, 6) are **hermetic**: `Memory` unit tests, `MockProvider`, in-memory / temp-file stores — no model, no `whisper` feature, no cmake, no Metal, no network. `cargo test --workspace` must NEVER require the `whisper` feature or a model file (Plan 06 requirement 4 — the load-bearing CI invariant). The Swift task (5) is **not CI-gated**: it builds only in real-core mode (needs the gitignored `MurmurCoreFFI` xcframework), so its gate is a manual `xcodebuild` run by dam, and its **visual design is explicitly sac's** (see D8). Never read `.env` or `project.local.yml`.

**Goal:** Close the *write half* of Murmur's product differentiator — the vocabulary → STT biasing loop. The read half is DONE: `begin_walk` reads `Memory::section_texts("vocabulary")` (≤100 terms) → `build_bias_prompt` → whisper `initial_prompt` (+10–19 pp term recall, spike `RESULTS.md`). What is missing is everything that puts terms *into* that section deliberately:

1. **Core** — a vocabulary management surface on `harness::Memory`: a canonical section constant, a write-time ≤100-term cap, normalization + case-insensitive dedup, and a provenance floor so user vocabulary is not evicted casually (D2/D3/D4).
2. **FFI** — three throwing, panic-free `MurmurEngine` methods (`list_vocabulary` / `add_vocabulary_term` / `remove_vocabulary_term`) with the established lock-then-save discipline (D6).
3. **Swift** — a functional-but-plain vocabulary editor (view / add / remove) wired through the `WalkEngine` seam, with a prominent `// sac:` visual-design handoff (D8).
4. **Verification** — a hermetic end-to-end test proving *added term → memory section → `begin_walk` bias assembly picks it up*, plus a device-gated spike note for measuring real recall lift (D7).

**What this plan is NOT.** It does not change the biasing *mechanism* (`build_bias_prompt`, the whisper `initial_prompt` seam, `collect_bias_terms`), the reflection cadence/policy, the `Memory` eviction algorithm (`clamp_to_cap`/`prune_stale` semantics), the sync/tombstone/snapshot machinery, `EngineConfig`, `WalkSession`, `WalkEvent`, or any STT decoding. It does not build the **onboarding interview** (joint dam+sac design — D9, named as such), **auto-harvest** of proper nouns from live extraction (seam designed, not built — D5), a reflection re-injection / vocabulary-immunity tier (open question — D3), per-template seed vocabularies (`collect_bias_terms`'s `_template` stays reserved), or trie/logit biasing. It adds a management surface to an existing section and a CRUD path across FFI.

**Hard dependencies (all DONE, `main` @ `b7d79c8`):**
- **Plan 02** (`crates/harness`): `Memory` with `sections: BTreeMap<String, Vec<MemoryEntry>>`, `remember`/`remember_from`/`forget`/`section_texts`/`word_count`/`clamp_to_cap`/`prune_stale`; `FactSource` (Inferred < Stated < Corrected) with the **upgrade-never-downgrade** rule and **corrected-never-pruned / evicted-last** eviction; `FileMemoryStore` (atomic write + 3 rotating snapshots); `UpdateMemoryTool` (the lock-then-save + clamp precedent); `ReflectionEngine` (full-replace, verbatim-survivor rule).
- **Plan 05/06/08** (`crates/stt`): `build_bias_prompt(terms, max_terms)`, `SttConfig::max_bias_terms = 100`.
- **Plan 07** (`crates/ffi`): `MurmurEngine` holding `memory: Arc<Mutex<Memory>>` + `memory_store: Arc<dyn MemoryStore>`; `EngineError` (`#[uniffi(flat_error)]`, no panics across FFI); `begin_walk` → `collect_bias_terms(&memory, template)` → `build_stt_stream(&bias)`; `with_providers` test constructor.
- **iOS shell** (`apps/ios`): `WalkEngine` protocol + `DemoWalkEngine` / `MurmurEngine` conformers; `@Observable AppModel`; `GalleryApp` navigation; `Theme` design system.

**Verified API facts (checked against source, not guessed):**
- `harness::Memory` methods used here: `section_texts(&self, section) -> Vec<&str>` (`memory/mod.rs:112`), `remember_from(section, text, now, FactSource, Option<String>)` (`:61`, upgrade-never-downgrade), `forget(section, text) -> bool` (`:89`, drops empty sections), `clamp_to_cap(cap) -> usize` (`:175`, evicts by `(source.rank(), last_touched, section)`), `word_count()` (`:103`). `DEFAULT_WORD_CAP = 500` (`:9`). Exported from `harness/src/lib.rs:20`.
- `FactSource::rank()`: `Inferred=0, Stated=1, Corrected=2` (`memory/mod.rs:24`). `clamp_to_cap` evicts ascending rank → **all `Inferred` facts are evicted before any `Stated`** (the basis of D3). `prune_stale` keeps `e.source == Corrected || e.last_touched >= cutoff` (`:163`) — **`Stated` is NOT immune to staleness pruning** (D3 open question).
- `UpdateMemoryTool::execute` precedent (`memory/tool.rs:92–108`): mutate under `memory.lock()`, `clamp_to_cap(word_cap)`, `mem.clone()` snapshot, **release lock**, then `store.save(&snapshot)?`. Save failure surfaces as `Err` but the in-memory mutation is kept. This plan's FFI methods mirror it exactly.
- `crates/ffi/src/session.rs:96` `collect_bias_terms(memory, _template)` reads `memory.section_texts("vocabulary")`, `.take(SttConfig::default().max_bias_terms)` (=100). The `bias_terms_from_memory_vocabulary` test (`:1266`) already pins this seam.
- `MurmurEngine` fields: `memory: Arc<Mutex<Memory>>` (`engine.rs:114`), `memory_store: Arc<dyn MemoryStore>` (`:115`). Production `memory_store` = `FileMemoryStore::new(format!("{}.memory.json", db_path))` (`:150`). `EngineError` variants: `Store`/`Runtime`/`BeginWalk` (`:17–28`), all `String`-carrying, `flat_error`.
- The **`"vocabulary"` string is duplicated** across `ffi/src/session.rs:99`, `stt/src/bias.rs` (doc), `harness/src/memory/tool.rs:73` (schema hint), and murmur-core tests — no single constant. The **≤100 cap lives only on the read side** (`SttConfig::max_bias_terms`); nothing caps writes today, so a section with >100 terms silently drops its tail at `begin_walk` (D2/D4).
- `murmur-core/src/coordinator.rs:133` reflection persists via `*memory = outcome.memory.clone()` under the lock — a **whole-Memory swap** (D3 race note).
- Swift: **no Settings/`Form`/`List`/`onDelete` exists anywhere** in `apps/ios/Sources`; every list is a hand-built `VStack { ForEach { row } }`. `WalkEngine` (`Engine/WalkEngine.swift`) is the seam; `MurmurEngine.swift` wraps `MurmurCoreFFI.MurmurEngine` under `#if canImport(MurmurCoreFFI)`; the generated FFI surface exposes **no** vocabulary methods (D6/D8 require regenerating bindings via `build-ffi.sh` + `generate.sh`).

**Spec:** vision spec §vocabulary point 3 ("Onboarding is an interview … answers seed the memory vocabulary, which feeds (a) LLM context and (b) the STT contextual-biasing list … reflection keeps enriching it"), item 8 ("Memory transparency — a screen … user can read, edit, delete"), §7 (500-word cap, reflection compresses), Rev 2 amendment F ("Vocabulary … ≤100 curated, phonetically-confusable domain terms — iOS `contextualStrings` limit; the most aggressively curated part of memory"), Plan 02 provenance rules (upgrades never downgrade, corrected never pruned — **locked**).

---

## Architecture — decisions, justified (reviewers read these first)

### D1. Vocabulary is a hardened *edge* on the existing memory section — not a new store
Terms already live in `Memory`'s `"vocabulary"` section and are already read by `collect_bias_terms`. The gap is a *management surface*, not a data structure. Adding a parallel vocabulary store would fork the sync/tombstone/snapshot/word-cap machinery that already covers `Memory`, and would desync from the section reflection reads and rewrites. So this plan adds methods **on `Memory`** (write-time cap, normalization, dedup, provenance floor) and a thin FFI wrapper — nothing else. (Rejected: a dedicated `VocabularyStore` table in `murmur-core` — needless duplication; vocabulary is a handful of short strings, not a domain entity with tombstones.)

### D2. A `VOCABULARY_SECTION` constant + `MAX_VOCABULARY_TERMS` retire the duplicated string and the write-side blind spot
The literal `"vocabulary"` is duplicated across four crates and the ≤100 cap exists only on the read side (`SttConfig::max_bias_terms`), so today a section can hold 150 terms and `begin_walk` silently keeps the first 100 (insertion order) — the user's most recent 50 additions never bias anything, with no signal. Introduce, in `harness`:
- `pub const VOCABULARY_SECTION: &str = "vocabulary";`
- `pub const MAX_VOCABULARY_TERMS: usize = 100;` — doc: **must equal `stt::SttConfig::max_bias_terms`**; the harness cannot depend on `stt`, so this is a mirrored constant with a doc cross-reference (a Task 6 test asserts they match numerically via the FFI crate, which sees both).

`collect_bias_terms` and the murmur-core/ffi call sites migrate to `harness::VOCABULARY_SECTION` (mechanical; no behavior change). The write-time cap (D4) makes the limit honest at the point of entry.

### D3. User-managed vocabulary defaults to `FactSource::Stated`; provenance *is* the eviction protection (no new budget)
The team-lead constraint is "vocabulary terms should NOT be evicted casually." Provenance already delivers this: `clamp_to_cap` evicts ascending `(rank, last_touched)`, so **every `Inferred` fact in all of memory is evicted before any `Stated` vocabulary term**. Therefore user-typed/onboarding vocabulary is written with `source = Stated` (the user asserted it). We do **not** add a separate vocabulary word-budget carved out of the 500-word cap — that complicates the single-cap invariant for a set that is ≤100 short terms (≈≤200 words, comfortably inside 500 alongside other facts). The `Stated` floor is sufficient for v1.

**Honest limits, surfaced (not hidden):**
- `Stated` is **not immune** to `clamp_to_cap` (only relative priority) nor to `prune_stale` (only `Corrected` is immune, `mod.rs:163`). Under extreme memory pressure, or if the app ever wires `prune_stale` over the vocabulary section, a `Stated` term can still be evicted after all `Inferred` facts are gone.
- Reflection swaps the whole `Memory` (`coordinator.rs:133`); a `Stated` vocabulary term the reflection model omits from its `write_memory` output is **lost** (unlike `Corrected`, which the reflection prompt protects).
- **OPEN QUESTION for dam:** if device testing shows reflection or cap-pressure eroding user vocabulary, escalate user vocabulary to a *protected tier* — either mark it `Corrected` (reuses the existing immunity, but overloads provenance semantics) or add a new `FactSource::Pinned` rank / a vocabulary-aware `prune_stale` skip. This is a real product-risk vs. semantic-purity tradeoff; v1 ships `Stated` + the D5 prompt line and measures before escalating. **Not built here.**

### D4. Write-time normalization + case-insensitive dedup + reject-when-full
`add_vocabulary_term` normalizes and guards at the point of entry:
- **Normalize:** trim ends, collapse internal whitespace runs to a single space. Case is preserved as typed (LLM context wants human casing; the whisper `initial_prompt` is effectively case-insensitive, so casing is a display/context concern only).
- **Reject empty** (after normalization) → a distinct outcome the FFI maps to an error.
- **Case-insensitive dedup:** a term whose normalized-lowercased form already exists is a no-op (keeps the first-seen casing). Prevents "French Drain" and "french drain" from each eating a cap slot. (Judgment call flagged: exact-match dedup would be simpler but wastes scarce slots on casing variants; the ≤100 budget makes dedup worth the scan.)
- **Cap = reject, not silent-evict:** at `MAX_VOCABULARY_TERMS`, a new term returns `Full` (FFI → thrown error → the editor shows "vocabulary full (100); remove a term first"). Rejecting is honest and puts the curation decision on the user, versus silently evicting their oldest term. (Rejected: evict-oldest-on-add — hides the limit and can drop a term the user still wants.)

The outcome is a total enum, no `Result` gymnastics inside `Memory`:
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VocabAdd { Added, Duplicate, Full, Empty }
```
`Duplicate` is success-shaped at the FFI (idempotent), `Full`/`Empty` are errors.

### D5. Reflection curation in v1: preserve, don't decay — one prompt sentence, no new machinery
Reflection already treats vocabulary as an ordinary section (it appears in the rewrite). v1 keeps that but adds **one sentence** to the reflection system prompt: *preserve vocabulary terms verbatim; only drop a vocabulary term that is clearly a transcription artifact, not a real domain term.* This is additive prompt text — existing reflection tests assert on substrings/behavior that this does not disturb (a Task 2 test pins the new guidance is present). We do **not** build vocabulary decay, dedupe-in-reflection, or re-injection of dropped user terms in v1 (that is the D3 escalation, deferred). Rationale: cheap, on-spec ("reflection keeps enriching it"), and conservative — it biases reflection toward keeping the differentiator's data without new code paths.

### D6. FFI: three throwing methods on `MurmurEngine`, lock-then-save discipline, new `EngineError::Memory`
A new `crates/ffi/src/vocabulary.rs` adds a `#[uniffi::export] impl MurmurEngine` block (uniffi allows impl blocks across files — `begin_walk` already lives in `session.rs`, not `engine.rs`):
```rust
pub fn list_vocabulary(&self) -> Result<Vec<String>, EngineError>
pub fn add_vocabulary_term(&self, term: String) -> Result<Vec<String>, EngineError>
pub fn remove_vocabulary_term(&self, term: String) -> Result<Vec<String>, EngineError>
```
Each mirrors `UpdateMemoryTool::execute`: lock `self.memory`, mutate (`add` uses `FactSource::Stated`, D3), `clamp_to_cap(DEFAULT_WORD_CAP)` after a mutation (keeps the global invariant, same as the tool), clone a snapshot, **release the lock**, then `self.memory_store.save(&snapshot)?`. Errors:
- lock poisoned → `EngineError::Memory("memory lock poisoned")` (never panic across FFI).
- `add` outcome `Full` → `EngineError::Memory("vocabulary is full (100 terms); remove one first")`; `Empty` → `EngineError::Memory("term is empty")`; `Added`/`Duplicate` → `Ok`.
- save failure → propagated as `EngineError::Store` (via a `From`/`map_err`), consistent with the rest of the crate.

`add`/`remove` return the **resulting list** so the Swift editor updates in one round-trip (no follow-up `list` call). `&self` methods (not `self: Arc<Self>`) — they hand out no session. Add `#[error("memory error: {0}")] Memory(String)` to `EngineError`.

**Concurrency note (surfaced, not fixed here):** `self.memory` is shared with the live extractor and the reflection swap. The editor is a Settings surface used **outside** a walk, and reflection runs at session end with guaranteed compute (coordinator contract), so an edit does not realistically race a reflection swap. If they did interleave, last-writer-wins on the whole `Memory` under the lock — a term added during an in-flight reflection could be lost. Acceptable for v1 (documented); the D3 escalation would also address it.

### D7. Biasing verification is hermetic at the *assembly seam*; real recall-lift is device/model-gated
The whole loop cannot be observed hermetically inside a real `SttStream` (bias only reaches whisper's `initial_prompt` under the `whisper` feature; `ScriptedDecoder` ignores it). So the CI gate proves the **assembly seam**: *add via the FFI method → `Memory` vocabulary section → `collect_bias_terms` picks it up → `build_bias_prompt` contains it* (a single hermetic test through the public engine method, extending the existing `bias_terms_from_memory_vocabulary` pin). The empirical **+pp recall lift on real audio** is measured with the existing `spikes/stt-whisper` harness (real model + `say`-generated WAVs), **flagged for dam on device** — the same manual/not-CI pattern as Plan 09 D8. This plan does not make that measurement a CI gate.

### D8. The Swift editor is functional-plain; **visual design is sac's** (prominent handoff)
Per `meta/WORKFLOWS.md` and the division of labor, **sac owns UI layout and visual direction.** This plan delivers a *functional* screen only: a plain SwiftUI list of terms + an add field + delete affordance, wired to the three FFI methods through `WalkEngine`, styled minimally (bare `List`/`Form` is acceptable — the app has none today, so this introduces the first). It carries a prominent `// sac:` handoff comment marking every visual decision (row style, section headers, empty state, add affordance, where the entry point lives in `BoardView`'s header, sheet-vs-push). The **onboarding interview flow is explicitly out of scope** (D9). Entry point recommendation for sac: a `.sheet` off `BoardView` (modal, matches existing sheet usage) rather than a new `AppModel.Phase` — but that is sac's call to finalize.

Swift is not CI-gated (hermetic Rust is the gate). The build check is a manual real-core `xcodebuild` by dam after `build-ffi.sh` + `generate.sh` regenerate the bindings with the new methods.

### D9. Onboarding-seeded and auto-harvest: seam designed, not built
Spec Rev 3: onboarding seeds vocabulary that feeds both LLM context and STT biasing. v1 supports **manual** (the editor) and **onboarding-seeded** terms *through the same `add_vocabulary_term` path* — the onboarding *interview UI/flow* is joint dam+sac design and is **out of scope** (named here so it isn't silently assumed built). **Auto-harvest** (live extraction detecting proper nouns / trade terms and feeding them back) is designed as a seam only: `Memory::add_vocabulary_term` takes an explicit `source` parameter precisely so a future harvester adds with `FactSource::Inferred` (evicted first, never crowds out user/onboarding terms) without touching the public API. Building the harvester needs proper-noun detection in the extraction prompt/pipeline — not free, so deferred. No task.

---

## File Structure

```
crates/
  harness/src/
    memory/
      mod.rs         # MODIFY: VOCABULARY_SECTION, MAX_VOCABULARY_TERMS consts; VocabAdd enum;
                     #         vocabulary_terms / add_vocabulary_term / remove_vocabulary_term on Memory
      lib.rs         # (harness/src/lib.rs) MODIFY: export the consts, VocabAdd, (methods ride Memory)
    reflection/
      engine.rs      # MODIFY: one sentence of vocabulary-preservation guidance in system_prompt (+ test)
  ffi/src/
    vocabulary.rs    # NEW: #[uniffi::export] impl MurmurEngine { list/add/remove_vocabulary* }
    engine.rs        # MODIFY: EngineError::Memory variant
    session.rs       # MODIFY: collect_bias_terms uses harness::VOCABULARY_SECTION (mechanical)
    lib.rs           # MODIFY: pub mod vocabulary;
    (tests)          # NEW hermetic e2e: add-via-FFI → collect_bias_terms → build_bias_prompt
apps/ios/Sources/
  Engine/WalkEngine.swift        # MODIFY: 3 vocabulary methods on the protocol
  Engine/DemoWalkEngine.swift    # MODIFY: in-memory conformance (demo works with no backend)
  Engine/MurmurEngine.swift      # MODIFY: call the new FFI methods (#if canImport(MurmurCoreFFI))
  App/AppModel.swift             # MODIFY: vocabulary state + load/add/remove + error surface
  <Vocabulary editor view>       # NEW: functional-plain editor (sac owns visuals — // sac: handoff)
  Flow/BoardView.swift           # MODIFY: entry point (gear → sheet) — // sac: placement
docs/
  plans/2026-07-05-rust-core-10-vocabulary-loop.md   # THIS FILE
meta/ROADMAP.md                  # MODIFY (Task 6): mark the write-half loop landed
```

Run cargo **inside** the Nix dev shell (`direnv` / `nix develop`). Run `xcodegen`/`build-ffi.sh`/`generate.sh` inside the shell, but `xcodebuild` **outside** it (CLAUDE.md: Nix linker env breaks Xcode `ld`).

---

## Part A — Core: the vocabulary management surface

### Task 1: `Memory` vocabulary API — constants, `VocabAdd`, add/remove/list

**Files:** Modify `crates/harness/src/memory/mod.rs`, `crates/harness/src/lib.rs`.

- [ ] **Step 1 — failing tests** (bottom of `memory/mod.rs` `mod tests`):

```rust
#[test]
fn add_vocabulary_term_normalizes_and_defaults_stated() {
    let mut m = Memory::default();
    assert_eq!(m.add_vocabulary_term("  french   drain ", 10, FactSource::Stated), VocabAdd::Added);
    // normalized: trimmed + internal whitespace collapsed
    assert_eq!(m.vocabulary_terms(), vec!["french drain"]);
    let e = &m.sections[VOCABULARY_SECTION][0];
    assert_eq!(e.source, FactSource::Stated, "user vocabulary is Stated (survives casual eviction)");
    assert_eq!(e.last_touched, 10);
}

#[test]
fn add_vocabulary_term_is_case_insensitively_idempotent() {
    let mut m = Memory::default();
    assert_eq!(m.add_vocabulary_term("French Drain", 1, FactSource::Stated), VocabAdd::Added);
    assert_eq!(m.add_vocabulary_term("french drain", 2, FactSource::Stated), VocabAdd::Duplicate);
    assert_eq!(m.vocabulary_terms(), vec!["French Drain"], "first-seen casing kept, one slot used");
}

#[test]
fn add_vocabulary_term_rejects_empty() {
    let mut m = Memory::default();
    assert_eq!(m.add_vocabulary_term("   ", 1, FactSource::Stated), VocabAdd::Empty);
    assert!(m.vocabulary_terms().is_empty());
}

#[test]
fn add_vocabulary_term_enforces_the_hundred_term_cap() {
    let mut m = Memory::default();
    for i in 0..MAX_VOCABULARY_TERMS {
        assert_eq!(m.add_vocabulary_term(&format!("term{i}"), 1, FactSource::Stated), VocabAdd::Added);
    }
    assert_eq!(m.vocabulary_terms().len(), MAX_VOCABULARY_TERMS);
    assert_eq!(m.add_vocabulary_term("one too many", 1, FactSource::Stated), VocabAdd::Full);
    assert_eq!(m.vocabulary_terms().len(), MAX_VOCABULARY_TERMS, "cap holds; nothing silently evicted");
    // a duplicate is NOT rejected as full — idempotent even at cap
    assert_eq!(m.add_vocabulary_term("term0", 2, FactSource::Stated), VocabAdd::Duplicate);
}

#[test]
fn remove_vocabulary_term_is_case_insensitive_and_reports() {
    let mut m = Memory::default();
    m.add_vocabulary_term("French Drain", 1, FactSource::Stated);
    assert!(m.remove_vocabulary_term("french drain"), "case-insensitive match");
    assert!(m.vocabulary_terms().is_empty());
    assert!(!m.remove_vocabulary_term("french drain"), "already gone");
}

#[test]
fn inferred_vocabulary_is_evicted_before_stated_vocabulary() {
    // D3: an auto-harvested (Inferred) term goes before a user (Stated) term under cap pressure.
    let mut m = Memory::default();
    m.add_vocabulary_term("user term one", 100, FactSource::Stated);   // 3 words
    m.add_vocabulary_term("harvested term", 200, FactSource::Inferred); // 2 words, newer
    m.clamp_to_cap(3); // must drop the Inferred one despite it being newer
    assert_eq!(m.vocabulary_terms(), vec!["user term one"]);
}
```

- [ ] **Step 2 — implement** (in `memory/mod.rs`):

Add near the top (beside `DEFAULT_WORD_CAP`):
```rust
/// The one memory section read by the STT biasing layer (`collect_bias_terms`
/// → `build_bias_prompt` → whisper `initial_prompt`). Canonical name — every
/// crate references this constant rather than the bare string "vocabulary".
pub const VOCABULARY_SECTION: &str = "vocabulary";

/// Write-time cap on vocabulary terms. MUST equal `stt::SttConfig::max_bias_terms`
/// (the read-side cap); `harness` cannot depend on `stt`, so this mirrors it —
/// a Task 6 FFI test asserts they are numerically equal. iOS `contextualStrings`
/// / whisper `initial_prompt` budget (spec Rev 2 amendment F: ≤100 curated terms).
pub const MAX_VOCABULARY_TERMS: usize = 100;

/// Outcome of [`Memory::add_vocabulary_term`]. Total (no `Result` needed):
/// `Added`/`Duplicate` are success (idempotent), `Full`/`Empty` are refusals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VocabAdd {
    Added,
    Duplicate,
    Full,
    Empty,
}
```

Add to `impl Memory`:
```rust
/// The user's vocabulary terms in insertion order (alias for
/// `section_texts(VOCABULARY_SECTION)`).
pub fn vocabulary_terms(&self) -> Vec<&str> {
    self.section_texts(VOCABULARY_SECTION)
}

/// Add one vocabulary term. Normalizes (trim + collapse internal whitespace),
/// rejects empty, dedups case-insensitively (keeps first-seen casing), and
/// enforces `MAX_VOCABULARY_TERMS` at write time (reject-when-full, never
/// silent-evict). `source` is `Stated` for user/onboarding terms; a future
/// auto-harvester (D9) passes `Inferred`. Does NOT enforce the 500-word cap —
/// callers clamp globally (the FFI layer / `UpdateMemoryTool` do).
pub fn add_vocabulary_term(&mut self, term: &str, now: u64, source: FactSource) -> VocabAdd {
    let normalized = term.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return VocabAdd::Empty;
    }
    let exists = self
        .section_texts(VOCABULARY_SECTION)
        .iter()
        .any(|t| t.eq_ignore_ascii_case(&normalized));
    if exists {
        // Touch/upgrade provenance via the existing remember_from path, but do
        // NOT add a second casing variant. Find the stored casing and refresh it.
        if let Some(stored) = self
            .section_texts(VOCABULARY_SECTION)
            .into_iter()
            .find(|t| t.eq_ignore_ascii_case(&normalized))
            .map(str::to_string)
        {
            self.remember_from(VOCABULARY_SECTION, &stored, now, source, None);
        }
        return VocabAdd::Duplicate;
    }
    if self.vocabulary_terms().len() >= MAX_VOCABULARY_TERMS {
        return VocabAdd::Full;
    }
    self.remember_from(VOCABULARY_SECTION, &normalized, now, source, None);
    VocabAdd::Added
}

/// Remove one vocabulary term (case-insensitive, normalized match). Returns
/// whether anything was removed.
pub fn remove_vocabulary_term(&mut self, term: &str) -> bool {
    let normalized = term.split_whitespace().collect::<Vec<_>>().join(" ");
    let Some(stored) = self
        .section_texts(VOCABULARY_SECTION)
        .into_iter()
        .find(|t| t.eq_ignore_ascii_case(&normalized))
        .map(str::to_string)
    else {
        return false;
    };
    self.forget(VOCABULARY_SECTION, &stored)
}
```

`crates/harness/src/lib.rs` — extend the memory re-export:
```rust
pub use memory::{FactSource, Memory, MemoryEntry, VocabAdd, DEFAULT_WORD_CAP, MAX_VOCABULARY_TERMS, VOCABULARY_SECTION};
```

- [ ] **Step 3 — verify:** `cargo test -p harness memory` (green; existing memory/provenance tests unchanged — the new methods only *use* `remember_from`/`forget`/`section_texts`).

- [ ] **Step 4 — commit:** `git add -A && git commit -m "feat(harness): vocabulary management surface on Memory (cap, dedup, Stated floor)"`

---

### Task 2: reflection preserves vocabulary (one prompt sentence)

**Files:** Modify `crates/harness/src/reflection/engine.rs`.

Per D5 — a conservative, additive guard for the differentiator's data. No new machinery.

- [ ] **Step 1 — failing test** (add to `reflection/engine.rs` `mod tests`):

```rust
#[test]
fn system_prompt_protects_vocabulary() {
    let engine = ReflectionEngine::new(std::sync::Arc::new(MockProvider::new(vec![])));
    let p = engine.system_prompt();
    assert!(p.to_lowercase().contains("vocabulary"), "reflection must be told to preserve vocabulary");
}
```
(`system_prompt` is currently private — make it `pub(crate)` or test via the assembled request; a `pub(crate) fn system_prompt` keeps the test simple and is harmless.)

- [ ] **Step 2 — implement:** append one sentence to `system_prompt`'s format string, before the "Call {} exactly once" clause:
  > *Vocabulary terms are domain jargon that improve transcription accuracy — preserve them verbatim and drop a vocabulary term only if it is clearly a transcription artifact, not a real term.*

Confirm the existing reflection tests (`rebuilds_memory_preserving_full_prior_entries`, `churn_*`, `empty_result_*`) still pass — they assert on substrings/outcomes this sentence does not affect.

- [ ] **Step 3 — verify:** `cargo test -p harness reflection` green.

- [ ] **Step 4 — commit:** `git add -A && git commit -m "feat(harness): reflection preserves vocabulary terms (prompt guidance, no new machinery)"`

---

## Part B — FFI: vocabulary CRUD across UniFFI

### Task 3: `EngineError::Memory` + `MurmurEngine` vocabulary methods

**Files:** Create `crates/ffi/src/vocabulary.rs`; modify `crates/ffi/src/engine.rs` (error variant), `crates/ffi/src/lib.rs` (`pub mod vocabulary;`), `crates/ffi/src/session.rs` (`collect_bias_terms` uses the constant).

- [ ] **Step 1 — failing tests** (`crates/ffi/src/vocabulary.rs` `mod tests`): use a save-recording `SpyStore` (copy the pattern from `harness/src/memory/tool.rs` tests) and `MurmurEngine::with_providers`.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{MurmurEngine, Providers};
    use harness::{HarnessError, Memory, MemoryStore, MockProvider};
    use std::sync::{Arc, Mutex as StdMutex};

    struct SpyStore { saved: StdMutex<Vec<Memory>> }
    impl MemoryStore for SpyStore {
        fn load(&self) -> Result<Memory, HarnessError> { Ok(Memory::default()) }
        fn save(&self, m: &Memory) -> Result<(), HarnessError> { self.saved.lock().unwrap().push(m.clone()); Ok(()) }
    }
    fn engine(store: Arc<SpyStore>) -> Arc<MurmurEngine> {
        let s = murmur_core::Store::open_in_memory("device-a").unwrap();
        MurmurEngine::with_providers(s, Memory::default(), store, Providers {
            live: Arc::new(MockProvider::new(vec![])),
            processing: Arc::new(MockProvider::new(vec![])),
            reflection: Arc::new(MockProvider::new(vec![])),
        })
    }

    #[test]
    fn add_list_remove_round_trip_and_persist() {
        let store = Arc::new(SpyStore { saved: StdMutex::new(Vec::new()) });
        let e = engine(store.clone());
        assert_eq!(e.add_vocabulary_term("french drain".into()).unwrap(), vec!["french drain"]);
        assert_eq!(e.list_vocabulary().unwrap(), vec!["french drain"]);
        // persisted: the last save carries the term
        assert!(store.saved.lock().unwrap().last().unwrap().vocabulary_terms().contains(&"french drain"));
        assert!(e.remove_vocabulary_term("French Drain".into()).unwrap().is_empty(), "case-insensitive remove");
    }

    #[test]
    fn add_is_idempotent_and_full_is_an_error() {
        let store = Arc::new(SpyStore { saved: StdMutex::new(Vec::new()) });
        let e = engine(store);
        e.add_vocabulary_term("term".into()).unwrap();
        assert_eq!(e.add_vocabulary_term("TERM".into()).unwrap(), vec!["term"], "duplicate is Ok, not an error");
        // fill to the cap, then the next add throws
        for i in 0..harness::MAX_VOCABULARY_TERMS { let _ = e.add_vocabulary_term(format!("t{i}")); }
        assert!(matches!(e.add_vocabulary_term("overflow".into()), Err(EngineError::Memory(_))));
        assert!(matches!(e.add_vocabulary_term("   ".into()), Err(EngineError::Memory(_))), "empty is an error");
    }

    #[test]
    fn read_side_cap_matches_the_write_side_constant() {
        // D2: the mirrored consts must agree across the crate boundary.
        assert_eq!(harness::MAX_VOCABULARY_TERMS, stt::SttConfig::default().max_bias_terms);
    }
}
```

- [ ] **Step 2 — implement** `EngineError::Memory` in `engine.rs`:
```rust
/// A memory / vocabulary mutation failed (lock poisoned, vocabulary full, or
/// an empty term). Recoverable by the host — surface, don't crash. Never
/// contains an api key (memory/vocab strings only).
#[error("memory error: {0}")]
Memory(String),
```

- [ ] **Step 3 — implement** `crates/ffi/src/vocabulary.rs`:
```rust
//! Vocabulary CRUD across UniFFI (Plan 10). The write half of the vocabulary →
//! STT biasing loop: these mutate the `Memory` "vocabulary" section that
//! `begin_walk`'s `collect_bias_terms` reads. Lock-then-save discipline mirrors
//! `harness::UpdateMemoryTool` (mutate under the lock, clamp the global cap,
//! snapshot, release, persist). Panic-free across FFI (Plan 07 CANON).

use std::sync::Arc;

use harness::{DEFAULT_WORD_CAP, FactSource, VocabAdd};

use crate::engine::{EngineError, MurmurEngine};

impl MurmurEngine {
    fn memory_err(msg: impl Into<String>) -> EngineError { EngineError::Memory(msg.into()) }
}

#[uniffi::export]
impl MurmurEngine {
    /// The user's vocabulary terms, insertion order. Read-only — no lock held
    /// across FFI beyond the clone.
    pub fn list_vocabulary(&self) -> Result<Vec<String>, EngineError> {
        let mem = self.memory.lock().map_err(|_| Self::memory_err("memory lock poisoned"))?;
        Ok(mem.vocabulary_terms().into_iter().map(str::to_string).collect())
    }

    /// Add one user vocabulary term (`FactSource::Stated`, D3). Idempotent
    /// (case-insensitive). Errors: `Full` at 100 terms, `Empty` for blank input,
    /// a poisoned lock, or a persistence failure. Returns the resulting list so
    /// the editor updates in one round-trip.
    pub fn add_vocabulary_term(&self, term: String) -> Result<Vec<String>, EngineError> {
        let snapshot = {
            let mut mem = self.memory.lock().map_err(|_| Self::memory_err("memory lock poisoned"))?;
            let now = now_secs();
            match mem.add_vocabulary_term(&term, now, FactSource::Stated) {
                VocabAdd::Added | VocabAdd::Duplicate => {}
                VocabAdd::Full => return Err(Self::memory_err(format!(
                    "vocabulary is full ({} terms); remove one first", harness::MAX_VOCABULARY_TERMS))),
                VocabAdd::Empty => return Err(Self::memory_err("term is empty")),
            }
            mem.clamp_to_cap(DEFAULT_WORD_CAP); // global 500-word invariant, like UpdateMemoryTool
            mem.clone()
        };
        self.memory_store.save(&snapshot).map_err(|e| EngineError::Store(e.to_string()))?;
        Ok(snapshot.vocabulary_terms().into_iter().map(str::to_string).collect())
    }

    /// Remove one vocabulary term (case-insensitive). Returns the resulting list.
    /// Removing a term that isn't present is not an error (idempotent).
    pub fn remove_vocabulary_term(&self, term: String) -> Result<Vec<String>, EngineError> {
        let snapshot = {
            let mut mem = self.memory.lock().map_err(|_| Self::memory_err("memory lock poisoned"))?;
            mem.remove_vocabulary_term(&term);
            mem.clone()
        };
        self.memory_store.save(&snapshot).map_err(|e| EngineError::Store(e.to_string()))?;
        Ok(snapshot.vocabulary_terms().into_iter().map(str::to_string).collect())
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
```
Add `pub mod vocabulary;` to `crates/ffi/src/lib.rs`. In `session.rs`, change `collect_bias_terms` to read `memory.section_texts(harness::VOCABULARY_SECTION)` (mechanical; the existing `bias_terms_from_memory_vocabulary` test stays green because the constant *is* `"vocabulary"`).

- [ ] **Step 4 — verify:** `cargo test -p ffi vocabulary` and `cargo test -p ffi` (all green; no `whisper` feature). Confirm the generated bindings note: `EngineError` gains a `Memory` case and three methods appear on `MurmurEngine` — nothing else in the surface changes.

- [ ] **Step 5 — commit:** `git add -A && git commit -m "feat(ffi): vocabulary CRUD on MurmurEngine (list/add/remove; lock-then-save, EngineError::Memory)"`

---

### Task 4: hermetic end-to-end biasing-loop test

**Files:** Add a test to `crates/ffi/src/session.rs` `mod tests` (it can call the private `collect_bias_terms`).

Per D7 — prove the seam the loop actually traverses, hermetically.

- [ ] **Step 1 — failing test:**
```rust
#[tokio::test]
async fn vocabulary_added_via_ffi_feeds_begin_walk_bias_assembly() {
    let store = Store::open_in_memory("device-a").unwrap();
    let engine = MurmurEngine::with_providers(
        store, Memory::default(), Arc::new(NullMemoryStore),
        Providers {
            live: Arc::new(MockProvider::new(vec![])),
            processing: Arc::new(MockProvider::new(vec![])),
            reflection: Arc::new(MockProvider::new(vec![])),
        },
    );
    // WRITE half (the new FFI path):
    engine.add_vocabulary_term("french drain".into()).unwrap();
    engine.add_vocabulary_term("ledger board".into()).unwrap();

    // READ half (what begin_walk assembles): the terms flow through
    // collect_bias_terms → build_bias_prompt exactly as begin_walk uses them.
    let bias = {
        let mem = engine.memory.lock().unwrap();
        collect_bias_terms(&mem, Some("landscape"))
    };
    assert_eq!(bias, vec!["french drain".to_string(), "ledger board".to_string()]);
    let prompt = stt::build_bias_prompt(&bias, stt::SttConfig::default().max_bias_terms).unwrap();
    assert!(prompt.contains("french drain") && prompt.contains("ledger board"),
        "the whisper initial_prompt carries the user's added terms: {prompt}");
}
```
(`NullMemoryStore` is already defined in `session.rs` tests. Note: `with_providers` builds a text-only engine, but the assertion is on the *bias assembly*, which is model-independent — D7.)

- [ ] **Step 2 — verify:** `cargo test -p ffi vocabulary_added_via_ffi` green.

- [ ] **Step 3 — spike-harness note (docs, no code):** in the task's PR thinking, record that real recall-lift on device is measured by adding the terms, generating audio with those terms via `say`, and diffing WER with/without the vocabulary through the `spikes/stt-whisper` harness (the Plan 06/08 sweep tool) — **manual, flagged for dam**, not CI (mirrors Plan 09 D8). No hermetic test can observe whisper's `initial_prompt` effect.

- [ ] **Step 4 — commit:** `git add -A && git commit -m "test(ffi): hermetic vocabulary-loop e2e — add-via-FFI → begin_walk bias assembly"`

---

## Part C — Swift: the vocabulary editor (functional-plain; **sac owns visuals**)

### Task 5: `WalkEngine` vocabulary methods + editor screen

> **⚠️ VISUAL DESIGN IS SAC'S (D8 / `meta/WORKFLOWS.md`).** This task delivers a *functional* screen only — a plain list, an add field, a delete affordance, wired end-to-end. Every visual decision (row style, section headers, empty state, add affordance, entry-point placement, sheet-vs-push) gets a `// sac:` comment and is left for sac to design. Do **not** invent visual direction. The **onboarding interview is out of scope** (D9, joint dam+sac).

**Files:** Modify `apps/ios/Sources/Engine/WalkEngine.swift`, `Engine/DemoWalkEngine.swift`, `Engine/MurmurEngine.swift`, `App/AppModel.swift`, `Flow/BoardView.swift`; add a new `VocabularyView.swift`.

- [ ] **Step 1 — regenerate FFI bindings** (needs Task 3 merged/present): from the dev shell, `cd apps/ios && ./build-ffi.sh && ./generate.sh`. Confirm the generated `Packages/MurmurCoreFFI/Sources/MurmurCoreFFI/ffi.swift` now exposes `listVocabulary()`, `addVocabularyTerm(term:)`, `removeVocabularyTerm(term:)` (throwing) on `MurmurEngine` and an `.memory` case on the error enum. (If the demo project is active, this is a no-op for demo — the methods are only wired in real-core mode.)

- [ ] **Step 2 — extend the `WalkEngine` protocol** (`Engine/WalkEngine.swift`):
```swift
@MainActor
protocol WalkEngine: AnyObject {
    // ... existing begin/append/pushAudio/finish/cancel ...
    func listVocabulary() throws -> [String]
    func addVocabularyTerm(_ term: String) throws -> [String]   // returns the new list
    func removeVocabularyTerm(_ term: String) throws -> [String]
}
```

- [ ] **Step 3 — conform both engines:**
  - `DemoWalkEngine`: back it with a `private var vocabulary: [String]` seeded with a couple of demo terms (e.g. `["french drain", "ledger board"]`) so the editor is usable with no backend. Implement add (dedup case-insensitively, cap at 100, plain), remove, list — mirror the Rust semantics loosely; this is demo data.
  - `MurmurEngine` (inside `#if canImport(MurmurCoreFFI)`): forward to the FFI methods, translating the thrown `MurmurCoreFFI` error to the app's error surface. Outside the `#if`, the demo engine is used, so no stub needed there.

- [ ] **Step 4 — `AppModel`** (`App/AppModel.swift`): add `private(set) var vocabulary: [String] = []`, a `vocabularyError: String?` (or reuse the existing error surface), and:
```swift
func loadVocabulary() { vocabulary = (try? engine.listVocabulary()) ?? [] }
func addVocabulary(_ term: String) {
    do { vocabulary = try engine.addVocabularyTerm(term) }
    catch { vocabularyError = "\(error)" }   // // sac: how errors surface (full-at-100, empty) is a design call
}
func removeVocabulary(_ term: String) {
    do { vocabulary = try engine.removeVocabularyTerm(term) } catch { vocabularyError = "\(error)" }
}
```

- [ ] **Step 5 — `VocabularyView.swift`** (functional-plain):
```swift
// sac: This whole screen is a functional placeholder — visual design is yours.
// A bare List is used only because the app has none yet; restyle to the Theme
// (Theme.C / Theme.F / Theme.S) or hand-roll rows like the rest of the app.
struct VocabularyView: View {
    @Bindable var model: AppModel
    @State private var newTerm = ""
    var body: some View {
        List {
            Section {                                   // sac: header/empty-state design
                ForEach(model.vocabulary, id: \.self) { term in Text(term) }
                    .onDelete { idx in idx.map { model.vocabulary[$0] }.forEach(model.removeVocabulary) }
            }
            Section {                                    // sac: add affordance design (this is placeholder)
                HStack {
                    TextField("Add a term", text: $newTerm)  // sac: styling / focus / placeholder copy
                    Button("Add") {
                        let t = newTerm; newTerm = ""
                        if !t.trimmingCharacters(in: .whitespaces).isEmpty { model.addVocabulary(t) }
                    }
                }
            }
        }
        .onAppear { model.loadVocabulary() }
        // sac: title, chrome, navigation style are yours.
    }
}
```

- [ ] **Step 6 — entry point** (`Flow/BoardView.swift`): add a gear button in the existing header `VStack` that presents `VocabularyView` as a `.sheet`.
```swift
// sac: entry point + presentation (sheet vs. a new AppModel.Phase) is your call;
// a gear → .sheet is a functional default, not a design decision.
```

- [ ] **Step 7 — verify (dam, manual, real-core, OUTSIDE the Nix shell):**
  - Demo build (clean-checkout path, no FFI dep): `cd apps/ios && xcodegen generate` then `xcodebuild -project SitewalkGallery.xcodeproj -scheme SitewalkGallery -destination 'platform=iOS Simulator,name=iPhone 17' build` — the editor works against `DemoWalkEngine`.
  - Real-core build: after `build-ffi.sh` + `generate.sh`, build again and confirm add/remove/list drive the FFI methods (a term added in the editor lands in `{db_path}.memory.json`'s vocabulary section and biases the next walk). Not a CI gate.

- [ ] **Step 8 — commit:** `git add -A && git commit -m "feat(ios): functional vocabulary editor wired through WalkEngine (visuals: sac handoff)"`

---

## Part D — Docs & final review

### Task 6: docs + independent whole-artifact review

**Files:** `crates/harness/src/memory/mod.rs` (module doc note if useful), `meta/ROADMAP.md`, and the review itself.

- [ ] **Step 1 — docs:** add a `meta/ROADMAP.md` note that the vocabulary → STT biasing loop's **write half** landed (management surface + FFI CRUD + editor), and that the **onboarding interview** (D9, joint) and **auto-harvest** (D9 seam) and any **protected-vocabulary tier** (D3) remain open. Cross-reference this plan.

- [ ] **Step 2 — full hermetic gate** (from inside the dev shell; paste real output — exit codes, not grep counts, per MEMORY lesson about tee/pipefail):
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - confirm neither compiles the `whisper` feature (feature-off default).

- [ ] **Step 3 — independent whole-artifact review** (CANON: independent final review has caught a real issue 9/9 times — a **separate agent** from the builder). Read the diff `memory/mod.rs → reflection/engine.rs → ffi/vocabulary.rs → session.rs → Swift` as one artifact and re-check:
  - **Provenance floor is load-bearing (D3):** user vocabulary is written `Stated`; confirm `clamp_to_cap` evicts all `Inferred` before any `Stated` (the `inferred_vocabulary_is_evicted_before_stated_vocabulary` test), and that the **honest limits are documented, not papered over** (Stated is not immune to `clamp`/`prune`; reflection can still drop a term). The D3 open question is flagged for dam, not silently decided.
  - **Cap is enforced at write, honestly (D4):** `MAX_VOCABULARY_TERMS` rejects (not silent-evicts); duplicates are idempotent even at cap; the read-side/write-side constants are asserted equal (`read_side_cap_matches_the_write_side_constant`).
  - **Normalization + dedup:** case-insensitive dedup keeps first-seen casing; empty rejected; `remove` matches case-insensitively. No second casing variant can ever occupy a slot.
  - **Lock-then-save discipline (D6):** the FFI methods mutate under the lock, clamp the global 500 cap, snapshot, **release the lock before `save`**, and never panic across FFI (lock-poison → `EngineError::Memory`; save-fail → `EngineError::Store`). No api key can reach an error string.
  - **The loop is closed (D7):** the hermetic e2e proves added-term → `collect_bias_terms` → `build_bias_prompt`; the real recall-lift measurement is correctly scoped as device/model-gated (not oversold as a CI gate).
  - **Constant migration is mechanical:** `collect_bias_terms` reading `harness::VOCABULARY_SECTION` changes no behavior; the pre-existing `bias_terms_from_memory_vocabulary` test stays green.
  - **Reflection guard is additive (D5):** the one prompt sentence doesn't break existing reflection tests and isn't oversold as immunity.
  - **CI hermeticity:** `cargo test --workspace` needs no model / `whisper` feature / network; the Swift task is correctly outside the CI gate with the sac handoff prominent.
  - **Scope honesty:** onboarding interview (D9), auto-harvest (D9), and the protected-vocabulary tier (D3) are named as out-of-scope with seams (the `source` param) rather than half-built.

- [ ] **Step 4 — commit:** `git add -A && git commit -m "docs: Plan 10 vocabulary-loop write-half — ROADMAP note + independent review sign-off"`

---

## Non-goals

- **Onboarding interview flow** (D9) — joint dam+sac design; v1 seeds vocabulary through the same `add_vocabulary_term` path but builds no interview UI.
- **Auto-harvest of proper nouns / trade terms from live extraction** (D9) — the `source` parameter is the seam (a harvester adds `Inferred`); the detection is not built.
- **A protected-vocabulary tier / reflection re-injection / vocabulary decay** (D3) — v1 uses the `Stated` floor + the D5 prompt line; escalation is an open question flagged for dam, measured before built.
- **Changing the biasing mechanism** — `build_bias_prompt`, the whisper `initial_prompt` seam, `collect_bias_terms`'s cap logic, trie/logit biasing: untouched.
- **Per-template seed vocabularies** — `collect_bias_terms`'s `_template` stays reserved.
- **Reflection cadence/policy, `Memory` eviction algorithm, sync/tombstone/snapshot machinery, `EngineConfig`, `WalkSession`/`WalkEvent`** — unchanged.
- **Vocabulary visual design** — sac's, per D8.

## Acceptance criteria

1. `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` green **with the `whisper` feature off** (CI invariant); no whisper/model/network dependency.
2. `Memory` has a vocabulary surface: `VOCABULARY_SECTION`/`MAX_VOCABULARY_TERMS` constants, `VocabAdd`, and `vocabulary_terms`/`add_vocabulary_term`/`remove_vocabulary_term` with normalization, case-insensitive dedup, write-time ≤100 cap (reject-when-full), and a `Stated` provenance floor. Existing memory/provenance tests unchanged.
3. `clamp_to_cap` evicts `Inferred` vocabulary before `Stated` (dedicated test); the write-side and read-side caps are asserted numerically equal.
4. `MurmurEngine` exposes `list_vocabulary`/`add_vocabulary_term`/`remove_vocabulary_term` across UniFFI — throwing, panic-free, lock-then-save, `EngineError::Memory` for full/empty/poison, `EngineError::Store` for persistence failure; add/remove return the resulting list; persistence is asserted (spy store).
5. A hermetic e2e test proves add-via-FFI → `Memory` vocabulary section → `collect_bias_terms` → `build_bias_prompt` carries the term; real recall-lift is documented as device/model-gated (spike harness, dam).
6. Reflection carries vocabulary-preservation guidance (test-pinned) with no new machinery and no regression to existing reflection tests.
7. The Swift editor lists/adds/removes terms through `WalkEngine` (demo + real-core conformers); it is functional-plain with a prominent `// sac:` visual-design handoff and no onboarding-interview scope; it builds in demo and real-core modes (manual, dam — not CI).
8. Independent whole-artifact review (separate agent) signs off on the Task 6 Step 3 checklist.

## Open questions (need a call)

- **[dam] Protected-vocabulary tier (D3):** ship v1 with the `Stated` floor + reflection prompt line, then measure on device whether reflection/cap-pressure erode user vocabulary. If it does, escalate to a protected tier (`Corrected` overload vs. a new `Pinned` rank vs. vocabulary-aware `prune_stale`). Deferred, not built.
- **[dam+sac] Onboarding interview (D9):** the flow that *seeds* vocabulary (and BYOK, trade, crew/client names) is joint design — named out of scope here so it isn't assumed built. The `add_vocabulary_term` path is ready to receive its output.
- **[sac] Editor visuals & entry point (D8):** row/section/empty-state style, add affordance, and sheet-vs-`Phase` placement are sac's. This plan ships a functional placeholder behind `// sac:` markers.
