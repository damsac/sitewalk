# Murmur — the story so far

One repository, two eras. Murmur began as a SwiftUI iOS app and was reborn on
2026-07-01 as a field-work voice agent built on a Rust core. This document is the
narrative arc; the git history is the primary source. Where a section says "browse
in history," the commits are really there — nothing has been squashed away.

---

## Era I — Murmur, the Swift app (≈2026-02 → 2026-07-01)

**What it was.** Speak into your phone, get organized entries. An autonomous
second brain: voice → transcription → an LLM agent that captured, categorized,
surfaced, and acted on your entries (todos, notes, reminders, ideas, lists,
habits, questions). Not a transcription app — a thinking partner that managed
mental load.

**Shape of the codebase.**

- **`Murmur/`** — SwiftUI iOS app, generated from `project.yml` via XcodeGen.
- **`Packages/MurmurCore/`** — Swift package: transcription pipeline, LLM
  service (PPQ.ai), data models.

**Landmarks (all browsable in git history):**

- **Agent pipeline** — voice → `AppleSpeechTranscriber` → `Pipeline.processWithAgent`
  → `PPQLLMService` → `AgentActionExecutor` → conversation thread. Multi-turn,
  real tool-result feedback, resilient per-tool-call error isolation,
  file-based agent memory (`update_memory` tool).
- **Composition system** — unified `HomeComposition` model driving two home
  variants (Scanner: urgency-grouped; Navigator: category-grouped), LLM-composed
  via a forced `compose_view` tool, with a layout-diff refresh path.
- **Categories as behavior** — a category earns its existence only by driving
  different behavior (7 categories; `thought` was removed for having none).
- **Shipping infrastructure** — TestFlight release pipeline, CI signing,
  credit system, Studio analytics SDK.
- **~151 merged PRs** of product and platform work.

**How to browse Era I.** The Swift app tree is preserved in history. After the
Phase-2 pivot commit (below) removes it from the working tree, you can still read
any of it:

```
# The last commit where the Swift app was live in the tree:
git log --oneline --all -- Murmur/ Packages/MurmurCore/ | head -1
# Browse the app as it stood then:
git show <that-sha>:Murmur/MurmurApp.swift
git ls-tree -r <that-sha> -- Murmur/
```

Design specs, plans, and reviews from this era live under `docs/` (e.g.
`docs/specs/`, `docs/plans/`, `docs/reviews/`, and the pre-pivot
`docs/superpowers/specs/` design docs).

---

## Era transition — the 2026-07-01 pivot

Murmur pivoted from a general-purpose second brain to **AI meeting notes for
blue-collar field work**: general contractors walking a site, inspectors, trades
— speak through the walk, get a structured document (site walk → landscape /
property / inspection report).

The pivot is documented, not just decided:

- **Vision spec** (4 revisions): `docs/superpowers/specs/2026-07-01-murmur-rebuild-vision-design.md`
- **UI mocks**: `docs/superpowers/mocks/2026-07-01-rebuild-ui/`
- **Canon** (rebuild decisions, agreement noted per row): `meta/CANON.md`,
  section "Rebuild Decisions (2026-07 pivot)".

Why rebuild rather than refactor: the product's atomic unit changed (entry →
site-walk session/job with items and documents), the architecture changed (Rust
core + native shells over a Swift-only app), and the constraints changed
(local-first, audio never leaves device, vocabulary → STT biasing loop). The
Swift codebase was superseded but kept as reference — hence this single repo.

Key architecture decisions from the pivot (see `meta/CANON.md`):

- **Rust workspace** — `harness` → `murmur-core` → `stt` → `ffi` (UniFFI) —
  with native SwiftUI / Compose shells. Native chosen over Tauri for background
  audio and feel.
- **Local-first** — SQLite, UUIDv7, tombstones, single-writer store; sync-ready
  but no sync/accounts in v1.
- **Product rules R1–R9** — hidden transcript, deliberate stop, under-extraction
  bias (R6), spend meter (R9), and more (spec §3).

---

## Era II — the Rust rebuild (2026-07-01 → present)

Built in the open at **`damsac/sitewalk`** (the working title graduated to a repo
name; the brand stays **Murmur**). The rebuild proceeded as a numbered plan
series, each plan a chapter:

| Plan | What | Status |
|------|------|--------|
| 01 | `crates/harness` — agent loop, tools, Anthropic + mock providers | done (14 commits) |
| 02 | memory + reflection + context assembler (provenance, snapshots, importance-aware forgetting) | done (15 commits) |
| 03 | `crates/murmur-core` — SQLite store, jobs/sessions/items/contacts, tombstones, sync-ready | done (14 commits) |
| 04 | processing pipeline (two-phase extract + summary), reflection coordinator, R9 cost log | done (16 commits) |
| 05 | live in-session extraction (`LiveExtractor`) | done (6 commits) |
| 05b | `crates/evals` — synthetic corpus + deterministic grader (F0.5, R6-weighted) | done (8 commits) |
| 06-spike | STT benchmark: whisper-rs feasibility / RTF / biasing, GO-KILL exit criteria | **GO** |
| 06 | STT for real (+ items `source` column, swap-contract fix) | next |
| 07 | layout protocol + FFI (UniFFI) — where the iOS `WalkEngine` bridge lands | queued |

**iOS shell** — sac built the native SwiftUI app (design system + full flow)
behind a `WalkEngine` protocol seam at the FFI boundary (`AppModel.init(engine:)`
is the swap point), against a `DemoWalkEngine` until Plan 07 wires the real core.
Landed as sitewalk **PR #1**; follow-ups tracked in sitewalk **issue #2**.

**Research that shaped it** — memory frontier research (snapshots, provenance,
importance-aware forgetting) and STT frontier research (engine survey, iOS 26
SpeechAnalyzer dropping custom-vocabulary biasing, benchmark plan) live under
`docs/research/`.

**How to browse Era II.** Until the Phase-2 merge, the code lives at
`github.com/damsac/sitewalk`. After the unrelated-histories merge, sitewalk's full
commit history is grafted into this repo and browsable here:

```
# Rebuild history is carried over intact (not copy-imported):
git log --oneline 86c16792f8aa602bc005812a3c78591221e65447^2   # 138 rebuild commits
# The Rust workspace sits at the repo root after migration.
```

---

## Era III — the 2026-07-04 re-unification

The rebuild was living in a separate repo (`damsac/sitewalk`). We decided to fold
it **back into `damsac/Murmur`** so one repository tells the whole arc — Swift era
→ pivot → Rust rebuild — with no history lost.

**Phase 1** (done): land the pivot chapter on Murmur `main` — vision spec, UI
mocks, plan series 01–07, memory/STT research, rebuild-era meta. Merged as a
merge commit (branch history preserved) in **PR #152**.

**Phase 2** (gated on in-flight rebuild work landing): see
`docs/reunify/RUNBOOK.md`. In order:

1. An explicit **pivot commit** removes the Swift app (`Murmur/`,
   `Packages/MurmurCore/`) from the working tree — preserved in history only.
   Conceptually: *"pivot: retire the Swift app from the tree (Era I preserved in
   history)."* Its SHA gets recorded here in Phase 2:
   `7cc1c2428d6e8aa76da2b38cf4d28624bee56fa2`.
2. sitewalk `main` is fetched as a remote and merged with
   `git merge --allow-unrelated-histories` — never a copy-import — so Era II's
   history is carried over intact. The merge commit's SHA:
   `86c16792f8aa602bc005812a3c78591221e65447`.
3. The Rust workspace ends up at the repo root; `README.md` is replaced (draft at
   `docs/reunify/README.next.md`); sitewalk issue #2 is migrated here.
4. `damsac/sitewalk` gets a pointer README and is archived (`gh repo archive`) —
   history preserved and browsable, code frozen.

**Browsing map after re-unification:**

| To see… | Do… |
|---------|-----|
| The Swift app as it lived | `git show 7cc1c2428d6e8aa76da2b38cf4d28624bee56fa2^:Murmur/…` (parent of the pivot commit) |
| The pivot decision | `docs/superpowers/specs/2026-07-01-murmur-rebuild-vision-design.md` + `meta/CANON.md` |
| The Rust rebuild's chapter-by-chapter history | `git log 86c16792f8aa602bc005812a3c78591221e65447^2` |
| The current product & workspace | repo root + `README.md` |

*The pivot and unrelated-histories merge commits above landed in Phase 2
(2026-07-05); the SHAs are final.*
