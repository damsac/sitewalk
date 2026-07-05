# Roadmap

Shared priorities and sequencing. Who's doing what, what's next, what's blocked.

Updated when priorities shift. Either person can propose changes via PR.

---

## Active

| Work | Owner | Status |
|------|-------|--------|
| Real-mic device voice walk (`live=1`) + on-device tuning: voiceproc A/B, vad_rms ~0.01, quiet-flush validation (final-review notes A/B) | dam | Plan 08 FULLY merged — device session is the gate |
| Issue #155 — PR #1 review follow-ups (4 state bugs + seam hygiene) | sac | open (several now also guarded core-side by 07-carry) |
| Rebuild-era nix-based CI (cargo test needs nix deps — naive runner job goes red) | dam | follow-up from #157 |

## Up Next (sequenced)

1. **Plan 08 Part C** — noise robustness (see Active).
2. **Rebuild-era TestFlight pipeline** — release.yml is Era-I, manual-only; a real apps/ios pipeline is required before the next external build.
3. **Accuracy hardening** (Plan 09): thread 1 (word-level timestamps) **landed** — `token_timestamps` → per-word timing → word-anchored coarse-seam drop, degrading to segment-coarse when absent/mismatched; thread 2 (live-prompt pins) **landed as scaffolding** — golden assembled-prompt snapshot + grader-over-live-board, hermetic. The real-API live-grading extension (non-circular F0.5 movement) is **flagged/deferred** to the optimization loop (item 4). The SNR sweep rerun (`--token-timestamps`, WER/RTF delta + the `word_timestamps: true` default verdict) is **device-gated for dam** (Task 7).
4. **Prompt-optimization loop** on the 05b eval suite (rank on F0.5, gate on recall).
5. **Photo attachment schema** (rides a migration after `source`).

### Vocabulary → STT biasing loop (Plan 10)

**Write half LANDED** — the differentiator's data path is now closable end-to-end. A vocabulary management surface on `harness::Memory` (`VOCABULARY_SECTION`/`MAX_VOCABULARY_TERMS`(100)/`MAX_VOCABULARY_TERM_WORDS`(6) constants, `VocabAdd`, symmetric-normalized case-insensitive dedup, write-time reject-when-full cap, a `Stated` provenance floor so user terms outlive `Inferred` ones under cap pressure); FFI CRUD on `MurmurEngine` (`list`/`add`/`remove_vocabulary_term`, throwing/panic-free, lock-then-save, `EngineError::Memory`); a functional-plain iOS editor wired through `WalkEngine` (**visuals are sac's** — `// sac:` handoff markers throughout); and a hermetic e2e proving add-via-FFI → `collect_bias_terms` → `build_bias_prompt`. Reflection carries one preserve-vocabulary prompt sentence (no new machinery). Real recall-lift on device is spike-harness-measured, **flagged for dam** (not CI). Plan: `docs/plans/2026-07-05-rust-core-10-vocabulary-loop.md`.

**Still open:** the **onboarding interview** that SEEDS vocabulary (D9, joint dam+sac) — the `add_vocabulary_term` path is ready to receive its output; **auto-harvest** of proper nouns from live extraction (D9 seam — the `source` param takes `Inferred`, detection not built); a **protected-vocabulary tier** (D3, dam) — v1 ships the `Stated` floor + reflection prompt line and measures on device before escalating (`Corrected` overload vs. a new `Pinned` rank vs. vocabulary-aware `prune_stale`).

## Done 2026-07-05 (the big day)

Re-unification complete (repo = **damsac/sitewalk**, one history, Swift Era I preserved; archive = sitewalk-archive) · issue/PR slate cleaned (19+2 Swift-era closed; #155/#156 remain) · CLAUDE.md + CI rewritten for the rebuild (#157) · **first real walk** (EST-0047, real core + key on sim) · **Plan 08 Parts A+B merged**: mic→whisper→append wiring, cancel() (closes #156's core half), transcript events, use_gpu knob (sim=CPU — D7's "Metal degrades on sim" was falsified: it SIGTRAPs), voice-walk-from-WAV proven end-to-end on sim (whisper decoded the fixture; transcript verified in SQLite). 290 tests.

## Decisions needed (joint)

- Template keys: adopt `landscape | property | inspection` as canonical? (dam: yes — needs sac's ack)
- STT DONE semantics: flush final utterance vs speed
- Fate of the Gallery/Screens static twins after design freeze

## Completed (rebuild era)

| Work | Date | Where |
|------|------|-------|
| Vision spec (4 revs) + UI mocks + user stories | 2026-07-01 | `damsac/Murmur` `pr/dam/rebuild-vision` |
| Plan 01 — harness foundation (agent loop, tools, providers) | 2026-07-01 | this repo, 14 commits |
| Plan 02 — memory/reflection/context (provenance, snapshots) | 2026-07-02 | this repo, 15 commits |
| Plan 03 — domain + SQLite store (tombstones, sync-ready) | 2026-07-02 | this repo, 14 commits |
| Plan 04 — processing pipeline + reflection coordinator + R9 cost log | 2026-07-03 | this repo, 16 commits |
| Plan 05 — live in-session extraction | 2026-07-03/04 | this repo, 6 commits |
| Plan 05b — eval suite (corpus + deterministic grader + runners) | 2026-07-04 | this repo, 8 commits |
| Memory frontier research / STT frontier research | 2026-07-02/04 | `damsac/Murmur` `docs/research/` |
| Repo → damsac org, public | 2026-07-04 | github.com/damsac/sitewalk |
| iOS app: design system + full flow behind WalkEngine seam | 2026-07-04 | PR #1 (sac), merged |
