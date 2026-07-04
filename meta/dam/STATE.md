# Dam's State

What dam is working on right now. Updated with every PR.

---

## Current focus

**The rebuild.** Murmur pivoted 2026-07-01 to AI meeting notes for blue-collar field work (GC site walks, inspections). Ground-up rewrite: Rust core + native shells. This repo (`damsac/Murmur`) is now the docs home — specs, plans, research live on `pr/dam/rebuild-vision`. Code lives at **`damsac/sitewalk`** (public).

dam owns: harness / murmur-core / STT / FFI. sac owns: renderers / component library / visual direction.

## Where the core is (sitewalk main @ 7cf4bdb, 214 tests, clippy clean)

| Plan | What | Status |
|------|------|--------|
| 01 | `crates/harness` — agent loop, tools, Anthropic provider, mock provider | done |
| 02 | memory + reflection + context assembler (provenance, snapshots, forgetting) | done |
| 03 | `crates/murmur-core` — SQLite store, jobs/sessions/items/contacts, tombstones, sync-ready | done |
| 04 | processing pipeline (two-phase extract+summary), reflection coordinator, R9 cost log | done |
| 05 | live in-session extraction (`LiveExtractor`) — incremental passes onto the live board | done |
| 05b | `crates/evals` — synthetic site-walk corpus + deterministic grader (F0.5, R6-weighted) | done |
| 06-spike | STT benchmark: whisper-rs feasibility/RTF/biasing, GO-KILL exit criteria | plan ready, not run |
| 06 | STT for real (+ items `source` column, swap-contract fix) | blocked on spike |
| 07 | layout protocol + FFI (UniFFI) — **this is where sac's WalkEngine bridge lands** | queued |

## What sac should know

- **Your PR #1 is reviewed** (comment on the PR): merges after small fixes. Key asks: fixture-type honesty (they're the interim domain model, incl. in the WalkEngine signature), a `TranscriptSource` injection seam, four verified state-transition bugs (DISCARD marks SENT, share-cancel finalizes, discard-while-paused spins, `recognition.cancel()` drops the final phrase).
- **STT may move Rust-side.** iOS 26's SpeechAnalyzer dropped custom-vocabulary biasing, which our vocabulary→STT loop needs. The 06-spike benchmark decides. Your seam survives either way (`append(transcript:)` takes text).
- **Answers to your HANDOFF questions**: events batched per live pass; core mints document numbers; photos need a schema migration (queued); your `landscape | property | inspection` template keys — proposed as canonical, say yes and it's locked.
- **The bridge contract's core-side realities**: `finish()` = `end_and_record_session` + `process()` — two-phase, budgeted <8s; live items get tombstoned and re-extracted at process time (the board will "swap" — UI should anticipate); `LiveExtractor.maybe_extract` is `&mut self`, the FFI wrapper serializes it.

## What I need from sac

- Apply the PR #1 review conditions (or push back — it's a conversation).
- The two harness patches on your machine (PPQ Bearer auth + `ANTHROPIC_BASE_URL`) as a proper PR with tests.
- Template-keys ack (see above) before the artifact seam ossifies.
- Read the vision spec + STT research on `pr/dam/rebuild-vision` — the branch is finally pushed; a formal spec review from you is still owed.

## Open questions

- STT engine: whisper-rs Rust-side vs staged hybrid — the 06-spike benchmark decides (dam's preference: Rust-side if the numbers hold).
- STT finish semantics on DONE (flush final phrase vs speed) — old canon says speed, site-walk reality says flush. Joint call with sac, flagged in PR #1 review.

---

*Pre-pivot state (TestFlight era) archived in git history of this file.*
