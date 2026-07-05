# Murmur

Field-work voice agent. Speak through a site walk; an on-device agent turns it
into a structured document (landscape / property / inspection report). Audio never
leaves the device — local-first, sync-ready.

A Rust core workspace with a native SwiftUI iOS shell. (Murmur began as a
Swift/SwiftUI second-brain app, pivoted to field work on 2026-07-01, and was
rebuilt on Rust — the full arc lives in this one repo; see `docs/HISTORY.md`.)

## Quick Start

```bash
direnv allow                 # Enter the Nix dev shell (Rust toolchain, xcodegen, ...)
cargo build --workspace      # Build the Rust core
cargo test --workspace       # Run the workspace tests
```

## Architecture

Rust workspace at the repo root, native shell under `apps/`:

```
crates/
  harness/       agent loop, tools, LLM providers (Anthropic + mock) — app-agnostic
  murmur-core/   domain + SQLite store (jobs, sessions, items, contacts;
                 tombstones, UUIDv7, single-writer, sync-ready)
  stt/           speech-to-text (whisper-rs) + vocabulary→STT biasing
  ffi/           UniFFI boundary — WalkEngine protocol, domain types only
  evals/         synthetic site-walk corpus + deterministic grader (F0.5)
apps/
  ios/           SwiftUI shell (WalkEngine seam; demo engine + real-core mode)
docs/            specs, plans, research, design, history
meta/            dam + sac collaboration hub (CANON, ROADMAP, STATE)
```

Workspace members are declared in `Cargo.toml`; `spikes/` is excluded. Product
rules (R1–R9) live in `docs/superpowers/specs/2026-07-01-murmur-rebuild-vision-design.md`.

## Dev Commands

Rust (run inside the Nix dev shell — `direnv` / `nix develop`):

| Command | What it does |
|---------|-------------|
| `cargo build --workspace` | Build all crates |
| `cargo test --workspace` | Run all tests |
| `cargo clippy --workspace --all-targets -- -D warnings` | Lint (warnings are errors) |

iOS shell (`apps/ios/` — see `apps/ios/README.md` for detail):

| Command | What it does |
|---------|-------------|
| `cd apps/ios && xcodegen generate` | Generate the **demo** project (no FFI dep; clean-checkout build) |
| `./build-ffi.sh` | Build the gitignored `MurmurCoreFFI` xcframework (real-core mode; slow first run) |
| `./generate.sh` | Generate the **real-core** project (needs the xcframework + API key) |
| `xcodebuild -project SitewalkGallery.xcodeproj -scheme SitewalkGallery -destination 'platform=iOS Simulator,name=iPhone 17' build` | Build for the simulator |

A fresh clone builds and runs on the scripted demo engine with zero setup — the
base `project.yml` has no `MurmurCoreFFI` dependency, so `#if canImport(MurmurCoreFFI)`
is false and real-core code compiles out.

## Collaboration (dam + sac)

Built by **damsac** — dam (harness / murmur-core / STT / FFI) and sac (renderers /
component library / visual direction). The `meta/` directory is the hub:

| File | Purpose |
|------|---------|
| `meta/CANON.md` | Shared decisions both have agreed on |
| `meta/ROADMAP.md` | Shared priorities and sequencing |
| `meta/WORKFLOWS.md` | How dam and sac work together |
| `meta/RECONCILIATION.md` | PR review protocol (review thinking, not code) |
| `meta/dam/STATE.md` · `meta/sac/STATE.md` | What each is working on right now |
| `meta/dam/PROCESS.md` · `meta/sac/PROCESS.md` | How each works with Claude |

**Key principle:** PRs must include a **Thinking** section. Reviewers read thinking
first, code second. If the thinking is sound, the code follows.

**Session start:** read the other person's STATE.md and check CANON.md before working.

## Conventions

- Default simulator: **iPhone 17** (iOS shell).
- Run `xcodebuild` **outside** the Nix dev shell — inside it, Nix injects linker env
  that breaks Xcode's `ld` (`-objc_abi_version` error). Run `xcodegen`/`cargo` inside.
- Switching iOS demo↔real-core in an existing checkout can leave a stale SwiftPM
  graph in DerivedData; if a build errors with `Unable to find module dependency:
  'ffiFFI'`, delete DerivedData and rebuild. A clean checkout is unaffected.
- No `Co-Authored-By` footers on commits. GitHub org: `damsac`.
