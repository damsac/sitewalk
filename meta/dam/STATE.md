# Dam's State

What dam is working on right now. Updated with every PR.

---

## Current focus

**The rebuild — this repo.** Murmur pivoted 2026-07-01 to AI meeting notes for blue-collar field work (GC site walks, inspections). Rust core + native shells, built here in `damsac/sitewalk`. Specs, plans, research, and mocks all live HERE now (`docs/`); `damsac/Murmur` is archive-only.

dam owns: harness / murmur-core / STT / FFI. sac owns: renderers / component library / visual direction (`apps/ios/`).

## Where the core is (main @ baa8848, 277 tests, clippy clean)

| Plan | What | Status |
|------|------|--------|
| 01 | `crates/harness` — agent loop, tools, Anthropic provider, mock provider | done |
| 02 | memory + reflection + context assembler (provenance, snapshots, forgetting) | done |
| 03 | `crates/murmur-core` — SQLite store, jobs/sessions/items/contacts, tombstones, sync-ready | done |
| 04 | processing pipeline (two-phase extract+summary), reflection coordinator, R9 cost log | done |
| 05 | live in-session extraction (`LiveExtractor`) — incremental passes onto the live board | done |
| 05b | `crates/evals` — synthetic site-walk corpus + deterministic grader (F0.5, R6-weighted) | done |
| 06-spike | STT benchmark — verdict **GO** (RTF ≪0.5 all models, +10-19pp biasing lift, append-only proven) | done; iPhone T5 tier pending (dam, ~1hr) |
| 06a | items `source` column + atomic swap-at-finish; failed process PRESERVES live board | done |
| 06 | `crates/stt` — whisper-rs feature-gated, chunked streaming, time-anchored dedup finalizer, initial_prompt biasing | done |
| 07 | `crates/ffi` (UniFFI) + `MurmurEngine.swift` — **the bridge is LIVE**: app builds with the real core linked | done |
| 07-carry | all 6 carry notes + 3 cross-model findings: fallible constructors, atomic begin_walk, mint-with-artifact-write, throwing WalkEngine.begin (dead walk never starts), tick fault counter, narrowed artifact sweep | done (merged be88bca) |
| first walk | **THE MILESTONE LANDED 2026-07-05**: real core + .env key on sim → document EST-0047 end-to-end. Clean checkout builds demo with zero setup; `generate.sh` opts into real | done (merged baa8848) |
| 08 A+B | STT stage-2 wiring: push_audio → pump thread → append path; TranscriptCommitted/Preview events; finish() flush + async cancel() (Store::delete_session — closes the #156 core half); AudioCaptureSource (mic→16kHz) + WavFileAudioSource; use_gpu knob (sim=CPU compile-time — D7 "Metal degrades on sim" FALSIFIED: SIGTRAP; device=Metal) | **done, merged 2026-07-05** — 290 tests; voice-from-WAV proven end-to-end on sim |
| 08 Part C | noise robustness: voiceproc A/B knob, dual R3 gates (complementary, proven), SNR sweep: base.en bundled | done, merged 2026-07-05 |
| 09 | word-level whisper timestamps → mode-aware finalizer seam (`time_precise`: precise drops by start, coarse keeps end) — resolves the Plan 06 coarse-seam CAVEAT; eval live-prompt golden pins | **done, merged 2026-07-05** (#162/#163) |
| 10 | vocabulary loop write half: editor → FFI CRUD → Memory vocab surface (normalize funnel, caps 100/6 at write) → whisper bias. **The differentiator is live e2e** | **done, merged 2026-07-05** (#164/#165) — 322 tests |
| infra | CI live (#160: nix Rust gates + iOS demo, every PR), threshold knobs launch-arg tunable (#161), **TestFlight rebuild pipeline built** (branch `pr/dam/testflight-rebuild`, UNMERGED on purpose — merging arms auto-publish on every main push; dry-run blocked on the Apple agreement signature) | 2026-07-05 |

## Where we are (2026-07-05, post re-unification)

**One repo: github.com/damsac/sitewalk** (renamed from damsac/Murmur; the whole story in one history — `docs/HISTORY.md`). Rebuild-era clone `~/murmur-rmp` retired. Codex cross-model review is standard on state-machine diffs — use the `codex` skill wrapper, not raw CLI (13/13 verified findings across 4 uses). **Voice walk on sim works** (wavwalk=1 → whisper decode → board → document). **dam's device session now carries 5 items**: real-mic `live=1` walk · T5 RTF/battery · voiceproc A/B + `sttvad≈0.01` sweep · Plan 09 Task 7 sweep rerun · vocabulary-biasing spot check (add your jargon in the new editor, walk, compare). Before that: sign the Apple agreement (developer.apple.com — blocks the TestFlight dry-run), and coordinate TestFlight merge timing with sac (his TestFlight app becomes the rebuild). Model provisioning: 60MB ggml file goes in `apps/ios/Sources/Resources/` (generate.sh prints the curl if missing). Regenerated bindings mean: re-run `build-ffi.sh` once after pulling.

## What sac should know

- **PR #1 is merged** (main); review conditions carried as **issue #2** — four state-transition bugs + three seam-hygiene items.
- **STT is Rust-side — decided.** The spike GO'd whisper-rs (iOS 26's SpeechAnalyzer dropped custom vocabulary, and our biasing loop needs it). `crates/stt` is built; mic wiring (stage 2) is the next plan. Your `append(transcript:)` path works today.
- **The real bridge is ACTIVE**: `MurmurEngine.swift` compiles against `MurmurCoreFFI`. Run `apps/ios/build-ffi.sh` once on your machine to produce the gitignored xcframework, then `xcodegen generate` — demo engine still runs by default; a configured key switches to the real core.
- **Small change in your file**: `CapturedFixture.id` gained an explicit init (default `UUID()`) so core-assigned ids stay stable across `boardUpdated` snapshots — additive, no call-site changes.
- **finish() edge behavior (new)**: silent walk returns a truthful empty document (queued=false, doc_number=0); double-finish returns the cached document — both no-panic by contract.
- **HANDOFF answers**: events batched per live pass; core mints document numbers; photos need a schema migration (queued); template keys `landscape | property | inspection` proposed as canonical — needs your ack (CANON).
- **Bridge realities**: `finish()` = `end_and_record_session` + `process()` — two-phase, budgeted <8s; live items get tombstoned and re-extracted at process time (the board "swaps" — UI should anticipate); `LiveExtractor.maybe_extract` is `&mut self`, the FFI wrapper serializes it.

## What I need from sac

- Work through issue #2 (or push back per item — it's a conversation).
- The two harness patches on your machine (PPQ Bearer auth + `ANTHROPIC_BASE_URL`) as a proper PR with tests.
- Two CANON acks: template keys; STT DONE semantics (flush vs speed).
- Formal review of the vision spec (`damsac/Murmur` → `pr/dam/rebuild-vision` → `docs/superpowers/specs/`).

## Open questions

- STT engine: whisper-rs Rust-side vs staged hybrid — the 06-spike benchmark decides (dam's preference: Rust-side if the numbers hold).
- Who runs the 06-spike: builder agent or dam's hands.
