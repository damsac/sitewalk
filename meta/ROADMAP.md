# Roadmap

Shared priorities and sequencing. Who's doing what, what's next, what's blocked.

Updated when priorities shift. Either person can propose changes via PR.

---

## Active

| Work | Owner | Status |
|------|-------|--------|
| Real-mic device voice walk (`live=1`) + on-device tuning: voiceproc A/B, vad_rms ~0.01, quiet-flush validation (final-review notes A/B) | dam | Plan 08 FULLY merged — device session is the gate |
| Issue #155 — PR #1 review follow-ups (4 state bugs + seam hygiene) | sac | open (several now also guarded core-side by 07-carry) |

## Up Next (sequenced)

1. **TestFlight first publish** — pipeline BUILT (`pr/dam/testflight-rebuild`, unmerged: merging arms auto-publish on every main push). Sequence: dam signs the Apple agreement → dry-run (workflow_dispatch upload=false) → coordinate with sac → merge. CI itself landed (#160, every PR gated).
2. **Accuracy hardening** (Plan 09): thread 1 (word-level timestamps) **landed** — `token_timestamps` → per-word timing → word-anchored coarse-seam drop, degrading to segment-coarse when absent/mismatched; thread 2 (live-prompt pins) **landed as scaffolding** — golden assembled-prompt snapshot + grader-over-live-board, hermetic. The real-API live-grading extension (non-circular F0.5 movement) is **flagged/deferred** to the optimization loop (item 4). The SNR sweep rerun (`--token-timestamps`, WER/RTF delta + the `word_timestamps: true` default verdict) is **device-gated for dam** (Task 7).
3. **Prompt-optimization loop** on the 05b eval suite (rank on F0.5, gate on recall).

### Vocabulary → STT biasing loop (Plan 10)

**Write half LANDED** — the differentiator's data path is now closable end-to-end. A vocabulary management surface on `harness::Memory` (`VOCABULARY_SECTION`/`MAX_VOCABULARY_TERMS`(100)/`MAX_VOCABULARY_TERM_WORDS`(6) constants, `VocabAdd`, symmetric-normalized case-insensitive dedup, write-time reject-when-full cap, a `Stated` provenance floor so user terms outlive `Inferred` ones under cap pressure); FFI CRUD on `MurmurEngine` (`list`/`add`/`remove_vocabulary_term`, throwing/panic-free, lock-then-save, `EngineError::Memory`); a functional-plain iOS editor wired through `WalkEngine` (**visuals are sac's** — `// sac:` handoff markers throughout); and a hermetic e2e proving add-via-FFI → `collect_bias_terms` → `build_bias_prompt`. Reflection carries one preserve-vocabulary prompt sentence (no new machinery). Real recall-lift on device is spike-harness-measured, **flagged for dam** (not CI). Plan: `docs/plans/2026-07-05-rust-core-10-vocabulary-loop.md`.

**Still open:** the **onboarding interview** that SEEDS vocabulary (D9, joint dam+sac) — the `add_vocabulary_term` path is ready to receive its output; **auto-harvest** of proper nouns from live extraction (D9 seam — the `source` param takes `Inferred`, detection not built); a **protected-vocabulary tier** (D3, dam) — v1 ships the `Stated` floor + reflection prompt line and measures on device before escalating (`Corrected` overload vs. a new `Pinned` rank vs. vocabulary-aware `prune_stale`).

## Done 2026-07-06

**Photo attachments (Plan 11) LANDED** — `photos` table (migration v5, transactional, append-only): mandatory `session_id`, optional `item_id`, a shell-owned opaque `filename`, `captured_at`, sync-ready row shape (UUIDv7/timestamps/device_id/tombstone). The load-bearing fix is **demote-on-swap (D3)**: an item tombstone (the live→authoritative swap at finish, `clear_authoritative_outputs`, a manual `delete_item`) demotes that item's photos to session-level (`item_id := NULL`) rather than leaving them dangling on a tombstoned item or losing them; a session tombstone (including via `WalkSession::cancel()`) cascades and tombstones its photos outright. **File-handling seam (D4):** core owns metadata only — one query, `list_live_photo_filenames()` — and never touches bytes; the shell owns `<Documents>/photos/`, writes bytes *before* calling `add_photo` (crash-safe orphan-then-sweep), and reclaims bytes via a **reconciling sweep on app-open only** (never background — would race an in-flight capture). Processing is untouched (`SessionProcessor::process()` unmodified); photos surface via a parallel `list_photos_for_session` read path — vision analysis and document-artifact photo refs are named future work. FFI: `add_photo`/`list_photos`/`remove_photo`/`list_live_photo_filenames` on `MurmurEngine` (throwing, panic-free, `EngineError::Photo`), `WalkSession.session_id()`. iOS: functional-plain capture (PhotosPicker) + gallery wired through `WalkEngine` (demo + real-core), **visuals sac's** (`// sac:` markers). Follow-ups named, not built: vision-model photo analysis, document/PDF photo embedding, cross-device photo sharing (bytes are local-only forever). Plan: `docs/superpowers/plans/2026-07-06-rust-core-11-photo-attachments.md`.

**Photo count fast-follow LANDED (#174)** — the one follow-up from Plan 11 that didn't wait: `BoardItem.photo_count` is now wired to real per-item counts on the live board snapshot, batched one query per snapshot tick (not per-item), stale-until-next-tick accepted as the posture rather than chased with per-write invalidation.

**Base-URL Info.plist fix LANDED (#173, sac)** — `ANTHROPIC_BASE_URL` now bakes into the built app's Info.plist the same way `PPQ_API_KEY` already did, so icon-tap launches (not just simctl-launched ones) pick up a non-default provider base URL.

**Whisper model provisioning LANDED (#175)** — `fetch-whisper-model.sh` does a sha256-verified download of the bundled ggml model; `small.en` is now the default (strictly better WER/hallucination than base.en at every measured SNR on the Mac-proxy spike), with a one-arg revert (`STT_MODEL=base.en` / `sttmodel=base.en`) kept live pending the iPhone T5 on-device RTF proof.

## Done 2026-07-05 (the big day)

Re-unification complete (repo = **damsac/sitewalk**, one history, Swift Era I preserved; archive = sitewalk-archive) · issue/PR slate cleaned (19+2 Swift-era closed; #155/#156 remain) · CLAUDE.md + CI rewritten for the rebuild (#157) · **first real walk** (EST-0047, real core + key on sim) · **Plan 08 Parts A+B merged**: mic→whisper→append wiring, cancel() (closes #156's core half), transcript events, use_gpu knob (sim=CPU — D7's "Metal degrades on sim" was falsified: it SIGTRAPs), voice-walk-from-WAV proven end-to-end on sim (whisper decoded the fixture; transcript verified in SQLite). 290 tests.

## Decisions needed (joint)

- Fate of the Gallery/Screens static twins after design freeze

Template keys (`landscape | property | inspection`) and STT DONE semantics (flush over speed) are **closed** — see CANON.md's 2026-07-06 entry (sac ack via PR #167, dam via Plan 08 D6).

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
