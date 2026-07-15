# Roadmap

Shared priorities and sequencing. Who's doing what, what's next, what's blocked.

Updated when priorities shift. Either person can propose changes via PR.

---

## Active

| Work | Owner | Status |
|------|-------|--------|
| **Tap-to-fix edit UI** on the notes screen — Plan 16's item CRUD seam is live (`updateItem`/`addItem`/`removeItem`, contract in WalkEngine.swift) | sac | unblocked 2026-07-14, next up |
| **Vocab pack curation** — Plan 15's bundled JSONs are placeholders (landscape/property/inspection); `VocabPackTests` schema-gates | sac | unblocked |
| **Field-feedback issues #220–#228** (dam's build-44 beta feedback, triaged + ground-truthed) — #220 dark-mode text / #221 row labels / #224 gallery UX marked sac; #222 demo work_order, #223 walk-reopen seam, #228 whisper warm-up marked dam/core | both | filed 2026-07-15, sequencing open |
| Real-mic device tuning session: voiceproc A/B, vad_rms, small.en RTF, Plan 09 Task 7 SNR rerun | dam | still pending a dedicated device session |
| Issue #155 — PR #1 review follow-ups (4 state bugs + seam hygiene) | sac | open (several now also guarded core-side by 07-carry) |

## Up Next (sequenced)

1. **Plan 17 — corrections that teach**: item-edit diff → vocabulary *suggestion* (never auto-add) + `record_correction`'s first production activation, landing TOGETHER so the signal carries its content (the Plan 16 adversarial review showed a bare counter could reinforce the misheard term). **Deliberately sequenced AFTER sac's edit UI ships** — real field corrections inform the suggest-card UX (dam confirmed 2026-07-14).
2. **Walk-reopen read seam** (#223): `listSessions`/`loadNotes` over FFI + board affordance — notes are durable (Plan 13), the data is all there; also subsumes the old recover-walk UI item.
3. **Whisper warm-up** (#228): stop constructing a fresh WhisperContext per walk; app-open or first-idle warm.
4. **Photos epic** (#224, layered): post-walk visibility → select-into-paperwork → markup editor → gallery. Persistence (Plan 11 D4) already works.
5. Paperwork Structure v2 (sac's #207 design + dam's §7 answers): core-owned `DocumentSchema` + doc-number minting for custom types; LLM-filled custom fields. Gated on sac folding the answers into a build plan.
6. Small hardening pile: cross-session gate assert on item mutations, exact `user_version == 6` pin, logo replace-failed-write edge, `phase`/`path` consolidation, `AppModel` split.
7. **Prompt-optimization loop** on the 05b eval suite (rank on F0.5, gate on recall) + the real-API live-grading extension (Plan 09 deferred thread).

### Vocabulary → STT biasing loop (Plan 10)

**Write half LANDED** — the differentiator's data path is now closable end-to-end. A vocabulary management surface on `harness::Memory` (`VOCABULARY_SECTION`/`MAX_VOCABULARY_TERMS`(100)/`MAX_VOCABULARY_TERM_WORDS`(6) constants, `VocabAdd`, symmetric-normalized case-insensitive dedup, write-time reject-when-full cap, a `Stated` provenance floor so user terms outlive `Inferred` ones under cap pressure); FFI CRUD on `MurmurEngine` (`list`/`add`/`remove_vocabulary_term`, throwing/panic-free, lock-then-save, `EngineError::Memory`); a functional-plain iOS editor wired through `WalkEngine` (**visuals are sac's** — `// sac:` handoff markers throughout); and a hermetic e2e proving add-via-FFI → `collect_bias_terms` → `build_bias_prompt`. Reflection carries one preserve-vocabulary prompt sentence (no new machinery). Real recall-lift on device is spike-harness-measured, **flagged for dam** (not CI). Plan: `docs/plans/2026-07-05-rust-core-10-vocabulary-loop.md`.

**Still open:** the **onboarding interview** that SEEDS vocabulary (D9, joint dam+sac) — the `add_vocabulary_term` path is ready to receive its output; **auto-harvest** of proper nouns from live extraction (D9 seam — the `source` param takes `Inferred`, detection not built); a **protected-vocabulary tier** (D3, dam) — v1 ships the `Stated` floor + reflection prompt line and measures on device before escalating (`Corrected` overload vs. a new `Pinned` rank vs. vocabulary-aware `prune_stale`).

## Done 2026-07-13/15 (the clearing)

**sac rounds 1+2 merged (6 PRs)** — #207 paperwork design (+dam §7 answers on the PR), #204 bucket rendering (Plan 14 loop closed), #208 back-arrow, #209 Letterhead Studio, #214 document basics (TERMS + signature, `DocumentLayout`), #215 editable-notes design (+dam §4 answers; became Plan 16). Builds 36–40.

**Plan 15 — vocabulary seeding (#212/#213, build 39)** — per-trade packs → vocab store → both STT bias and LLM context; deletion-durable pack markers that survive reflection (the adversarial review's blocker catch: reflection would have erased the marker and resurrected deleted seeds — fixed with a fifth internal-section exclusion site pre-code).

**Plan 16 — editable items (#216/#217, build 41)** — the item CRUD seam from #215: update/add/remove in core so corrections reach every rebuilt document; migration v6 `right_text` (quantity); Processed-only gating; `record_correction` deferred whole to Plan 17 (review-driven product change, dam-confirmed). sac's tap-to-fix unblocked.

**Cleanups** — #211 sweep (logo leak, dup ForEach ids, dead BuildView, dead partial_document fn), #218 SpeechSource + plist key deleted (dam's ruling; whisper is the only STT). Builds 38, 42.

**Cert-cap incident + fleet fix (#219, build 44)** — the shared Apple team hit the dev-cert cap (automatic archive signing mints a cert per CI run × three damsac apps). Jefe's archive flipped to manual signing (dry-run-gated before merge); fleet rule recorded: *the archive step mints nothing*, two blessed implementations (see `~/athanor/forge/factory-apps/release-pipeline.md` gotcha e).

**Field-feedback loop live** — dam's 9 beta submissions (build 44) pulled via the ASC API, ground-truthed against code, filed as #220–#228 with screenshots on the `feedback-assets` orphan branch. dam's standing-items list cleared to empty (ASC rename done; Anthropic-direct billing confirmed; factory secrets seeded).

## Done 2026-07-12/13

**Notes-first core, Plan 13, two stages (#197, #198)** — CANON's 2026-07-10 pivot: a walk's primary output is notes, documents become explicit per-trade action buttons instead of an auto-built DONE artifact. Split into two mergeable stages because merging on this plan auto-publishes the TestFlight internal lane (real-engine): **Stage 1 (#197)** landed the additive, inert half — the on-demand `build_document(kind)` FFI path exists and works, but `MurmurEngine.swift` calls nothing new and `finish()` still auto-builds, so a merge behaves identically to before. Final review caught the **N3 blocker** before it shipped: the plan's approved condition would have redefined `doc_kind_for_template` (property → `"condition"`) in a way that changed *live* behavior through a function Stage 1 still shared with the old auto-build path and the offline fallback — deferred cleanly to Stage 2 rather than landing an inert-looking change with a live side effect. **Stage 2 (#198)** was the atomic flip: `finish()` now returns `NotesPayload` (items + summary, no auto document build), and every `docKind` Swift `switch` arm was updated in the same PR so no build ever ships with a dangling case.

**Plan 14 — comprehensive notes (#203)** — Isaac's coordination-artifact ask, sharpened on sac's #199 thread: notes needed to carry client/team coordination detail (budget, deadlines, access, "darker mulch than last year"), not just terse extracted items. Landed by growing the existing `write_summary` call into `write_notes` (option b of three considered — no new LLM call, no load added to the latency-sensitive live-extraction pass) with a **four-bucket contract**: `scope_of_work` / `constraints` / `conditions_and_issues` as `{bucket, label, detail}` triples (summary is not a bucket, it lives on `session.summary`); the FFI boundary drops any bucket string it doesn't recognize rather than coercing it (R6). Persisted as a `kind="notes"` artifact. The eval suite's invariance is gated, not assumed: `cargo test -p evals` is **Δ=0** against pre-14 output, because the grader never reads artifacts. Data is payload-only until sac's #204 renders it.

**TestFlight-honesty saga, builds #24–#34** — a chain of publish-pipeline fixes closing out real-world use: app icon (Walked Wave glyph, missing 120x120 was silently rejecting every build, #194) alongside a loose-grep false-green fix (a bare `error` grep had red-flagged build #24's actually-successful upload, #195); export-compliance key + STT default reverted to base.en (#196); version bumped to 2.0.0. Then the real bug: every TF walk 401'd because the baked key is Anthropic-issued, not PPQ-issued, and an earlier fix (#193) had wired a `ppq.ai` base-URL override on the inverted assumption — **root-cause fixed by removing the override (#205)**, making build #33 the first TF build where real walks actually work. **#206** closed the loop: `Failed` sessions (including everything stranded during the #24-32 dead-key window) now retry automatically on next app-open, capped at 5 oldest-first, with zombie-Recording recovery falling out of the same ordering for free. **dam confirmed real walks completing end-to-end on build #34, 2026-07-13** — the first field-verified proof this pipeline works outside the demo engine.

**Jefe brand (#200, #202)** — Isaac picked **Jefe** off the #188 rename shortlist (sac's research found "Sitewalk" collides with ≥7 products including a direct competitor); sac shipped the hard-hat icon + amber theme (#200), dam co-signed in CANON (#202), superseding the 2026-07-06 Sitewalk brand decision. Repo name stays `damsac/sitewalk`; ASC/TestFlight listing rename remains a standing follow-up.

## Done 2026-07-08

**TestFlight pipeline reconciled and merged/ARMED, first publish SUCCESS (#184)** — the branch carried over from #183's Up Next (`pr/dam/testflight-rebuild`) landed reconciled, dry-run green on run `28900094459` (workflow_dispatch, upload=false: build + sign + export the .ipa, no ASC upload). Merging armed the two live lanes: a push to main now means internal auto-publish (continuous, team dogfoods), a `v*` tag means an external candidate. **The milestone**: merging #184 fired the first-ever publish — **build #18, 2026-07-08, the rebuild is live on internal TestFlight**. It ships the demo engine (no PPQ key baked into the archive) — a standing decision, not an oversight, pending the real-engine-beta call below.

**Zombie-Recording sweep (#185)**, published as build #19 — a session crash-orphaned mid-recording (app killed, device reboot) previously sat in `Recording` state forever, invisible to any UI. On app open, the core now sweeps and flips any such session to `Failed`. The race (a session finishing normally *during* the sweep) is closed by a pinned ordering invariant. The todo-leak — orphaned partial work is failed-out, not recovered — persists **deliberately**; a future recover-walk UI is named, not built.

## Done 2026-07-07

**iOS defaults to live mic on physical devices (#182)** — icon-tap launches (no launch args) previously resolved `live=false` on every platform, so a real-core device build silently ran the scripted text walk with no microphone. `resolveLive()` keeps the scripted default on the simulator (Metal STT still SIGTRAPs on `MTLSimDevice`, and screenshot/QA automation is built around scripted) but defaults to live on a physical device; explicit `live=1`/`live=0` launch args always win on either platform.

**Onboarding vocabulary-seeding design doc (#181)** — discussion draft, not a build. Lays out how the onboarding interview seeds the vocabulary surface Plan 10 built. Wants dam+sac reactions; top 3 open questions include 2 that are joint before implementation starts.

**CI: stale UniFFI bindings gate (#180)** — new third CI job (`apps/ios/check-bindings.sh`) regenerates `ffi.swift` from a hermetic host build of `crates/ffi` (release, no whisper feature — same build already exercised by `cargo test --workspace`, no macOS runner needed since uniffi's proc-macro metadata is platform-independent) and fails the build if it diverges from the committed copy. Added because the bindings went stale twice: #176 regenerated late, #179 shipped stale until final review caught it.

**Photo capture off the main actor (#178, closes PR #176's should-fix)** — `WalkEngine.attachPhoto` is now async; `MurmurEngine` runs the actual FFI call on `Task.detached` so the main actor stays free while the call waits on the store lock the Rust pump thread also contends for. `capturePhoto` keeps its synchronous, fire-and-forget signature but chains onto `AppModel.photoCaptureChain`, so rapid taps stay sequential and photos append in tap order rather than completion order.

**Doc-row item identity (Plan 12) LANDED (#179)** — document rows carry an optional, durable core `item_id`, so the review document can group photos per item. The mechanism is **echo-and-validate**: the forced `build_document` call is now fed this run's **authoritative** items (id/kind/text, a reference block appended to the user message) and the model echoes the matching `item_id` onto a line it builds from that item; `BuildDocumentTool` validates every echoed id against the run's authoritative set and applies **degrade-to-None** (missing / hallucinated / cross-session / already-claimed → `None`, first-wins dedup for injectivity) — no branch ever fails the build (R7). The **dangle invariant is earned by construction** (Plan 11 D3, for rows): the validation set is the same `created_ids` Arc threaded through `run_build_document` — never a fresh store query — and that identical Arc is what `finish_session_processed` sweeps by immediately after, so a validated row id can never reference a tombstoned item. **No SQLite migration** — the document lives in `artifacts.body` as JSON parsed by the existing hand-rolled tolerant parser; a pre-Plan-12 body has no `item_id` on its lines and renders unchanged. FFI: `DocLine.item_id: Option<String>` (additive uniffi field); `partial_document_from_items` (the offline fallback) carries `Some(item.id)` trivially. iOS: `DocRowFixture.itemId: String?`, `MurmurEngine`/`DemoWalkEngine` parity, and a functional client-side join in `ReviewView` grouping `model.photos` under the row whose `itemId` matches (session-level catch-all for the rest) — **grouping visuals are sac's**, `// sac:` markers throughout. Follow-ups named, not built: **tap-a-row → jump to item/photos** (sac), photos-in-PDF, vision analysis (Plan 11 future work, unchanged). Plan: `docs/superpowers/plans/2026-07-07-rust-core-12-docrow-item-ids.md`.

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
