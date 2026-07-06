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
