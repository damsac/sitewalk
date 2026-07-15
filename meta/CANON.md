# Canon

Shared source of truth between dam and sac for the rebuild. Every entry here has been explicitly agreed on by both — rows marked *pending* or *proposed* are not yet canon. If it's not in this file, it's not canonical.

Updated via PR. Changes to this file require review from the other person.

Pre-pivot canon (the old voice-notes app) lives in `damsac/Murmur` `meta/CANON.md` history — historical only.

---

## Product

- **What Sitewalk is now:** AI meeting notes for blue-collar field work. One button; record an hour-long site walk with the phone in your pocket; on-device transcription; an agent turns it into structured items, documents, and todos. Brand: Sitewalk (formerly Murmur — see Decisions Log, 2026-07-06). Repo: `damsac/sitewalk`.
- **Capture-first:** artifacts are a pluggable seam; live in-session extraction onto a board; jobs-board home with `Job` as first-class.
- **Privacy:** audio never leaves the device; on-device STT; local-first storage.

## Architecture

- **Rust workspace** (`crates/`): `harness` (reusable agent crate, zero app logic) → `murmur-core` (domain + SQLite) → `stt` (planned) → `ffi` (UniFFI, planned). Native shells consume it: `apps/ios` (SwiftUI), Android later.
- **Local-first storage:** SQLite, UUIDv7 ids, tombstones, single-writer `Store` — sync-ready, no sync/accounts/server in v1.
- **UI↔core seam:** `WalkEngine` protocol at the FFI boundary, domain types only (never harness wire types). Swap point: `AppModel.init(engine:)`. `DemoWalkEngine` is the placeholder until the Plan 07 bridge.
- **`apps/ios` is outside the cargo workspace** (`exclude = ["spikes"]` covers spike crates; the app has its own xcodegen project). Workspace gates: `cargo test --workspace` + `cargo clippy --workspace --all-targets`, always green on main.

## Roles

- **dam** — harness, murmur-core, STT, FFI. Owns the foundation.
- **sac** — renderers, component library, visual direction (`apps/ios/`). Owns the user-facing experience.

Centers of gravity, not hard boundaries.

## Conventions

- **Branch model:** `main` + PR branches (`sac/<name>`, `pr/dam/<name>`). PRs include a **Thinking** section; review thinking first, code second (`meta/RECONCILIATION.md`).
- **Commit format:** `type(scope): short description` — no Co-Authored-By footers.
- **Secrets:** `.env` at repo root, gitignored, shell-sourced — never committed, never read into agent context.
- **Review machinery (dam's side):** plan → plan-review → builder execution → spec+quality reviews → whole-artifact final review. The final review has caught a real cross-module issue in six of six plans — never skip it.

## Decisions Log

| Date | Decision | Proposed by | Agreed via |
|------|----------|-------------|------------|
| 2026-07-01 | Pivot: field-work voice agent; ground-up rebuild; old Swift app superseded | dam | sac's SITEWALK study shaped spec Rev 2; sac built `apps/ios` against it — de facto. Formal spec review still owed |
| 2026-07-01 | Native shells over Tauri (background audio, feel) | dam | pending sac's spec review |
| 2026-07-01 | Product rules R1–R9 (hidden transcript, deliberate stop, under-extraction bias R6, spend meter R9, …) | dam | pending sac's spec review |
| 2026-07-03 | Repo `damsac/sitewalk`; brand stays Murmur | dam | sac PR'd into it |
| 2026-07-06 | **Product name is Sitewalk** — supersedes the 2026-07-03 "repo = sitewalk, brand stays Murmur" split. Sitewalk is now both the repo and the brand; Murmur remains the historical name for the pre-pivot app (`docs/HISTORY.md`). Follow-ups owned elsewhere, not part of this sweep: TestFlight/App Store Connect listing name, and the app display name in the unmerged TestFlight branch's `project-release.yml` | dam | brand decision sweep |
| 2026-07-04 | Swap-contract fix: items get `source` (live/authoritative/manual); process() clears live items only AFTER successful extraction | dam | core-side; informs sac's board UX |
| 2026-07-04 | STT: benchmark-first — whisper.cpp Rust-side preferred, 06-spike confirms or kills. Driver: vocabulary→STT biasing; iOS 26 SpeechAnalyzer dropped custom vocabulary | dam | supersedes HANDOFF's "STT stays in Swift"; flagged in PR #1 review |
| 2026-07-04 | `WalkEngine` seam + `AppModel.init(engine:)` swap point | sac | dam's PR #1 review endorses |
| proposed | Template keys `landscape \| property \| inspection` as canonical | sac (in code) | awaiting explicit ack |
| proposed | STT DONE semantics: flush final utterance (site-walk reality) vs speed (old-app canon) | dam | joint call pending |

## 2026-07-06 — two joint decisions closed (sac ack via PR #167, dam via Plan 08 D6)

- **Template keys**: `landscape | property | inspection` are canonical. (dam proposed; sac ACK PR #167.)
- **STT DONE semantics**: **flush over speed** — the walk's words are the product; latency is negotiable. DONE flushes the final utterance (bounded grace), DISCARD drops immediately. Live implementation today is Rust-side (`EngineConfig.stt_flush_on_finish=true`, Plan 08); the Swift SpeechSource flush from PR #167 is staged fallback (un-instantiated while the whisper path is live). Supersedes the Era-I "cancelRecording over stopRecording / speed over completeness" canon.

## 2026-07-10 — notes-first pivot + companion decisions (dam answers to sac's #189/#179/#181 rounds; full context in `docs/design/2026-07-10-decisions-notes-first.md`)

- **NOTES-FIRST (adopted, supersedes document-at-DONE):** a walk's primary output is **notes** (items + summary — the payload finish() already computes); documents are explicit per-trade **action buttons** off the notes screen, never auto-built at DONE. The action-button row is the differentiation made visible (rivals stop at clean notes). Core work = Plan 13: finish() stops calling build_document; new on-demand `build_document(kind)` path. (Isaac's direction post device walks; sac design #189; dam adopted 2026-07-10.)
- **Document transform = HYBRID, LLM-pricing v1:** document structure re-renders deterministically from the structured items (free, zero hallucination); one focused LLM pricing pass only on monetizing taps (Estimate/Invoice). Price-book lookup slots in front later (lookup first, LLM fallback) — seam noted in Plan 13, not built.
- **Note = durable per-session artifact; documents = derived SNAPSHOTS** (0..N per note, keyed to it; note edits never silently change generated documents; regenerate is explicit).
- **TestFlight internal lane goes real-engine:** bake the key via GitHub Actions secret at archive time (never in repo). External-tester key handling = future decision. (dam authorized 2026-07-10; blocked only on dam running `gh secret set`.)
- **Device STT default reverts to base.en** (small.en stays one launch arg away): sac's real-device felt-lag on iPhone 16e is the datapoint the T5 tier was waiting for; accuracy work continues via vocabulary biasing, not model size.
- **Plan 12 answers (sac, dam-acked):** manual rows group photos via session scope client-side; item↔row strictly 1:1 in v1 (log split demand before designing).
- **Vocab seeding (joint, closed):** hybrid trigger riding the first-run profile flow + demo walk before the vocab card; packs = bundled JSON, sac-curated, CI schema test, all writes through the Plan 10 funnel; type-only interview v1; SEED_MAX≈60. Implementation follows Plan 13.
## 2026-07-13/15 — dam-side rulings from the sac rounds + field feedback (sac: ack or push back via PR)

- **Edits live where the item lives (from sac's #215, dam adopted):** an operator correction mutates the CORE store (`update_item`/`add_item`/`remove_item`, Plan 16), never app-side state over core data — "a correction that doesn't reach the PDF is worse than none." Corollaries: edit affordances gate on `!notes.queued`; the UI fresh-reads from the engine after every mutation; `remove` (tombstone) ≠ `done` (completion); items carry no price (pricing stays document-only until the DocumentSchema seam).
- **Confirm-once (from sac's #207 §3, dam adopted as a product rule candidate — proposed R10):** the LLM only reliably fills a *named schema*; "upload your own document" = infer once → operator confirms the mapping once → every walk fills automatically. Never trust live per-walk comprehension of an unconfirmed document.
- **Corrections→learning is Plan 17, deliberately sequenced (dam confirmed 2026-07-14):** `record_correction` + the vocab suggestion land TOGETHER (a bare correction counter can make reflection re-learn the misheard term sooner — adversarial-review finding). Suggestion, never auto-add: the 100-term vocab space is user-curated. Sequenced after sac's edit UI so field corrections inform the suggest-card UX.
- **Vocab cap is invisible (dam field ruling, issue #225):** the ≤100 curated-terms constraint is real (whisper bias budget) but the NUMBER never shows in UI; the limit degrades gracefully. Raising it is an eval-gated discussion, not a UI edit.
- **Billing: Anthropic-direct is the posture (dam, 2026-07-14):** TestFlight builds bake dam's Anthropic key; any PPQ re-route moves key + base-URL host together (release.yml documents the pair invariant).
- **Fleet signing rule (cert-cap incident, 2026-07-14):** on the shared Apple team, *the archive step mints nothing* — Jefe uses manual-at-archive (#219, valid because the FFI is a prebuilt binary target); source-SPM projects must use sign-at-export-only. One pipeline on automatic signing re-caps the whole team.

- **Rename (#188): RESOLVED — the product is JEFE** (Isaac's pick from the shortlist; **dam co-signed 2026-07-12**). Supersedes the 2026-07-06 Sitewalk brand decision (#188's research: "sitewalk" collides with ≥7 products incl. a direct competitor). Brand landed on main via #200 (hard-hat icon + amber theme, sac). Follow-ups: ASC/TestFlight listing rename to Jefe (at next publish touch); repo stays `damsac/sitewalk` for now (rename optional, redirects cover it); Walked Wave icon retired in favor of the hard-hat (glyph archive: r2 gallery).
