# Canon

Shared source of truth between dam and sac. Every entry here has been explicitly agreed on by both. If it's not in this file, it's not canonical.

Updated via PR. Changes to this file require review from the other person.

---

## Architecture

- **Two-layer project:** SwiftUI iOS app (`Murmur/`) + Swift package (`Packages/MurmurCore/`)
- **LLM provider:** Currently PPQ.ai. Architecture supports swapping to any provider.
- **Xcode project generated** from `project.yml` via XcodeGen — never edit `.xcodeproj` directly
- **Per-developer settings** in `project.local.yml` (gitignored)

## Product

- **What Murmur is:** An autonomous second brain. You speak, and an agent captures, organizes, surfaces, and acts on your entries. Not a transcription app — a thinking partner that manages your mental load.
- **Core insight:** Capture first, categorize automatically. The agent doesn't just structure your input — it actively curates what you need to see and when.
- **Entry model:** The atomic unit is `Entry`. Category (todo, note, reminder, idea, list, habit, question) carries the semantic weight.
- **Agent-first UI:** Three layers — smart list (flat, agent-curated), gestures (swipe to act), mic (voice to agent).
- **Privacy (goal):** All user data encrypted at rest. Zero plaintext storage. Not yet implemented — required for production release.
- **Credits as fuel:** Token-based usage with starter balance. Additional payment methods planned post-launch.

## Roles

- **dam** — Architecture, backend, core systems (`Packages/MurmurCore/`), and frontend contributions. Owns the foundation that the UI builds on.
- **sac** — Frontend, UI/UX, SwiftUI views and interactions (`Murmur/`). Owns the user-facing experience.

Both touch the full stack when needed — these are centers of gravity, not hard boundaries.

## Conventions

- **Branch model:** `main` (stable), `dam` (dam's working branch), `sac` (sac's working branch). PRs go from `pr/dam/<name>` or `pr/sac/<name>` → `main`. Rebase working branch onto main after merge.
- **Commit format:** `type: short description` (no Co-Authored-By footers)
- **PR process:** Feature branches to main, PR review required, includes Thinking section
- **Default simulator:** iPhone 17 Pro
- **Dev shell:** Nix flake, activated via `direnv allow`
- **Make targets:** All dev commands through `make` (see CLAUDE.md)

## Decisions Log

Append new decisions here with date, who proposed, who agreed.

| Date | Decision | Proposed by | Context |
|------|----------|-------------|---------|
| 2026-02-28 | Adopt collaborative meta structure at `meta/` | dam | Genesis: bootstrap shared process |
| 2026-02-28 | Archive old `workflows/` to `workflows.archive/` | dam | Clean slate for meta |
| 2026-02-28 | PRs must include Thinking section | dam | Review thinking, not just code |
| 2026-02-28 | Metacraft skills installed at user level (~/.claude/skills/) | dam | Shared tooling: genesis, meta-agent, session-lifecycle, tmux-lanes, gather, skill-creator |
| 2026-02-28 | Use cancelRecording() over stopRecording() in agent path | dam | Optimizing for speed. Accept potential loss of final partial transcript (~500ms of speech) rather than blocking for finalization. Revisit if users report missing words. |

## Rebuild Decisions (2026-07 pivot — agreement noted per row)

Everything above this section describes the pre-pivot app and is historical. The rebuild's canon:

| Date | Decision | Proposed by | Agreed via |
|------|----------|-------------|------------|
| 2026-07-01 | Pivot: AI meeting notes for blue-collar field work (site walks → documents). Ground-up rebuild; Swift codebase superseded, kept as reference | dam | sac's SITEWALK design study shaped spec Rev 2; sac built the iOS app against it (PR #1) — de facto. Formal spec review still owed |
| 2026-07-01 | Architecture: Rust workspace (harness → murmur-core → stt → ffi/UniFFI) + native SwiftUI/Compose shells. Native over Tauri (background audio, feel) | dam | pending sac's spec review |
| 2026-07-01 | Division of labor: dam = harness/core/STT/FFI; sac = renderers/components/visual direction | dam | operating as such since |
| 2026-07-01 | Local-first: SQLite, UUIDv7, tombstones, single-writer store; sync-ready, no sync/accounts in v1. Audio never leaves device | dam | pending sac's spec review |
| 2026-07-02 | Product rules R1–R9 (hidden transcript, deliberate stop, under-extraction bias R6, spend meter R9, …) — spec §3 | dam | pending sac's spec review |
| 2026-07-03 | Repo: `damsac/sitewalk` (sac's working title graduated to repo name; brand stays Murmur) | dam | sac PR'd into it |
| 2026-07-04 | Swap-contract fix: items get a `source` column (live/authoritative/manual); process() clears live items only AFTER successful extraction | dam | core-side, informs sac's board UX |
| 2026-07-04 | STT: benchmark-first — whisper.cpp Rust-side is the preferred bet, 06-spike confirms or kills. Driver: vocabulary→STT biasing needs it; iOS 26 SpeechAnalyzer dropped custom vocabulary | dam | supersedes HANDOFF's "STT stays in Swift" — flagged to sac in PR #1 review |
| 2026-07-04 | UI↔core seam: `WalkEngine` protocol at the FFI boundary, domain types only (never harness wire types); swap point `AppModel.init(engine:)` | sac | dam's PR #1 review endorses |
| proposed | Template keys: `landscape \| property \| inspection` as canonical | sac (in code) | awaiting explicit ack from both |
| proposed | STT DONE semantics: flush final utterance (supersedes 2026-02-28 cancel-for-speed canon for the site-walk context) | dam | joint call pending |
