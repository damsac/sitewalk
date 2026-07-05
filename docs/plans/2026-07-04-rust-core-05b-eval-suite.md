# Murmur Rust Core — Plan 05b: Extraction Eval Suite

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a hermetic-by-default **evaluation suite** for extraction quality: a synthetic site-walk corpus with typed ground truth, a deterministic grader (per-kind precision/recall/F1, kind-confusion, contact accuracy, R6 distractor false-positive rate, summary presence, cost), and a runner that executes either against a `MockProvider` (fast, no key, tests the harness itself) or against the real Anthropic API (gated on `ANTHROPIC_API_KEY`, opt-in, key never printed — the `anthropic_smoke` pattern). The suite emits a **machine-comparable JSON report + summary table** so a later DSPy/GEPA-style prompt-optimization loop can diff prompt variants on stable scores. This is the *foundation* for that loop — not the loop itself.

**Product framing (why this shape):** Murmur is one-button voice capture for blue-collar field work (GC site walks, inspections). The pipeline (Plan 04/05) is `transcript → extraction agent (add_item / upsert_contact / write_report) + forced summary`, with a live in-session pass (`LiveExtractor`) that lifts items onto a board during recording. Two product rules drive scoring:

- **R6 — bias toward UNDER-extraction.** Fewer, higher-confidence items. One invented todo costs more trust than three missed ones. → The grader's primary comparable scalar is **F0.5** (precision weighted 2× recall), and a dedicated **distractor false-positive rate** measures how often the model turns chatter/hedging into items it shouldn't.
- **R9 — every LLM call's cost is logged** (`llm_usage` table, purposes `live_extraction` / `processing` / `reflection`). → The runner surfaces **tokens + estimated $ per session** straight from the usage log; no separate accounting.

**Architecture:** One new **dev crate**, `crates/evals`, added as a workspace member. It depends on `murmur-core` and `harness` and reaches shipping code **only through public API** (`Store`, `SessionProcessor`, `LiveExtractor`, `LlmUsageRow`, `CapturedItem`, `Contact`, `MockProvider`, `AnthropicProvider`). Zero changes to `murmur-core` or `harness`. No new migration, no new columns, no touch to any shipping code path — the eval crate is pure consumer. Design decisions, justified:

1. **A separate `crates/evals` dev crate, not `murmur-core/tests/`.** Fixtures, grader, report types, and the runner binary are eval infrastructure, not core behavior. A dedicated crate keeps them out of `murmur-core`'s compile and test graph (core `cargo test` stays fast and focused), gives the runner a natural home as an `example`/`bin`, and lets the corpus grow to dozens of fixtures without bloating the shipping crate. `--workspace` still compiles and runs its hermetic tests, so nothing hides from CI. (Rejected: `murmur-core/tests/` — would drag fixture I/O and a price table into the shipping crate's dev-deps and slow its test loop; the eval suite has a genuinely different lifecycle.)
2. **Deterministic grader by normalized-token Dice, not substring.** Each item's `text` is normalized (lowercase, strip punctuation, collapse whitespace, drop a small stopword set, strip trailing plural `s`) into a token set; a candidate matches an expected item when **Dice coefficient ≥ 0.5** (`2·|A∩B| / (|A|+|B|)`) *and* the kind matches. Dice is symmetric and order-independent — "order the lumber" ≈ "lumber order" — where substring is asymmetric and brittle to STT reordering/misrecognition. It is fully hermetic and deterministic: same inputs → same score, every run, no network. This is the property prompt-optimization needs.
3. **Greedy bipartite matching, one-to-one.** Within a kind, score every (expected, candidate) pair, sort descending, assign greedily above threshold; each expected and each candidate is consumed once. Unmatched expected → false negative; unmatched candidate → false positive. Cross-kind text matches (right text, wrong kind) feed the **kind-confusion** matrix without counting as true positives. Greedy (not optimal Hungarian) because item counts are tiny (<30/session) and greedy-on-sorted is deterministic, dependency-free, and within rounding of optimal at this scale.
4. **F0.5 is the headline scalar; the vector is preserved.** The suite reports precision, recall, F1, F0.5, per-kind breakdown, confusion, contact accuracy, distractor-FP rate, summary presence, and cost — all separately. The single comparable number is **mean F0.5 across the corpus** (β=0.5 encodes R6's precision bias), so an optimizer can rank prompt variants by one scalar while still seeing every axis it might regress.
5. **Paired fixture files: `<id>.txt` (transcript) + `<id>.json` (ground truth).** Transcripts read naturally as plain text (disfluency, trade jargon, filler) so they're reviewable in the plan and easy to author; ground truth is typed JSON (`serde_json`, already a workspace dep). The loader globs the fixtures dir with `std::fs::read_dir` (no glob crate). Ground truth carries `items`, `contacts`, and **`distractors`** — spans that R6 says must *not* become items (hedging, chatter, incomplete thoughts) — plus `expects_summary` and `tags`.
6. **Two runner modes, one grader.** Hermetic mode drives the pipeline with a per-scenario `MockProvider` script and runs under `cargo test` (tests the grader and the harness plumbing, no key, fast). Real-API mode is an `example` binary gated exactly like `anthropic_smoke`: reads `ANTHROPIC_API_KEY` from env, refuses to run without it, never prints it, runs against a configurable model (default `claude-haiku-4-5`). A single `#[ignore]`d real-API test asserts the report is *well-formed* (never asserts score thresholds — real scores are non-deterministic).
7. **Cost from the usage log, not re-counted.** After a run the grader reads `Store::list_llm_usage_for_session` / `usage_totals` and multiplies by a documented price table constant to estimate $. R9 already put the tokens there; the eval just reads and prices them.
8. **`grade.rs` + `normalize.rs` are the "locked metric" (Karpathy `prepare.py` pattern).** The old Swift `ScenarioRunner` split into a locked metric file (never edited during prompt optimization) and the prompt code being optimized — but the two halves drifted (the grader referenced a type that no longer existed; scenarios lacked the fixtures its checks needed). Two lessons ported: (i) the prompt-optimizer touches only `murmur-core` prompts, **never** `grade.rs`/`normalize.rs`/the fixtures — comparability depends on the metric holding still; (ii) `Scenario` unifies transcript + typed ground truth in one loaded structure and the loader **errors on any orphan** (`.txt` without `.json` or vice versa), so the two halves cannot silently drift.

**Tech Stack:** existing workspace deps only — `serde`, `serde_json`, `tokio` (dev), `murmur-core`, `harness`. No new dependencies. All non-`#[ignore]` tests hermetic (`MockProvider` only, no network). `cargo test --workspace` stays green and fast without a key.

**Spec:** vision spec Rev 2 §6 (transformation quality / <8s budget context), R6 (under-extraction bias — scored via F0.5 + distractor-FP), R7 (inspectable outcomes — the grader reads real store rows), R9 (cost per call measured and logged — surfaced per session). Plan 04 (processing pipeline, `SessionProcessor`), Plan 05 (`LiveExtractor`, live board → process() swap) and its final-review carried scenarios (4a–d below). dam's stated purpose: the eval is the foundation for a DSPy/GEPA prompt-optimization loop — scores must be comparable across prompt variants.

---

## File Structure

```
crates/
  evals/
    Cargo.toml               # NEW: dev crate; deps murmur-core, harness, serde, serde_json; dev-dep tokio
    src/
      lib.rs                 # NEW: re-exports (corpus, grade, report, run)
      corpus.rs              # NEW: Scenario, GroundTruth, ExpectedItem/Contact, load_corpus()
      normalize.rs           # NEW: normalize_tokens, dice — pure, deterministic
      grade.rs               # NEW: grader — matching, per-kind P/R/F1, confusion, contacts, R6, F0.5
      report.rs              # NEW: ScenarioReport, SuiteReport, CostReport, price table, render_table()
      run.rs                 # NEW: run_hermetic (MockProvider), run_real (AnthropicProvider), Observed
    fixtures/
      punch_list_short.txt          # NEW seed 1
      punch_list_short.json
      deck_walk_contacts.txt        # NEW seed 2
      deck_walk_contacts.json
      rambling_long_walk.txt        # NEW seed 3
      rambling_long_walk.json
      empty_session.txt             # NEW (carried scenario d)
      empty_session.json
    tests/
      grader_hermetic.rs     # NEW: grader unit-ish tests + hermetic pipeline run over seed corpus
      carried_scenarios.rs   # NEW: 4a failed-process swap gap, 4b restart-dup, 4c multi-pass window
    examples/
      eval.rs                # NEW: gated real-API runner CLI → JSON report + table
README.md                    # MODIFY: plan-series line, how to run the eval
Cargo.toml (workspace)       # MODIFY: add "crates/evals" to members
```

Run cargo via the dev shell or `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo <cmd>` from the repo root.

---

### Task 1: `evals` crate skeleton + corpus schema + loader

**Files:**
- Create: `crates/evals/Cargo.toml`, `crates/evals/src/lib.rs`, `crates/evals/src/corpus.rs`
- Modify: workspace `Cargo.toml`

- [ ] **Step 1: Add the crate to the workspace**

Workspace `Cargo.toml` — extend members:
```toml
members = ["crates/harness", "crates/murmur-core", "crates/evals"]
```

`crates/evals/Cargo.toml`:
```toml
[package]
name = "evals"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
murmur-core = { path = "../murmur-core" }
harness = { path = "../harness" }
serde = { workspace = true }
serde_json = { workspace = true }

[dev-dependencies]
tokio = { workspace = true }
```

- [ ] **Step 2: Write the failing tests** (bottom of `crates/evals/src/corpus.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_truth_deserializes_with_defaults() {
        // contacts, distractors, tags default to empty; expects_summary defaults true
        let gt: FixtureGroundTruth =
            serde_json::from_str(r#"{"description":"x","items":[{"kind":"todo","text":"order lumber"}]}"#)
                .unwrap();
        assert_eq!(gt.items.len(), 1);
        assert!(gt.contacts.is_empty());
        assert!(gt.distractors.is_empty());
        assert!(gt.expects_summary, "defaults to true");
    }

    // These tests build synthetic fixtures in a temp dir — they exercise the
    // LOADER, not the real corpus (which Task 5 authors and asserts on). Keeping
    // Task 1 self-contained avoids a forward reference to files that don't exist
    // yet when the builder runs task-by-task.

    /// Writes a `<stem>.txt` + `<stem>.json` pair into `dir`.
    fn write_pair(dir: &std::path::Path, stem: &str, transcript: &str, json: &str) {
        std::fs::write(dir.join(format!("{stem}.txt")), transcript).unwrap();
        std::fs::write(dir.join(format!("{stem}.json")), json).unwrap();
    }

    fn fresh_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("evals-corpus-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn load_corpus_pairs_txt_and_json_by_stem() {
        let dir = fresh_dir("pairs");
        write_pair(&dir, "beta", "walking the back deck",
            r#"{"description":"d","items":[{"kind":"todo","text":"order lumber"}]}"#);
        write_pair(&dir, "alpha", "a short walk",
            r#"{"description":"d","items":[{"kind":"note","text":"soft joists"}]}"#);
        let scenarios = load_corpus(&dir).unwrap();
        let ids: Vec<&str> = scenarios.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["alpha", "beta"], "sorted by id");
        // transcript is wired to its truth
        let beta = scenarios.iter().find(|s| s.id == "beta").unwrap();
        assert!(beta.transcript.contains("deck"));
        assert_eq!(beta.truth.items.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_corpus_is_deterministic_order() {
        let dir = fresh_dir("order");
        write_pair(&dir, "zulu", "t", r#"{"description":"d","items":[]}"#);
        write_pair(&dir, "alpha", "t", r#"{"description":"d","items":[]}"#);
        let a: Vec<String> = load_corpus(&dir).unwrap().into_iter().map(|s| s.id).collect();
        let b: Vec<String> = load_corpus(&dir).unwrap().into_iter().map(|s| s.id).collect();
        assert_eq!(a, b, "corpus load order must be stable for comparable reports");
        assert_eq!(a, vec!["alpha", "zulu"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_json_for_a_txt_is_an_error() {
        // a .txt with no matching .json is a corpus authoring bug, not silently skipped
        let dir = fresh_dir("orphan");
        std::fs::write(dir.join("orphan.txt"), "some transcript").unwrap();
        let err = load_corpus(&dir).unwrap_err();
        assert!(err.to_string().contains("orphan"), "names the offending fixture");
        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 3: Implement** (`crates/evals/src/corpus.rs`)

```rust
//! Synthetic site-walk corpus: paired fixture files on disk.
//!
//! Each scenario is two files sharing a stem in `fixtures/`:
//!   - `<id>.txt`  — the transcript, plain text (natural disfluency, trade jargon)
//!   - `<id>.json` — typed ground truth: what SHOULD be extracted, plus
//!     `distractors` (spans R6 says must NOT become items) and `expects_summary`.
//!
//! Ground truth is reviewed *with the plan* (see Task 5). Transcripts are text so
//! they read naturally; truth is JSON so it is typed and diffable.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// One expected item: the kind it should be filed under and its gist. Matching
/// is fuzzy (normalized-token Dice, see `normalize`/`grade`), so `text` is the
/// *canonical* phrasing — the grader tolerates STT/phrasing drift.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ExpectedItem {
    pub kind: String,
    pub text: String,
}

/// One expected contact. `trade` optional: absent means "any trade (or none) is
/// acceptable"; present means the model should have captured that role.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ExpectedContact {
    pub name: String,
    #[serde(default)]
    pub trade: Option<String>,
}

fn default_true() -> bool {
    true
}

/// The on-disk JSON shape (`<id>.json`). `description`/`tags` are metadata;
/// the rest is ground truth.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FixtureGroundTruth {
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub items: Vec<ExpectedItem>,
    #[serde(default)]
    pub contacts: Vec<ExpectedContact>,
    /// Content that R6 says must NOT become items (hedging, chatter, incomplete
    /// thoughts, social filler). Each string is matched against produced items;
    /// a hit is an R6 violation (a false positive on a distractor).
    #[serde(default)]
    pub distractors: Vec<String>,
    #[serde(default = "default_true")]
    pub expects_summary: bool,
}

/// A loaded scenario: transcript + ground truth, ready to run and grade.
#[derive(Clone, Debug)]
pub struct Scenario {
    pub id: String,
    pub description: String,
    pub tags: Vec<String>,
    pub transcript: String,
    pub truth: GroundTruth,
}

/// Ground truth split out from fixture metadata for grading.
#[derive(Clone, Debug)]
pub struct GroundTruth {
    pub items: Vec<ExpectedItem>,
    pub contacts: Vec<ExpectedContact>,
    pub distractors: Vec<String>,
    pub expects_summary: bool,
}

/// Loads every `<id>.txt` + `<id>.json` pair from `dir`. A `.txt` with no
/// matching `.json` (or vice versa) is an error — a corpus authoring mistake
/// must be loud, not silently dropped. Order is sorted by id for stable,
/// comparable reports across runs.
pub fn load_corpus(dir: impl AsRef<Path>) -> io::Result<Vec<Scenario>> {
    let dir = dir.as_ref();
    // Collect stems and which extensions we saw.
    let mut txt: BTreeMap<String, std::path::PathBuf> = BTreeMap::new();
    let mut json: BTreeMap<String, std::path::PathBuf> = BTreeMap::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        match path.extension().and_then(|e| e.to_str()) {
            Some("txt") => {
                txt.insert(stem, path);
            }
            Some("json") => {
                json.insert(stem, path);
            }
            _ => {}
        }
    }
    let mut scenarios = Vec::new();
    for (stem, txt_path) in &txt {
        let json_path = json.remove(stem).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("fixture '{stem}' has {stem}.txt but no {stem}.json"),
            )
        })?;
        let transcript = fs::read_to_string(txt_path)?;
        let raw = fs::read_to_string(&json_path)?;
        let gt: FixtureGroundTruth = serde_json::from_str(&raw).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("{stem}.json: {e}"))
        })?;
        scenarios.push(Scenario {
            id: stem.clone(),
            description: gt.description,
            tags: gt.tags,
            transcript,
            truth: GroundTruth {
                items: gt.items,
                contacts: gt.contacts,
                distractors: gt.distractors,
                expects_summary: gt.expects_summary,
            },
        });
    }
    // Any leftover .json without a .txt is also an authoring error.
    if let Some((stem, _)) = json.into_iter().next() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("fixture '{stem}' has {stem}.json but no {stem}.txt"),
        ));
    }
    scenarios.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(scenarios)
}
```

`crates/evals/src/lib.rs`:
```rust
//! Extraction eval suite (Plan 05b): synthetic corpus + deterministic grader +
//! gated real-API runner. Foundation for a prompt-optimization loop — scores are
//! comparable across prompt variants. Pure consumer of `murmur-core` public API;
//! zero impact on shipping code.

pub mod corpus;
pub mod grade;
pub mod normalize;
pub mod report;
pub mod run;
```

> Note: `lib.rs` references `grade`/`normalize`/`report`/`run` which land in later tasks. Implement Task 1 with `lib.rs` declaring only `pub mod corpus;`, then add each `pub mod` line as its task lands (Task 2 adds `normalize`, Task 3 `grade`, Task 4 `report`, Task 6 `run`). The final shape is shown above.

- [ ] **Step 4: Seed the two loader-satisfying fixtures now** so Step 3's `load_corpus` tests can pass. Create minimal placeholders for the three seed ids (full authored transcripts land in Task 5; here just enough to load). Skip if you prefer to reorder Task 5 before Task 1's loader tests — but the loader tests reference the seed ids, so the fixtures must exist. Simplest: write the real Task 5 fixtures now and treat Task 5 as "review/extend."

- [ ] **Step 5: Run tests**

Run: `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo test -p evals corpus`

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(evals): crate skeleton + paired-fixture corpus loader"
```

---

### Task 2: Text normalization + Dice matcher (pure, deterministic)

**Files:**
- Create: `crates/evals/src/normalize.rs`
- Modify: `crates/evals/src/lib.rs` (add `pub mod normalize;`)

- [ ] **Step 1: Write the failing tests** (bottom of `crates/evals/src/normalize.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_lowercases_strips_punct_and_stopwords() {
        // "Order the lumber!" -> {order, lumber}  (the/for/a dropped, "!" gone)
        let t = token_set("Order the lumber!");
        assert!(t.contains("order"));
        assert!(t.contains("lumber"));
        assert!(!t.contains("the"));
    }

    #[test]
    fn normalize_strips_trailing_plural_s() {
        assert_eq!(token_set("joists"), token_set("joist"));
    }

    #[test]
    fn dice_is_one_for_identical_sets() {
        assert_eq!(dice(&token_set("order lumber"), &token_set("order lumber")), 1.0);
    }

    #[test]
    fn dice_is_order_independent() {
        let a = dice(&token_set("order the lumber"), &token_set("lumber order"));
        assert_eq!(a, 1.0, "stopword-stripped token SETS are equal regardless of order");
    }

    #[test]
    fn dice_is_zero_for_disjoint_sets() {
        assert_eq!(dice(&token_set("order lumber"), &token_set("call framer")), 0.0);
    }

    #[test]
    fn dice_partial_overlap_is_between() {
        // {order,lumber,deck} vs {order,lumber} -> 2*2/(3+2) = 0.8
        let d = dice(&token_set("order lumber deck"), &token_set("order lumber"));
        assert!((d - 0.8).abs() < 1e-9, "got {d}");
    }

    #[test]
    fn empty_sets_score_zero_not_nan() {
        assert_eq!(dice(&token_set(""), &token_set("")), 0.0);
        assert_eq!(dice(&token_set("order"), &token_set("")), 0.0);
    }
}
```

- [ ] **Step 2: Run to see failure**

Run: `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo test -p evals normalize`
Expected: compile FAIL.

- [ ] **Step 3: Implement** (`crates/evals/src/normalize.rs`)

```rust
//! Deterministic text normalization + set similarity for the grader. No
//! network, no randomness: same input → same output, every run. This is what
//! makes eval scores comparable across prompt variants.

use std::collections::BTreeSet;

/// A tiny, closed stopword list — words that carry no extraction signal and
/// only add noise to overlap. Kept small and fixed on purpose: a big list would
/// swallow real content ("no", "not"). Do NOT tune this per-corpus.
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "to", "of", "for", "and", "or", "is", "are", "was", "were",
    "on", "in", "at", "we", "i", "it", "that", "this", "with", "need", "needs",
];

/// Lowercase, split on non-alphanumerics, drop stopwords, strip a trailing
/// plural `s`, and collect into a set. Returns a `BTreeSet` for deterministic
/// iteration order (matters only for debug output; scores are set ops).
pub fn token_set(s: &str) -> BTreeSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .filter(|w| !STOPWORDS.contains(w))
        .map(strip_plural)
        .collect()
}

/// Strip a single trailing plural `s` (joists→joist), but not for 2-char words
/// (as→a would be wrong) or double-s (loss→los would be wrong).
fn strip_plural(w: &str) -> String {
    if w.len() > 3 && w.ends_with('s') && !w.ends_with("ss") {
        w[..w.len() - 1].to_string()
    } else {
        w.to_string()
    }
}

/// Dice coefficient: `2·|A∩B| / (|A|+|B|)`. Symmetric, order-independent, in
/// `[0,1]`. Empty-vs-anything is 0.0 (never NaN).
pub fn dice(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    let total = a.len() + b.len();
    if total == 0 {
        return 0.0;
    }
    let inter = a.intersection(b).count();
    (2.0 * inter as f64) / total as f64
}
```

Add `pub mod normalize;` to `lib.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo test -p evals normalize`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(evals): deterministic token normalization + Dice similarity"
```

---

### Task 3: The grader — matching, per-kind P/R/F1, confusion, contacts, R6, F0.5

**Files:**
- Create: `crates/evals/src/grade.rs`
- Modify: `crates/evals/src/lib.rs` (add `pub mod grade;`)

This task grades an **`Observed`** (what the pipeline produced) against a scenario's `GroundTruth`, producing a `ScenarioScore`. `Observed` is plain data (no store dependency) so grader tests need no pipeline. `run.rs` (Task 6) builds `Observed` from store rows.

- [ ] **Step 1: Write the failing tests** (bottom of `crates/evals/src/grade.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::{ExpectedContact, ExpectedItem, GroundTruth};

    fn item(kind: &str, text: &str) -> ExpectedItem {
        ExpectedItem { kind: kind.into(), text: text.into() }
    }
    fn obs_item(kind: &str, text: &str) -> ObservedItem {
        ObservedItem { kind: kind.into(), text: text.into() }
    }

    fn truth(items: Vec<ExpectedItem>) -> GroundTruth {
        GroundTruth { items, contacts: vec![], distractors: vec![], expects_summary: true }
    }

    #[test]
    fn perfect_extraction_scores_one() {
        let gt = truth(vec![item("todo", "order lumber"), item("safety", "loose railing")]);
        let obs = Observed {
            items: vec![obs_item("todo", "order the lumber"), obs_item("safety", "railing is loose")],
            contacts: vec![],
            summary_present: true,
        };
        let s = grade(&gt, &obs);
        assert_eq!(s.overall.true_positives, 2);
        assert_eq!(s.overall.false_positives, 0);
        assert_eq!(s.overall.false_negatives, 0);
        assert!((s.overall.precision - 1.0).abs() < 1e-9);
        assert!((s.overall.recall - 1.0).abs() < 1e-9);
        assert!((s.f_half - 1.0).abs() < 1e-9);
    }

    #[test]
    fn over_extraction_is_penalized_harder_than_under_by_f_half() {
        let gt = truth(vec![item("todo", "order lumber"), item("todo", "call inspector")]);
        // over-extraction: got both + 2 invented -> P=0.5, R=1.0
        let over = grade(&gt, &Observed {
            items: vec![
                obs_item("todo", "order lumber"), obs_item("todo", "call inspector"),
                obs_item("todo", "paint the fence"), obs_item("todo", "buy coffee"),
            ],
            contacts: vec![], summary_present: true,
        });
        // under-extraction: got one, missed one -> P=1.0, R=0.5
        let under = grade(&gt, &Observed {
            items: vec![obs_item("todo", "order lumber")],
            contacts: vec![], summary_present: true,
        });
        // R6: F0.5 weights precision, so the under-extractor scores HIGHER
        assert!(under.f_half > over.f_half, "under {} !> over {}", under.f_half, over.f_half);
    }

    #[test]
    fn wrong_kind_is_not_a_true_positive_but_shows_in_confusion() {
        let gt = truth(vec![item("safety", "loose railing")]);
        // right text, wrong kind (todo) -> FP + FN, and a confusion entry
        let s = grade(&gt, &Observed {
            items: vec![obs_item("todo", "loose railing")],
            contacts: vec![], summary_present: true,
        });
        assert_eq!(s.overall.true_positives, 0);
        assert_eq!(s.overall.false_positives, 1);
        assert_eq!(s.overall.false_negatives, 1);
        assert!(s.confusion.iter().any(|c| c.expected_kind == "safety" && c.produced_kind == "todo"));
    }

    #[test]
    fn distractor_hit_raises_r6_false_positive_rate() {
        let mut gt = truth(vec![item("todo", "order lumber")]);
        gt.distractors = vec!["might rain later maybe".into(), "grab lunch sometime".into()];
        // model wrongly turned a distractor into an item
        let s = grade(&gt, &Observed {
            items: vec![obs_item("todo", "order lumber"), obs_item("todo", "grab lunch sometime")],
            contacts: vec![], summary_present: true,
        });
        assert_eq!(s.distractor_count, 2);
        assert_eq!(s.distractor_hits, 1);
        assert!((s.distractor_fp_rate - 0.5).abs() < 1e-9);
    }

    #[test]
    fn contact_accuracy_matches_name_and_optional_trade() {
        let gt = GroundTruth {
            items: vec![], distractors: vec![], expects_summary: true,
            contacts: vec![
                ExpectedContact { name: "Dev".into(), trade: Some("framer".into()) },
                ExpectedContact { name: "Hank".into(), trade: None },
            ],
        };
        let s = grade(&gt, &Observed {
            items: vec![],
            contacts: vec![
                ObservedContact { name: "Dev".into(), trade: Some("framer".into()) },
                // Hank present, trade unknown — acceptable since expected trade is None
                ObservedContact { name: "Hank".into(), trade: None },
            ],
            summary_present: true,
        });
        assert_eq!(s.contacts_expected, 2);
        assert_eq!(s.contacts_matched, 2);
        assert!((s.contact_accuracy - 1.0).abs() < 1e-9);
    }

    #[test]
    fn wrong_trade_fails_the_contact_match() {
        let gt = GroundTruth {
            items: vec![], distractors: vec![], expects_summary: true,
            contacts: vec![ExpectedContact { name: "Dev".into(), trade: Some("framer".into()) }],
        };
        let s = grade(&gt, &Observed {
            items: vec![],
            contacts: vec![ObservedContact { name: "Dev".into(), trade: Some("plumber".into()) }],
            summary_present: true,
        });
        assert_eq!(s.contacts_matched, 0);
    }

    #[test]
    fn summary_presence_is_scored_against_expectation() {
        let gt = truth(vec![]);
        let missing = grade(&gt, &Observed { items: vec![], contacts: vec![], summary_present: false });
        assert!(!missing.summary_ok, "expected a summary, none produced");
        let present = grade(&gt, &Observed { items: vec![], contacts: vec![], summary_present: true });
        assert!(present.summary_ok);
    }

    #[test]
    fn per_kind_breakdown_is_reported() {
        let gt = truth(vec![item("todo", "order lumber"), item("safety", "loose railing")]);
        let s = grade(&gt, &Observed {
            items: vec![obs_item("todo", "order lumber")], // missed the safety item
            contacts: vec![], summary_present: true,
        });
        let todo = s.per_kind.iter().find(|k| k.kind == "todo").unwrap();
        assert!((todo.recall - 1.0).abs() < 1e-9);
        let safety = s.per_kind.iter().find(|k| k.kind == "safety").unwrap();
        assert!((safety.recall - 0.0).abs() < 1e-9);
    }
}
```

- [ ] **Step 2: Run to see failure**

Run: `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo test -p evals grade`
Expected: compile FAIL.

- [ ] **Step 3: Implement** (`crates/evals/src/grade.rs`)

Key logic — study these invariants before writing:
- **Item match** = same kind AND `dice(token_set(expected.text), token_set(candidate.text)) >= MATCH_THRESHOLD`.
- **Greedy bipartite:** enumerate all same-kind (expected_idx, candidate_idx, dice) triples, sort by dice desc (tie-break by (expected_idx, candidate_idx) for determinism), assign greedily, each idx used once.
- **Confusion:** for expected items still unmatched after same-kind matching, look for an unmatched candidate of a *different* kind whose text Dice ≥ threshold; record `(expected_kind, produced_kind)`. This is diagnostic only — it does not convert to a TP.
- **Distractor FP rate:** for each produced item, if it Dice-matches (any kind) any distractor string ≥ threshold, it's a distractor hit. `rate = hits / distractor_count` (0.0 when no distractors).
- **Contacts:** name Dice ≥ threshold; if expected `trade` is `Some`, produced trade must Dice-match it; if expected trade is `None`, trade is not checked. `contact_accuracy = matched / expected` (1.0 when none expected).
- **F0.5:** `(1+β²)·P·R / (β²·P + R)` with `β²=0.25`; 0.0 when denominator 0.

```rust
//! Deterministic grader (spec R6/R7). Grades an `Observed` (pipeline output as
//! plain data) against a scenario's `GroundTruth`. No store, no network — so
//! grader tests need no pipeline, and scores are reproducible across runs and
//! across prompt variants (the property a prompt-optimizer requires).
//!
//! Scoring encodes product values: F0.5 weights precision 2× recall (R6:
//! under-extraction is cheaper than over-extraction), and a distractor
//! false-positive rate measures chatter wrongly promoted to items.

use serde::{Deserialize, Serialize};

use crate::corpus::GroundTruth;
use crate::normalize::{dice, token_set};

/// Dice threshold for "these two texts are the same item". 0.5 = at least half
/// the combined token mass overlaps. Tuned once, fixed — moving it per-corpus
/// would make scores incomparable.
pub const MATCH_THRESHOLD: f64 = 0.5;

/// β² for F0.5 — precision weighted 2× recall (R6).
const BETA_SQ: f64 = 0.25;

/// Pipeline output as plain data (built from store rows by `run.rs`).
#[derive(Clone, Debug)]
pub struct Observed {
    pub items: Vec<ObservedItem>,
    pub contacts: Vec<ObservedContact>,
    pub summary_present: bool,
}

#[derive(Clone, Debug)]
pub struct ObservedItem {
    pub kind: String,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct ObservedContact {
    pub name: String,
    pub trade: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PrecisionRecall {
    pub true_positives: usize,
    pub false_positives: usize,
    pub false_negatives: usize,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
}

impl PrecisionRecall {
    fn from_counts(tp: usize, fp: usize, fn_: usize) -> Self {
        let precision = ratio(tp, tp + fp);
        let recall = ratio(tp, tp + fn_);
        let f1 = harmonic(precision, recall, 1.0);
        PrecisionRecall { true_positives: tp, false_positives: fp, false_negatives: fn_, precision, recall, f1 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct KindScore {
    pub kind: String,
    pub true_positives: usize,
    pub false_positives: usize,
    pub false_negatives: usize,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ConfusionEntry {
    pub expected_kind: String,
    pub produced_kind: String,
    pub count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScenarioScore {
    pub overall: PrecisionRecall,
    pub per_kind: Vec<KindScore>,
    pub confusion: Vec<ConfusionEntry>,
    /// F0.5 over all items (precision-weighted, R6). The headline scalar.
    pub f_half: f64,
    pub contacts_expected: usize,
    pub contacts_matched: usize,
    pub contact_accuracy: f64,
    pub distractor_count: usize,
    pub distractor_hits: usize,
    /// hits / distractor_count — the R6 over-extraction signal (lower better).
    pub distractor_fp_rate: f64,
    pub summary_ok: bool,
}

fn ratio(num: usize, den: usize) -> f64 {
    if den == 0 { 0.0 } else { num as f64 / den as f64 }
}

fn harmonic(p: f64, r: f64, beta_sq: f64) -> f64 {
    let den = beta_sq * p + r;
    if den == 0.0 { 0.0 } else { (1.0 + beta_sq) * p * r / den }
}

/// Grades one scenario. See module docs for the matching contract.
pub fn grade(truth: &GroundTruth, obs: &Observed) -> ScenarioScore {
    // Pre-tokenize once.
    let exp_tok: Vec<_> = truth.items.iter().map(|i| (i.kind.as_str(), token_set(&i.text))).collect();
    let obs_tok: Vec<_> = obs.items.iter().map(|i| (i.kind.as_str(), token_set(&i.text))).collect();

    // --- Same-kind greedy bipartite matching ---
    let mut exp_used = vec![false; exp_tok.len()];
    let mut obs_used = vec![false; obs_tok.len()];
    let mut pairs: Vec<(usize, usize, f64)> = Vec::new();
    for (ei, (ek, et)) in exp_tok.iter().enumerate() {
        for (oi, (ok, ot)) in obs_tok.iter().enumerate() {
            if ek == ok {
                let d = dice(et, ot);
                if d >= MATCH_THRESHOLD {
                    pairs.push((ei, oi, d));
                }
            }
        }
    }
    // Deterministic: highest Dice first, tie-break by indices.
    pairs.sort_by(|a, b| {
        b.2.partial_cmp(&a.2).unwrap().then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1))
    });
    for (ei, oi, _) in &pairs {
        if !exp_used[*ei] && !obs_used[*oi] {
            exp_used[*ei] = true;
            obs_used[*oi] = true;
        }
    }

    // --- Confusion: unmatched expected vs unmatched cross-kind candidate ---
    let mut confusion: Vec<ConfusionEntry> = Vec::new();
    for (ei, (ek, et)) in exp_tok.iter().enumerate() {
        if exp_used[ei] { continue; }
        for (oi, (ok, ot)) in obs_tok.iter().enumerate() {
            if obs_used[oi] || ek == ok { continue; }
            if dice(et, ot) >= MATCH_THRESHOLD {
                bump_confusion(&mut confusion, ek, ok);
                // Diagnostic only: leave counts as FP/FN. Mark neither used —
                // a wrong-kind produced item is still a false positive.
                break;
            }
        }
    }

    let tp = exp_used.iter().filter(|u| **u).count();
    let fn_ = exp_tok.len() - tp;
    let fp = obs_tok.len() - obs_used.iter().filter(|u| **u).count();
    let overall = PrecisionRecall::from_counts(tp, fp, fn_);
    let f_half = harmonic(overall.precision, overall.recall, BETA_SQ);

    // --- Per-kind breakdown ---
    let mut kinds: Vec<String> = exp_tok.iter().map(|(k, _)| k.to_string())
        .chain(obs_tok.iter().map(|(k, _)| k.to_string())).collect();
    kinds.sort();
    kinds.dedup();
    let per_kind = kinds.iter().map(|kind| {
        let ktp = (0..exp_tok.len()).filter(|&i| exp_used[i] && exp_tok[i].0 == kind).count();
        let kfn = (0..exp_tok.len()).filter(|&i| !exp_used[i] && exp_tok[i].0 == kind).count();
        let kfp = (0..obs_tok.len()).filter(|&i| !obs_used[i] && obs_tok[i].0 == kind).count();
        let pr = PrecisionRecall::from_counts(ktp, kfp, kfn);
        KindScore {
            kind: kind.clone(),
            true_positives: pr.true_positives, false_positives: pr.false_positives,
            false_negatives: pr.false_negatives, precision: pr.precision, recall: pr.recall, f1: pr.f1,
        }
    }).collect();

    // --- R6 distractor false positives (any kind) ---
    let distractor_toks: Vec<_> = truth.distractors.iter().map(|d| token_set(d)).collect();
    let distractor_hits = obs_tok.iter().filter(|(_, ot)| {
        distractor_toks.iter().any(|dt| dice(dt, ot) >= MATCH_THRESHOLD)
    }).count();
    let distractor_fp_rate = ratio(distractor_hits, truth.distractors.len());

    // --- Contacts ---
    let (contacts_matched, contact_accuracy) = grade_contacts(truth, obs);

    ScenarioScore {
        overall, per_kind, confusion, f_half,
        contacts_expected: truth.contacts.len(),
        contacts_matched,
        contact_accuracy,
        distractor_count: truth.distractors.len(),
        distractor_hits,
        distractor_fp_rate,
        summary_ok: obs.summary_present == truth.expects_summary,
    }
}

fn bump_confusion(confusion: &mut Vec<ConfusionEntry>, expected: &str, produced: &str) {
    if let Some(e) = confusion.iter_mut().find(|c| c.expected_kind == expected && c.produced_kind == produced) {
        e.count += 1;
    } else {
        confusion.push(ConfusionEntry { expected_kind: expected.into(), produced_kind: produced.into(), count: 1 });
    }
}

fn grade_contacts(truth: &GroundTruth, obs: &Observed) -> (usize, f64) {
    let mut used = vec![false; obs.contacts.len()];
    let mut matched = 0;
    for ec in &truth.contacts {
        let en = token_set(&ec.name);
        for (oi, oc) in obs.contacts.iter().enumerate() {
            if used[oi] { continue; }
            if dice(&en, &token_set(&oc.name)) < MATCH_THRESHOLD { continue; }
            // trade only checked when expected trade is Some
            let trade_ok = match (&ec.trade, &oc.trade) {
                (Some(et), Some(ot)) => dice(&token_set(et), &token_set(ot)) >= MATCH_THRESHOLD,
                (Some(_), None) => false,
                (None, _) => true,
            };
            if trade_ok {
                used[oi] = true;
                matched += 1;
                break;
            }
        }
    }
    let accuracy = if truth.contacts.is_empty() { 1.0 } else { matched as f64 / truth.contacts.len() as f64 };
    (matched, accuracy)
}
```

Add `pub mod grade;` to `lib.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo test -p evals grade`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(evals): deterministic grader — per-kind P/R/F1, confusion, contacts, R6, F0.5"
```

---

### Task 4: Report aggregation + JSON + summary table + cost

**Files:**
- Create: `crates/evals/src/report.rs`
- Modify: `crates/evals/src/lib.rs` (add `pub mod report;`)

- [ ] **Step 1: Write the failing tests** (bottom of `crates/evals/src/report.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::grade::{PrecisionRecall, ScenarioScore};

    fn score(f_half: f64, distractor_fp: f64) -> ScenarioScore {
        ScenarioScore {
            overall: PrecisionRecall { true_positives: 1, false_positives: 0, false_negatives: 0, precision: 1.0, recall: 1.0, f1: 1.0 },
            per_kind: vec![], confusion: vec![], f_half,
            contacts_expected: 0, contacts_matched: 0, contact_accuracy: 1.0,
            distractor_count: 2, distractor_hits: (distractor_fp * 2.0) as usize, distractor_fp_rate: distractor_fp,
            summary_ok: true,
        }
    }

    #[test]
    fn suite_aggregate_means_across_scenarios() {
        let suite = SuiteReport::assemble("claude-haiku-4-5", vec![
            ScenarioReport { id: "a".into(), score: score(1.0, 0.0), cost: CostReport::default() },
            ScenarioReport { id: "b".into(), score: score(0.0, 1.0), cost: CostReport::default() },
        ]);
        assert!((suite.aggregate.mean_f_half - 0.5).abs() < 1e-9);
        assert!((suite.aggregate.mean_distractor_fp_rate - 0.5).abs() < 1e-9);
    }

    #[test]
    fn report_serializes_to_stable_json() {
        let suite = SuiteReport::assemble("m", vec![
            ScenarioReport { id: "a".into(), score: score(1.0, 0.0), cost: CostReport::default() },
        ]);
        let json = serde_json::to_string_pretty(&suite).unwrap();
        // round-trips and contains the headline scalar
        assert!(json.contains("mean_f_half"));
        let back: SuiteReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.model, "m");
    }

    #[test]
    fn cost_estimate_uses_price_table() {
        // 1M input + 1M output tokens at the documented haiku rate
        let c = CostReport::estimate("claude-haiku-4-5", 1_000_000, 1_000_000);
        assert!(c.est_usd > 0.0);
        assert_eq!(c.input_tokens, 1_000_000);
    }

    #[test]
    fn table_renders_one_row_per_scenario_and_a_total() {
        let suite = SuiteReport::assemble("m", vec![
            ScenarioReport { id: "deck".into(), score: score(0.8, 0.0), cost: CostReport::default() },
        ]);
        let table = render_table(&suite);
        assert!(table.contains("deck"));
        assert!(table.contains("F0.5"));
        assert!(table.contains("TOTAL") || table.contains("mean"));
    }
}
```

- [ ] **Step 2: Run to see failure**, then **Step 3: Implement** (`crates/evals/src/report.rs`)

```rust
//! Machine-comparable eval report. `SuiteReport` serializes to JSON (for a
//! prompt-optimizer to diff variants) and renders a human summary table. Cost
//! is estimated from R9 usage tokens via a documented price table.

use serde::{Deserialize, Serialize};

use crate::grade::ScenarioScore;

/// Price table ($ per 1M tokens). APPROXIMATE — confirm against current
/// Anthropic pricing before trusting the dollar column; token counts are exact
/// (from the R9 usage log), only the $ conversion is a constant here. Unknown
/// models fall back to the haiku rate and are flagged in the report.
fn price_per_mtok(model: &str) -> (f64, f64) {
    match model {
        m if m.contains("haiku") => (1.00, 5.00),
        m if m.contains("sonnet") => (3.00, 15.00),
        m if m.contains("opus") => (15.00, 75.00),
        _ => (1.00, 5.00),
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CostReport {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub est_usd: f64,
}

impl CostReport {
    pub fn estimate(model: &str, input_tokens: u64, output_tokens: u64) -> Self {
        let (in_rate, out_rate) = price_per_mtok(model);
        let est_usd = (input_tokens as f64 / 1e6) * in_rate + (output_tokens as f64 / 1e6) * out_rate;
        CostReport { input_tokens, output_tokens, est_usd }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScenarioReport {
    pub id: String,
    pub score: ScenarioScore,
    pub cost: CostReport,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Aggregate {
    pub scenarios: usize,
    pub mean_f_half: f64,
    pub micro_precision: f64,
    pub micro_recall: f64,
    pub mean_distractor_fp_rate: f64,
    pub mean_contact_accuracy: f64,
    pub summaries_ok: usize,
    pub total_cost_usd: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SuiteReport {
    pub model: String,
    pub scenarios: Vec<ScenarioReport>,
    pub aggregate: Aggregate,
}

impl SuiteReport {
    pub fn assemble(model: impl Into<String>, scenarios: Vec<ScenarioReport>) -> Self {
        let n = scenarios.len().max(1) as f64;
        let mean_f_half = scenarios.iter().map(|s| s.score.f_half).sum::<f64>() / n;
        let (tp, fp, fn_) = scenarios.iter().fold((0, 0, 0), |(tp, fp, fnn), s| {
            (tp + s.score.overall.true_positives, fp + s.score.overall.false_positives, fnn + s.score.overall.false_negatives)
        });
        let micro_precision = if tp + fp == 0 { 0.0 } else { tp as f64 / (tp + fp) as f64 };
        let micro_recall = if tp + fn_ == 0 { 0.0 } else { tp as f64 / (tp + fn_) as f64 };
        let mean_distractor_fp_rate = scenarios.iter().map(|s| s.score.distractor_fp_rate).sum::<f64>() / n;
        let mean_contact_accuracy = scenarios.iter().map(|s| s.score.contact_accuracy).sum::<f64>() / n;
        let summaries_ok = scenarios.iter().filter(|s| s.score.summary_ok).count();
        let total_cost_usd = scenarios.iter().map(|s| s.cost.est_usd).sum();
        SuiteReport {
            model: model.into(),
            aggregate: Aggregate {
                scenarios: scenarios.len(),
                mean_f_half, micro_precision, micro_recall,
                mean_distractor_fp_rate, mean_contact_accuracy, summaries_ok, total_cost_usd,
            },
            scenarios,
        }
    }
}

/// Fixed-width summary table. Purely for humans; the JSON is the machine artifact.
pub fn render_table(suite: &SuiteReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("model: {}\n", suite.model));
    out.push_str(&format!("{:<24} {:>6} {:>6} {:>6} {:>7} {:>8} {:>8}\n",
        "scenario", "F0.5", "P", "R", "distFP", "contact", "usd"));
    for s in &suite.scenarios {
        out.push_str(&format!("{:<24} {:>6.2} {:>6.2} {:>6.2} {:>7.2} {:>8.2} {:>8.4}\n",
            s.id, s.score.f_half, s.score.overall.precision, s.score.overall.recall,
            s.score.distractor_fp_rate, s.score.contact_accuracy, s.cost.est_usd));
    }
    let a = &suite.aggregate;
    out.push_str(&format!("{:<24} {:>6.2} {:>6.2} {:>6.2} {:>7.2} {:>8.2} {:>8.4}\n",
        "TOTAL (mean)", a.mean_f_half, a.micro_precision, a.micro_recall,
        a.mean_distractor_fp_rate, a.mean_contact_accuracy, a.total_cost_usd));
    out.push_str(&format!("summaries ok: {}/{}\n", a.summaries_ok, a.scenarios));
    out
}
```

Add `pub mod report;` to `lib.rs`.

- [ ] **Step 4: Run tests, Step 5: Commit**

```bash
git add -A && git commit -m "feat(evals): comparable JSON report + summary table + token-cost estimate"
```

---

### Task 5: Seed corpus — 3 authored transcripts + ground truth (reviewed with the plan)

**Files:**
- Create: `crates/evals/fixtures/{punch_list_short,deck_walk_contacts,rambling_long_walk}.{txt,json}`
- Create: `crates/evals/fixtures/empty_session.{txt,json}` (carried scenario d)

> These three seeds establish ground-truth conventions and are reviewed *with this plan*. Implementers extend to **8–12 total** using the recipe at the end of this task.

- [ ] **Step 1: Seed 1 — short punch-list walk.** `punch_list_short.txt`:

```
Okay, punch list for unit twelve. Kitchen faucet's still dripping, needs a new
cartridge. Bedroom outlet by the window is dead, get the electrician back on
that. And the closet door doesn't latch, hinge is sprung. That's it for twelve.
```

`punch_list_short.json`:
```json
{
  "description": "Short punch-list walk: three crisp todos, no contacts, no chatter.",
  "tags": ["short", "punch-list"],
  "items": [
    { "kind": "todo", "text": "replace kitchen faucet cartridge, still dripping" },
    { "kind": "todo", "text": "electrician back on dead bedroom outlet by window" },
    { "kind": "todo", "text": "fix closet door latch, sprung hinge" }
  ],
  "contacts": [],
  "distractors": [],
  "expects_summary": true
}
```

- [ ] **Step 2: Seed 2 — medium deck walk with contacts, safety, decision, part, price, and distractors.** `deck_walk_contacts.txt`:

```
Alright, walking the back deck with Dev, he's the framer. So, uh, the joists on
the north side are soft — probably rot under the ledger. We're gonna pull those
two and swap 'em out. Dev says order two sixteen-foot pressure-treated
two-by-tens, should run about ninety bucks at the yard. Big thing, safety — that top railing
is loose, nobody leans on it till it's re-anchored, I mean it. Oh and Dev, call
him Thursday to schedule the swap. Um, weather looks like it might rain later,
maybe, we'll see. Decided we're going with the hidden fasteners on the decking
instead of face screws, cleaner look. Anyway, I think that's the deck. Grab
lunch after this.
```

`deck_walk_contacts.json`:
```json
{
  "description": "Medium deck walk: contact w/ role, safety, decision, part, price, plus weather/lunch chatter that R6 must NOT extract.",
  "tags": ["medium", "contacts", "safety", "r6"],
  "items": [
    { "kind": "safety", "text": "top railing loose, do not lean until re-anchored" },
    { "kind": "todo", "text": "pull the two soft north-side joists and swap them out" },
    { "kind": "part", "text": "two sixteen-foot pressure-treated two-by-tens" },
    { "kind": "price", "text": "about ninety dollars for the lumber at the yard" },
    { "kind": "todo", "text": "call Dev Thursday to schedule the joist swap" },
    { "kind": "decision", "text": "use hidden fasteners on the decking instead of face screws" }
  ],
  "contacts": [
    { "name": "Dev", "trade": "framer" }
  ],
  "distractors": [
    "might rain later maybe",
    "grab lunch after this"
  ],
  "expects_summary": true
}
```

- [ ] **Step 3: Seed 3 — long rambling walk exceeding the live window budget** (carried scenario c source). `rambling_long_walk.txt` should be **~700+ words** of continuous single-speaker rambling across multiple rooms/systems, with jargon, self-corrections ("no wait, that's the other bathroom"), tangents, and a handful of genuine items scattered throughout. Author it so the transcript char count comfortably exceeds `LiveExtractor`'s default `transcript_window_tokens` (2000 tokens ≈ 8000 chars) — this drives multi-pass catch-up. Include **8–12 genuine items** and **4–6 distractors** (weather, coffee, unrelated stories). Ground truth JSON follows the same shape; `tags` include `["long", "window-budget", "multi-pass"]`.

Author the full transcript inline when implementing (the reviewer of *this* plan reviews the two shorter seeds' ground truth as the convention; the long one follows identical rules). Keep every ground-truth item traceable to a literal span in the transcript — no inferred items.

- [ ] **Step 4: Carried scenario d — empty/near-empty.** `empty_session.txt`: a single line of pure filler, e.g. `Uh. Okay. Hmm, yeah.` — no genuine items. `empty_session.json`:
```json
{
  "description": "Near-empty session: only filler. Nothing to extract; summary may be the placeholder.",
  "tags": ["empty", "edge"],
  "items": [],
  "contacts": [],
  "distractors": ["uh okay hmm yeah"],
  "expects_summary": true
}
```
(Note: `process()` gives a truly-empty transcript the `(empty session)` placeholder summary with zero usage — so `summary_present` is true. A filler-but-nonempty transcript does hit the LLM. Set `expects_summary: true` for both; the near-empty file above is nonempty filler so it exercises the real path with an expected empty item set — a pure R6 stress: the model should extract nothing.)

- [ ] **Step 5: Extension recipe (for implementers, 8–12 total).** Document at the top of a new `crates/evals/fixtures/README.md`:
  - Each scenario = `<id>.txt` + `<id>.json`, ground truth traceable to literal transcript spans (no inferred items).
  - Cover the kind space: every `VALID_KINDS` value (`todo, decision, note, safety, part, price`) appears in ≥2 scenarios.
  - Every scenario except pure punch-lists carries ≥2 `distractors` (R6 is only measured where there's chatter to resist).
  - Vary length: ≥2 short (<150 words), ≥2 medium, ≥2 long (>500 words).
  - Vary trades: framing, plumbing, electrical, concrete, roofing vocabulary.
  - Include ≥1 STT-garble scenario (misrecognized jargon/names the model should still normalize via memory — e.g. "french drain" heard as "trench rain").
  - Target 8–12 fixtures; the grader and runner are corpus-size-agnostic.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(evals): seed site-walk corpus (punch-list, deck walk, long ramble, empty) + extension recipe"
```

---

### Task 6: Runner — hermetic (MockProvider) + carried characterization scenarios

**Files:**
- Create: `crates/evals/src/run.rs`, `crates/evals/tests/grader_hermetic.rs`, `crates/evals/tests/carried_scenarios.rs`
- Modify: `crates/evals/src/lib.rs` (add `pub mod run;`)

`run.rs` provides the shared plumbing both modes use: build a store, run the pipeline for a scenario, read back `Observed` + cost from store rows, grade.

- [ ] **Step 1: Implement `run.rs`** (the observe-from-store + grade helpers; a real-provider path used by the example in Task 7)

```rust
//! Runner plumbing shared by hermetic and real-API modes. Runs one scenario
//! through the *real* pipeline types (`SessionProcessor`, optionally
//! `LiveExtractor`) against whatever `LlmProvider` is supplied, then reads the
//! store back into an `Observed` and prices the R9 usage rows.

use std::sync::{Arc, Mutex};

use harness::{LlmProvider, Memory, MemoryStore, HarnessError};
use murmur_core::{SessionProcessor, Store};

use crate::corpus::Scenario;
use crate::grade::{grade, Observed, ObservedContact, ObservedItem, ScenarioScore};
use crate::report::{CostReport, ScenarioReport};

/// Memory store stub — evals don't persist memory.
pub struct NullMemoryStore;
impl MemoryStore for NullMemoryStore {
    fn load(&self) -> Result<Memory, HarnessError> { Ok(Memory::default()) }
    fn save(&self, _m: &Memory) -> Result<(), HarnessError> { Ok(()) }
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
    let guard = store.lock().map_err(|_| murmur_core::CoreError::InvalidState("store lock poisoned".into()))?;
    let items = guard.list_items_for_session(session_id)?
        .into_iter().map(|i| ObservedItem { kind: i.kind, text: i.text }).collect();
    let contacts = guard.list_contacts()?
        .into_iter().map(|c| ObservedContact { name: c.name, trade: c.trade }).collect();
    let summary_present = guard.get_session(session_id)?.summary
        .map(|s| !s.trim().is_empty() && s != "(empty session)").unwrap_or(false);
    let (input_tokens, output_tokens) = guard.list_llm_usage_for_session(session_id)?
        .iter().fold((0u64, 0u64), |(i, o), r| (i + r.input_tokens, o + r.output_tokens));
    let cost = CostReport::estimate(model, input_tokens, output_tokens);
    Ok((Observed { items, contacts, summary_present }, cost))
}
```

> Contacts are global (`list_contacts()` has no session filter). In an in-memory store per scenario that's fine — the store is fresh. The real-API runner (Task 7) uses one fresh in-memory store per scenario for the same reason.
> `summary_present` treats the `(empty session)` placeholder as "no real summary" so the empty-scenario grading is honest; ground truth for the near-empty fixture accounts for this.

Add `pub mod run;` to `lib.rs`.

- [ ] **Step 2: Hermetic suite test** — `crates/evals/tests/grader_hermetic.rs`: load the seed corpus, run each scenario against a **scripted `MockProvider`** that returns the *ideal* extraction, assert the grader reports near-perfect scores. This tests the whole harness (loader → pipeline → observe → grade → report) without a key.

```rust
//! Hermetic end-to-end: seed corpus → pipeline (MockProvider) → grade → report.
//! No network, runs under `cargo test --workspace`. Proves the harness wiring;
//! a perfect scripted extraction must grade ~1.0 so real-API regressions are
//! attributable to the model, not the harness.

use std::sync::Arc;

use evals::corpus::load_corpus;
use evals::report::{render_table, SuiteReport};
use evals::run::run_scenario;
use harness::{CompletionResponse, ContentBlock, MockProvider, StopReason, Usage};

fn tool_use(name: &str, input: serde_json::Value) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse { id: "tu".into(), name: name.into(), input }],
        stop_reason: StopReason::ToolUse,
        usage: Usage { input_tokens: 50, output_tokens: 10 },
    }
}
fn end_turn(t: &str) -> CompletionResponse {
    CompletionResponse { content: vec![ContentBlock::Text { text: t.into() }], stop_reason: StopReason::EndTurn, usage: Usage { input_tokens: 20, output_tokens: 4 } }
}
fn summary(t: &str) -> CompletionResponse {
    tool_use("write_summary", serde_json::json!({ "summary": t }))
}

/// Builds a MockProvider script that emits exactly the scenario's ground-truth
/// items + contacts, then a summary — the "perfect model".
fn perfect_script(scenario: &evals::corpus::Scenario) -> Vec<CompletionResponse> {
    let mut r = Vec::new();
    for it in &scenario.truth.items {
        r.push(tool_use("add_item", serde_json::json!({ "kind": it.kind, "text": it.text })));
    }
    for c in &scenario.truth.contacts {
        let mut input = serde_json::json!({ "name": c.name });
        if let Some(t) = &c.trade { input["trade"] = serde_json::json!(t); }
        r.push(tool_use("upsert_contact", input));
    }
    r.push(end_turn("done"));
    r.push(summary("Session processed."));
    r
}

#[tokio::test]
async fn perfect_model_grades_near_one_across_seed_corpus() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");
    let corpus = load_corpus(dir).unwrap();
    let mut reports = Vec::new();
    for scenario in &corpus {
        // empty/near-empty: no items scripted, just a summary
        let script = if scenario.truth.items.is_empty() && scenario.truth.contacts.is_empty() {
            vec![end_turn("nothing"), summary("Nothing notable.")]
        } else {
            perfect_script(scenario)
        };
        let report = run_scenario(scenario, Arc::new(MockProvider::new(script)), "claude-haiku-4-5").await.unwrap();
        reports.push(report);
    }
    let suite = SuiteReport::assemble("claude-haiku-4-5", reports);
    // A perfect scripted extraction must score high — proves harness fidelity.
    assert!(suite.aggregate.mean_f_half > 0.95, "harness lost fidelity: {}", suite.aggregate.mean_f_half);
    assert_eq!(suite.aggregate.mean_distractor_fp_rate, 0.0, "perfect model extracts no distractors");
    // table renders without panic
    let _ = render_table(&suite);
}

#[tokio::test]
async fn over_extracting_model_scores_lower_than_perfect() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");
    let deck = load_corpus(dir).unwrap().into_iter().find(|s| s.id == "deck_walk_contacts").unwrap();
    // perfect
    let perfect = run_scenario(&deck, Arc::new(MockProvider::new(perfect_script(&deck))), "m").await.unwrap();
    // greedy model: perfect + both distractors promoted to todos
    let mut greedy = perfect_script(&deck);
    let insert_at = greedy.len() - 2; // before end_turn + summary
    for d in &deck.truth.distractors {
        greedy.insert(insert_at, tool_use("add_item", serde_json::json!({ "kind": "todo", "text": d })));
    }
    let over = run_scenario(&deck, Arc::new(MockProvider::new(greedy)), "m").await.unwrap();
    assert!(over.score.distractor_fp_rate > 0.0, "distractors should be caught");
    assert!(over.score.f_half < perfect.score.f_half, "R6: over-extraction must score lower");
}
```

- [ ] **Step 3: Carried characterization scenarios** — `crates/evals/tests/carried_scenarios.rs`. These are **characterization/expected-behavior tests** (not fixes) that pin known Plan 05/06 gaps quantitatively. Each is hermetic.

```rust
//! Carried from the Plan 05 final review: characterization tests that PIN known
//! gaps (not fixes). Each documents current behavior so a later plan's fix has a
//! baseline to move. All hermetic (MockProvider).

use std::sync::{Arc, Mutex};

use harness::{CompletionResponse, ContentBlock, MockProvider, Memory, StopReason, Usage};
use murmur_core::{LiveExtractor, SessionProcessor, SessionStatus, Store};
// `evals::run::NullMemoryStore` is public (Task 6 Step 1) — reuse it, don't redeclare.

fn tool_use(name: &str, input: serde_json::Value) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::ToolUse { id: "tu".into(), name: name.into(), input }],
        stop_reason: StopReason::ToolUse,
        usage: Usage { input_tokens: 30, output_tokens: 8 },
    }
}

fn end_turn(t: &str) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::Text { text: t.into() }],
        stop_reason: StopReason::EndTurn,
        usage: Usage { input_tokens: 10, output_tokens: 2 },
    }
}

/// 4a — failed-processing-after-live-capture. Live pass populates the board;
/// then process() clears outputs at its top and FAILS before re-creating them.
/// CHARACTERIZATION: the board is left EMPTY (the known swap gap — live items
/// were tombstoned, the authoritative set never landed). This is the documented
/// behavior, asserted so a Plan 06 fix (e.g. defer clear until success) has a
/// baseline. Not a fix.
#[tokio::test]
async fn failed_processing_after_live_capture_leaves_empty_board() {
    let store = Store::open_in_memory("dev").unwrap();
    let session = store.start_session(None).unwrap();
    store.append_transcript(&session.id, "order lumber for the deck framing today").unwrap();
    let sid = session.id.clone();
    let store = Arc::new(Mutex::new(store));
    let memory = Arc::new(Mutex::new(Memory::default()));

    // Live pass captures one item.
    let mut live = LiveExtractor::new(
        Arc::new(MockProvider::new(vec![
            tool_use("add_item", serde_json::json!({"kind":"todo","text":"order lumber"})),
            end_turn("captured"),
        ])), store.clone(), memory.clone(), &sid);
    live.min_new_chars = 1;
    live.maybe_extract().await.unwrap();
    assert_eq!(store.lock().unwrap().list_items_for_session(&sid).unwrap().len(), 1);

    // End, then process with a provider that FAILS (summary returns no tool).
    store.lock().unwrap().end_and_record_session(&sid).unwrap();
    let processor = SessionProcessor::new(
        Arc::new(MockProvider::new(vec![end_turn("no extraction"), end_turn("no summary tool")])),
        store.clone(), memory, Arc::new(evals::run::NullMemoryStore));
    assert!(processor.process(&sid).await.is_err());

    // CHARACTERIZED GAP: clear_session_outputs already tombstoned the live item,
    // and the failed pass created nothing → board is empty despite a live capture.
    let after = store.lock().unwrap().list_items_for_session(&sid).unwrap();
    assert_eq!(after.len(), 0, "documents the swap gap: live board lost on processing failure");
    // Session is Failed (retry affordance exists, R7).
    assert_eq!(store.lock().unwrap().get_session(&sid).unwrap().status, SessionStatus::Failed);
}

/// 4b — restart-mid-session at a large item count. A fresh LiveExtractor starts
/// at cursor 0; the already-captured dedup list is budget-capped
/// (`already_captured_budget_tokens` = 400 tokens ⇒ `budget_chars(400)` = 1600
/// chars), and `format_already_captured` renders newest-first, so the assembler
/// truncates the OLDEST lines. An item that has scrolled out of that window is
/// INVISIBLE to the restart pass — the dedup blindspot — so the model re-adds it.
///
/// Budget arithmetic (why 80 items, not 64): each line "- [todo] task number NN"
/// is ~23 chars; N items render ≈ 23·N + (N−1) chars. At **64** items that is
/// ≈ 1_575 chars — UNDER the 1_600-char budget, so NOTHING is evicted and the
/// test would be vacuous (the reviewer's finding). At **80** items it is ≈ 1_930
/// chars — comfortably over budget — so the tail (oldest) is cut. The oldest item
/// is given a unique marker so the assertions can't alias another line.
///
/// CHARACTERIZATION for the Plan 06 dedup fix. Not a fix.
#[tokio::test]
async fn restart_after_many_items_re_adds_an_evicted_item() {
    let store = Store::open_in_memory("dev").unwrap();
    let session = store.start_session(None).unwrap();
    store.append_transcript(&session.id, "long live session, eighty-plus captured tasks and more talk").unwrap();
    let sid = session.id.clone();
    // Oldest item first (insertion = id order); unique marker so no substring alias.
    store.add_item(&sid, "todo", "oldest evicted marker task").unwrap();
    for n in 1..80 { store.add_item(&sid, "todo", &format!("task number {n:02}")).unwrap(); }
    let store = Arc::new(Mutex::new(store));

    // Fresh extractor (app restart) at cursor 0. Because the oldest item is no
    // longer in the (truncated) already-captured list, the pass re-extracts it.
    let provider = Arc::new(MockProvider::new(vec![
        tool_use("add_item", serde_json::json!({"kind":"todo","text":"oldest evicted marker task"})),
        end_turn("re-captured what looked new"),
    ]));
    let mut live = LiveExtractor::new(
        provider.clone(), store.clone(), Arc::new(Mutex::new(Memory::default())), &sid);
    live.min_new_chars = 1;
    live.maybe_extract().await.unwrap();

    // PRIMARY — the blindspot itself: assert against what was actually SENT. The
    // oldest item's text is ABSENT from the already-captured section of the
    // request (evicted by budget truncation) even though it IS on the board; a
    // recent item survives. This is the defect, independent of add_item dedup.
    let reqs = provider.requests();
    let user_text = match &reqs[0].messages[0].content[0] {
        ContentBlock::Text { text } => text.clone(),
        other => panic!("expected user text block, got {other:?}"),
    };
    assert!(user_text.contains("already captured"), "the dedup section is present");
    assert!(user_text.contains("task number 79"), "a recent item survives truncation");
    assert!(
        !user_text.contains("oldest evicted marker task"),
        "oldest item was evicted from the dedup window — the restart blindspot"
    );

    // SECONDARY consequence: the board now holds the oldest item twice.
    let items = store.lock().unwrap().list_items_for_session(&sid).unwrap();
    let dupes = items.iter().filter(|i| i.text == "oldest evicted marker task").count();
    assert_eq!(dupes, 2, "the evicted item was re-added → duplicate on the board");
}

/// 4c — long single-tick transcript exceeding the live window budget. One
/// maybe_extract() covers only its clamped window; the cursor advances by the
/// window, so catching up to the full transcript takes MULTIPLE passes.
/// CHARACTERIZATION: assert multi-pass catch-up (cursor < len after one pass,
/// reaches len after enough passes).
#[tokio::test]
async fn long_transcript_needs_multiple_live_passes_to_catch_up() {
    let store = Store::open_in_memory("dev").unwrap();
    let session = store.start_session(None).unwrap();
    let big = "word ".repeat(4000); // ~20k chars, well over the 2000-token window
    store.append_transcript(&session.id, &big).unwrap();
    let sid = session.id.clone();
    let total_chars = big.chars().count();
    let store = Arc::new(Mutex::new(store));

    // Enough end_turn responses for several passes.
    let responses: Vec<_> = (0..8).map(|_| end_turn("nothing new")).collect();
    let mut live = LiveExtractor::new(Arc::new(MockProvider::new(responses)),
        store.clone(), Arc::new(Mutex::new(Memory::default())), &sid);
    live.min_new_chars = 1;

    live.maybe_extract().await.unwrap();
    assert!(live.cursor() < total_chars, "one pass covers only the clamped window");
    // Drive to catch-up.
    let mut passes = 1;
    while live.cursor() < total_chars && passes < 8 {
        live.maybe_extract().await.unwrap();
        passes += 1;
    }
    assert_eq!(live.cursor(), total_chars, "cursor reaches transcript end after multi-pass catch-up");
    assert!(passes > 1, "a window-exceeding transcript required {passes} passes");
}
```

> **Implementer note on 4b/4c:** the exact eviction/window behavior depends on `LiveExtractor`'s `already_captured_budget_tokens` (default 400 → 1600 chars) and the window-clamp landed in commit `67a6676`. The 4b test's item count (80) and its primary `requests()` assertion are derived from the char arithmetic in the doc comment, so they should hold as written — but **run it and confirm** the oldest marker is genuinely absent from the sent request. If the rendered list happens to fit under budget (e.g. the assembler measures differently than `budget_chars(400)`), raise the item count until the oldest line is truncated and update the arithmetic comment; the *primary* assertion must always be the eviction from `requests()`, not the duplicate count (the duplicate is only the downstream consequence). For 4c, confirm one pass leaves `cursor() < total_chars`; if the window clamp differs, adjust the transcript size. `tool_use`/`end_turn` are inlined above; `NullMemoryStore` comes from `evals::run` (public).

- [ ] **Step 4: Run**

Run: `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo test -p evals`
Expected: all hermetic tests pass. Adjust 4b/4c assertions to observed behavior per the note.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(evals): hermetic runner + carried characterization scenarios (swap gap, restart-dup, multi-pass)"
```

---

### Task 7: Real-API runner (gated example) + gated smoke test + README + verification

**Files:**
- Create: `crates/evals/examples/eval.rs`
- Modify: `crates/evals/tests/grader_hermetic.rs` (add one `#[ignore]` real-API well-formedness test), `README.md`

- [ ] **Step 1: The gated real-API CLI** — `crates/evals/examples/eval.rs`. Mirrors the `walk.rs` / `anthropic_smoke.rs` key handling: `ANTHROPIC_API_KEY` from env, refuse without it, **never print the key**. Runs the whole corpus (or a `--scenario <id>` subset) against `--model` (default `claude-haiku-4-5`), writes the JSON report to `--out <path>` (default stdout) and prints the summary table to stderr.

```rust
//! Gated real-API eval runner. Runs the synthetic corpus through the real
//! pipeline against the Anthropic API and emits a comparable JSON report.
//!
//! ```sh
//! ANTHROPIC_API_KEY=sk-... nix shell nixpkgs#cargo nixpkgs#rustc -c \
//!     cargo run -p evals --example eval -- --model claude-haiku-4-5 --out report.json
//! ```
//! Never prints the key. Opt-in only — no key → clear error, no run.

use std::sync::Arc;

use evals::corpus::load_corpus;
use evals::report::{render_table, SuiteReport};
use evals::run::run_scenario;
use harness::AnthropicProvider;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => { eprintln!("{e}"); std::process::ExitCode::FAILURE }
    }
}

async fn run() -> Result<(), String> {
    // arg parse: --model, --out, --scenario (repeatable), --fixtures <dir>
    let mut model = "claude-haiku-4-5".to_string();
    let mut out: Option<String> = None;
    let mut only: Vec<String> = Vec::new();
    let mut fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures").to_string();
    let mut argv = std::env::args().skip(1);
    while let Some(a) = argv.next() {
        match a.as_str() {
            "--model" => model = argv.next().ok_or("--model needs a value")?,
            "--out" => out = Some(argv.next().ok_or("--out needs a path")?),
            "--scenario" => only.push(argv.next().ok_or("--scenario needs an id")?),
            "--fixtures" => fixtures = argv.next().ok_or("--fixtures needs a dir")?,
            "-h" | "--help" => return Err("usage: eval [--model M] [--out report.json] [--scenario id]... [--fixtures dir]".into()),
            other => return Err(format!("unexpected arg: {other}")),
        }
    }

    let api_key = std::env::var("ANTHROPIC_API_KEY").ok()
        .filter(|k| !k.trim().is_empty())
        .ok_or("ANTHROPIC_API_KEY is not set — export it to run the real-API eval (key is never printed)")?;

    let mut corpus = load_corpus(&fixtures).map_err(|e| format!("cannot load corpus: {e}"))?;
    if !only.is_empty() {
        corpus.retain(|s| only.contains(&s.id));
        if corpus.is_empty() { return Err("no scenarios matched --scenario".into()); }
    }

    let provider = Arc::new(AnthropicProvider::new(api_key, &model));
    let mut reports = Vec::new();
    for scenario in &corpus {
        eprintln!("running {} ...", scenario.id);
        let report = run_scenario(scenario, provider.clone(), &model).await
            .map_err(|e| format!("{}: {e}", scenario.id))?;
        reports.push(report);
    }
    let suite = SuiteReport::assemble(&model, reports);

    let json = serde_json::to_string_pretty(&suite).map_err(|e| e.to_string())?;
    match &out {
        Some(path) => std::fs::write(path, &json).map_err(|e| format!("cannot write {path}: {e}"))?,
        None => println!("{json}"),
    }
    eprintln!("\n{}", render_table(&suite));
    Ok(())
}
```

- [ ] **Step 2: Gated real-API well-formedness test** (append to `grader_hermetic.rs`, `#[ignore]`d)

```rust
/// Gated real-API run of ONE scenario. Asserts the report is well-formed — never
/// asserts score thresholds (real scores are non-deterministic). Costs tokens.
#[tokio::test]
#[ignore = "hits the real Anthropic API; set ANTHROPIC_API_KEY and run with --ignored"]
async fn real_api_eval_is_well_formed() {
    let api_key = std::env::var("ANTHROPIC_API_KEY").expect("set ANTHROPIC_API_KEY");
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");
    let deck = evals::corpus::load_corpus(dir).unwrap().into_iter()
        .find(|s| s.id == "deck_walk_contacts").unwrap();
    let provider = std::sync::Arc::new(harness::AnthropicProvider::new(api_key, "claude-haiku-4-5"));
    let report = evals::run::run_scenario(&deck, provider, "claude-haiku-4-5").await.unwrap();
    // well-formed: metrics in range, cost recorded (R9)
    assert!((0.0..=1.0).contains(&report.score.f_half));
    assert!((0.0..=1.0).contains(&report.score.distractor_fp_rate));
    assert!(report.cost.input_tokens > 0, "R9: usage was logged and priced");
}
```

- [ ] **Step 3: README** — **append** `, 05b eval suite` to the existing itemized `Done:` line (do NOT rewrite/compress it — keep every prior item), and add the eval run instructions below it:
```markdown
Done: 01 foundation, 02 memory + reflection + context assembler, 03 domain + storage, 04 processing pipeline + reflection coordinator, 05 live extraction, 05b eval suite.
Next: 06.

Evals: `cargo test -p evals` (hermetic, no key). Real-API:
`ANTHROPIC_API_KEY=sk-... cargo run -p evals --example eval -- --out report.json`.
```
(The `Done:` line above reproduces Plan 05's final line verbatim with the new item appended; match whatever the README actually says at build time — the rule is *append*, not *replace*.)

- [ ] **Step 4: Full verification**

Run: `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo test --workspace` → all pass, fast, no key needed.
Run: `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo clippy --all-targets` → zero warnings (fix mechanically, no `#[allow]`; STOP and report if a fix would change behavior).
Confirm: `cargo test --workspace` runs **zero** network calls (the only real-API test is `#[ignore]`d; the example is not a test).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(evals): gated real-API runner CLI + well-formedness smoke; docs: plan 05b done"
```

---

## Deferred (named, for later plans)

1. **The prompt-optimization loop itself (DSPy/GEPA).** This plan builds the *measurement* foundation — a comparable scalar (mean F0.5) + full metric vector + JSON report. The optimizer that mutates `extraction_system_prompt` / `live_extraction_system_prompt`, re-runs the suite, and keeps winners is a separate plan. The seam it needs is already here: `run_scenario` takes any provider; a variant loop would parameterize the *prompt* (requires a small `murmur-core` seam to inject a prompt override — noted for that plan, deliberately NOT added here to keep zero-impact).
2. **LLM-judge grading layer.** The deterministic Dice grader stands alone (hard requirement). An optional LLM judge — "is this produced item a faithful capture of a spoken intent?" — could catch semantic matches Dice misses (paraphrase with zero token overlap) and semantic false-positives Dice waves through. Proposed shape: a second gated pass that scores only the *disagreements* (unmatched pairs) to bound cost, reported as a separate `judge_precision`/`judge_recall` alongside the deterministic scores, never replacing them. Deferred: adds real-API cost and non-determinism; the deterministic grader is enough to start optimizing.
3. **Fixing the carried gaps (4a swap, 4b dedup).** This plan *characterizes* them (pins current behavior with tests). The fixes — defer `clear_session_outputs` until processing succeeds (4a); persist or widen the live dedup set across restart (4b) — belong to Plan 06. The characterization tests become the regression baseline those fixes must flip.
4. **Confidence-interval / multi-sample scoring.** Real-API scores vary run-to-run. A rigorous optimizer would run each scenario N times and report mean±stddev. Deferred: single-sample is enough for coarse variant ranking; add repetition when the optimizer needs to distinguish close variants.
5. **Live-board eval (grading `LiveExtractor` output directly).** This suite grades end-of-session `process()` output (the source of truth). Grading the *live* board mid-recording — precision/recall of the incremental passes against a "should-be-captured-by-now" ground truth — is a distinct eval with its own (harder) ground-truth authoring. The carried 4b/4c tests touch live behavior but characterize mechanics, not extraction quality. Deferred until live quality is a tuning target.
6. **Price table freshness.** `report::price_per_mtok` is a hardcoded constant, flagged approximate. A later plan could pull live pricing or read it from config. Token counts are always exact (R9 log); only the $ column is a constant.

## Self-Review Notes

- **Requirement coverage:** (1) synthetic corpus w/ trades vocab, disfluency, contacts, safety, decisions, measurements, varied lengths, fixture files + typed ground truth + distractors ✓ (Task 5, three seeds inline + recipe to 8–12). (2) deterministic grader, normalized-token Dice ≥0.5, hermetic, machine-comparable JSON + table ✓ (Tasks 2–4); optional LLM judge proposed but deferred, deterministic stands alone ✓ (Deferred 2). (3) runner: gated real-API (Task 7, `anthropic_smoke` key pattern) + hermetic MockProvider mode (Task 6); per-run cost from R9 log ✓ (`observe`). (4) carried scenarios 4a–d all present ✓ (4a/4b/4c Task 6 characterization tests, 4d empty fixture Task 5). (5) metrics: per-kind + overall P/R/F1, kind-confusion, contact accuracy, R6 distractor-FP, summary presence, cost tokens+$ ✓; precision-weighted via F0.5, documented ✓ (Task 3/4). (6) placement: new `crates/evals` dev crate, justified, zero shipping impact, no migration ✓ (Task 1). (7) TDD tasks w/ file paths, code sketches, test names, `feat(evals):` commits, Deferred + Self-Review ✓; hermetic-by-default, `cargo test --workspace` green without key ✓ (Task 7 Step 4). (8) 3 seed transcripts inline in the plan + extension recipe to 8–12 ✓ (Task 5).
- **API consistency:** every referenced symbol exists in the read source — `Store::{open_in_memory, start_session, append_transcript, end_and_record_session, add_item, list_items_for_session, list_contacts, list_llm_usage_for_session, usage_totals, get_session}`, `SessionProcessor::{new, process}`, `LiveExtractor::{new, min_new_chars, maybe_extract, cursor}`, `LiveExtractOutcome`, `CapturedItem{kind,text}`, `Contact{name,trade}`, `LlmUsageRow{input_tokens,output_tokens,purpose}`, `SessionStatus::Failed`, `MockProvider::new`, `AnthropicProvider::new(key, model)`, `MemoryStore`, `Memory::default`, `Usage{input_tokens,output_tokens}` (u64), `CoreError::InvalidState`. `evals` deps: `murmur-core`, `harness`, `serde`, `serde_json`, dev `tokio` — all workspace deps, no new crates.
- **Judgment calls (resolved by me, flag for reviewers):** (a) **Dice ≥0.5** over substring/exact — symmetric + order-independent + STT-tolerant; threshold fixed to keep scores comparable. (b) **F0.5 headline** (β²=0.25) — the cleanest encoding of R6's precision bias into one comparable scalar; full vector preserved so no axis is hidden. (c) **new dev crate** over `core/tests/` — different lifecycle, keeps core test loop fast, natural home for the runner binary; `--workspace` still covers it. (d) **paired `.txt`+`.json` fixtures** — transcripts read naturally (reviewable ground truth), truth stays typed; no glob dep (`read_dir`). (e) **distractors as explicit ground truth** — R6 needs a denominator; measuring "false positives on things that should NOT be items" requires naming them. (f) **`(empty session)` placeholder treated as no-summary** in `observe` — keeps the empty-scenario grade honest. (g) **greedy not Hungarian** matching — deterministic, dependency-free, ~optimal at <30 items/session. (h) carried gaps are **characterization, not fixes** (explicit in the plan and Deferred 3) — the ask was to pin/quantify, and the fixes belong to Plan 06.
- **Open questions resolved by judgment:** (1) *Where does `distractor_fp_rate` denominator come from when a scenario has no distractors?* → rate is 0.0 (no chatter to resist); excluded from R6 pressure, and the recipe requires ≥2 distractors on non-punch-list scenarios so the metric has teeth where it matters. (2) *Should the suite hard-fail on low scores?* → No — real scores are non-deterministic; the real-API test asserts *well-formedness* only, and threshold gating (if any) is the optimizer's job. (3) *Does grading `process()` output miss live-extraction quality?* → Yes by design; process() is the source of truth (Plan 05), live-board eval is Deferred 5. (4) *4b/4c exact numbers depend on the window/dedup constants landed in `67a6676`* → the plan instructs implementers to run-then-characterize rather than assert guessed constants, since I did not read `live.rs`'s post-`67a6676` window-clamp internals in full.
- **Test-count checkpoint:** T1 +4 (corpus), T2 +6 (normalize), T3 +8 (grader), T4 +4 (report), T6 +2 hermetic +3 carried, T7 +1 gated ≈ **28 new** (27 hermetic + 1 `#[ignore]`). Existing suite 179 → expect ~206 non-ignored. Counts are expectations, not gates.
- **Risks:** (i) The perfect-model hermetic test assumes MockProvider replays the scripted extraction verbatim through the agent loop — if the agent adds turns, the script must supply enough `end_turn`s (handled: `perfect_script` ends with `end_turn` + summary). (ii) Contact grading uses global `list_contacts()` (no session filter) — safe only because each scenario gets a fresh in-memory store; the plan enforces one store per scenario. (iii) Dice threshold 0.5 may reject a genuine match with heavy STT garble — that's a *measured* recall miss, exactly what the eval should surface, not a bug. (iv) `strip_plural` is deliberately crude (English, trailing single `s`); trade vocabulary is English so acceptable, documented.
