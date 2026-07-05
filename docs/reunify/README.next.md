<!--
  ⚠️ DRAFT — FOR PHASE 2 OF THE REPO RE-UNIFICATION.
  This is the intended replacement for the repo-root README.md AFTER the
  unrelated-histories merge brings sitewalk's Rust workspace to the root and the
  Swift app is retired from the tree. Do NOT copy this over the live README.md
  during Phase 1 — the live README still describes the Swift app, which is still
  in the tree. Promotion happens in docs/reunify/RUNBOOK.md, step "swap README".
-->

# Murmur

**Speak through a site walk, get a structured document.** Murmur is a field-work
voice agent for general contractors, inspectors, and trades: talk your way through
a walk-through and an on-device agent turns it into an organized report — a
landscape, property, or inspection document — without you touching the phone.

Audio never leaves the device. Local-first, sync-ready.

> Murmur began life as a general-purpose voice-to-second-brain iOS app. On
> 2026-07-01 it pivoted to field work and was rebuilt on a Rust core. The whole
> arc lives in this one repository — see **[docs/HISTORY.md](../HISTORY.md)**.

## Workspace layout

Murmur is a Rust workspace with native shells:

```
crates/
  harness/       agent loop, tools, LLM providers (Anthropic + mock)
  murmur-core/   domain + SQLite store (jobs, sessions, items, contacts;
                 tombstones, UUIDv7, single-writer, sync-ready)
  stt/           speech-to-text (whisper-rs) + vocabulary→STT biasing
  ffi/           UniFFI boundary — WalkEngine protocol, domain types only
  evals/         synthetic site-walk corpus + deterministic grader (F0.5)
apps/
  ios/           native SwiftUI shell (WalkEngine seam, AppModel.init(engine:))
docs/            specs, plans, research, history
meta/            dam + sac collaboration hub (CANON, ROADMAP, STATE)
```

*(Confirm exact crate/app paths against the tree at migration time — this reflects
the plan series 01–07 as of the re-unification.)*

## How it works

1. **Capture** — start a walk, talk. The transcript stays hidden (R1); you stop
   deliberately (R2).
2. **Live extraction** — as you speak, the agent drafts items onto the board;
   it biases toward under-extraction (R6).
3. **Process** — on finish, a two-phase extract-and-summarize pass produces the
   authoritative document (budgeted; live items are re-extracted, so the board
   "swaps").
4. **Spend meter** — token cost is surfaced (R9).

Full product rules R1–R9 are in the vision spec:
`docs/superpowers/specs/2026-07-01-murmur-rebuild-vision-design.md`.

## Building

*(Fill in from the migrated workspace — Rust toolchain + `cargo build`,
`cargo test`, `cargo clippy`; iOS shell via the app in `apps/ios/`. Replace the
Swift-era `make`/XcodeGen instructions, which no longer apply.)*

## Collaboration

Built by **damsac** — dam (harness / murmur-core / STT / FFI) and sac (renderers /
component library / visual direction). The `meta/` directory is the hub:
`CANON.md` (agreed decisions), `ROADMAP.md` (priorities), `dam/STATE.md` +
`sac/STATE.md` (current focus). PRs require a **Thinking** section — reviewers read
thinking first, code second.

## History

This repo tells the whole story: the Swift/SwiftUI Murmur (Era I), the 2026-07-01
field-work pivot, the Rust rebuild (Era II, formerly `damsac/sitewalk`), and the
2026-07-04 re-unification. Start at **[docs/HISTORY.md](../HISTORY.md)** for how to
browse each era in git history.
