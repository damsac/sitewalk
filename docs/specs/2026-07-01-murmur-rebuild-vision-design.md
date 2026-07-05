# Murmur Rebuild — Vision & Design

**Date:** 2026-07-01 (Rev 2, same day — post sac mock review)
**Status:** Approved by dam (this session). Pending sac review.
**Supersedes:** the current Swift/SwiftUI codebase. This is a ground-up rebuild in a fresh repo under the Murmur brand (working title in sac's design study: SITEWALK).

## Rev 2 amendments (sac's design study, `SITEWALK — DS-01`)

Sac's mock (jobs board → capture → transformation → document review, with landscape/property-mgmt/inspection trade templates) sharpened the product. Decisions taken with dam:

1. **Capture-first; artifacts are a pluggable seam.** The core commitment of v1 is: capture the session, learn the user, reason about the transcript. Which *artifacts* get generated (reports, priced estimates, move-out reports, inspection reports, todos…) is deliberately not fixed yet — the architecture must make artifact types easy to add (an `Artifact` abstraction + registered generators over session data), but v1 does not commit to specific document templates. The mock's estimate/report documents are the likely direction, not a locked scope.
2. **Live in-session extraction, offline-degradable.** During recording, a cheap incremental agent pass extracts typed items onto a live "Captured" board (ITEM / PART / PRICE / SAFETY-class tags) as they're spoken. The final pass at session end assembles results within a <8s "transformation" budget (no spinner — content streams in). If offline mid-session, the live board simply lags and catches up when processing queues; live extraction is an enhancement, never a dependency. This supersedes "process at end + checkpoints" in §6 and refines R1: the user watches structured items, never raw transcript.
3. **Schedule-first home.** The home screen is a deterministic jobs board (today's sites, times, statuses), not an agent-composed focus feed. `Job` (site, client, time, status, linked sessions/artifacts) is a first-class core entity. The generative-UI layer concentrates on the capture board and artifact surfaces rather than composing the home. Supersedes the composed-home framing inherited from Murmur v1.
4. Confirmed by the mock and kept from Rev 1: memory learns the *business* (letterhead identity, license numbers, pricing/rates vocabulary); manual parity includes voice edits against generated artifacts ("make that fourteen hundred"); deliberate stop (PAUSE/DONE split); share is the deliverable moment.

## Rev 3 amendments (dam steering, 2026-07-02)

1. **Adaptive reflection cadence, platform-honest.** Reflection runs when we have guaranteed compute: session end / app open. Cadence is signal-driven, not scheduled: every session at first (fast learning), stretching out once reflections stop producing meaningful memory changes; a user correction snaps cadence back up. iOS `BGProcessingTask` (opportunistic, typically overnight/charging) is a bonus deep-compression pass, never a dependency. Android may use WorkManager more freely.
2. **Forgetting is a feature.** Three mechanisms against over-remembering: the 500-word cap (reflection compresses, never accumulates), staleness decay (people/projects unmentioned for weeks are dropped), and the memory-transparency UI as user audit. Behavioral rule: **memory makes the agent quieter, not chattier** — knowing the user better means fewer questions and fewer surfaced items, never unprompted references to remembered facts.
3. **Vocabulary feeds two consumers (contextual biasing).** Onboarding is an interview: the agent asks trade, crew/client names, materials. Answers seed the memory vocabulary, which feeds (a) LLM context and (b) the STT contextual-biasing/hotword list (sherpa-onnx hotwords or Whisper prompt seeding — prior art: orator2's adaptive hotwords). Reflection keeps enriching it: transcription accuracy improves with use.
4. **Sequencing confirmed:** live extraction (Plan 04) layers on top of end-of-session processing (Plan 03), not built first. STT (Plan 05) optimizes for mobile constraints first (model size vs. quality is an explicit onboarding-cost tradeoff).
5. **Pacing principle:** this is a product build, not a feedback probe. Each plan lands complete — reviewed, tested, documented, next plan's seams explicit — before the next begins. No sprinting to milestones; rethinking is allowed when reviews surface better shapes.

## Rev 4 amendments (frontier memory research, 2026-07-02)

Survey: `docs/research/2026-07-02-agent-memory-frontier.md`. Core memory architecture validated (tiny always-in-context store, flat sectioned facts, user-visible memory, on-device). Adopted hardening: **pre-reflection snapshots** (3 rotating versions — rollback for the full-rewrite forgetting risk); **per-fact provenance** (stated/inferred/corrected + originating session); **importance-aware forgetting** (corrected facts never auto-pruned, evicted last; pure LRU rejected); **verbatim-survivor reflection rule** (protects churn signal from paraphrase noise; churn cadence is instrumented, not yet trusted — no published precedent). Vocabulary section: ≤100 curated, phonetically-confusable domain terms (iOS contextualStrings limit); it is the most aggressively curated part of memory. Episodic memory ("what did we agree at Hillside in May") is served by the session library, not fact memory — retrieval tier deferred until real usage demands it (~100+ sessions).

---

## 1. Vision

**Murmur is AI meeting notes for blue-collar field work.** A general contractor, property manager, or landscaper presses one button at the start of a site walk, inspection, or client meeting — and presses it again at the end. The agent turns what it heard into a clean report, todos, and updated customer/project records. No prompt box, no chat thread: the user should never feel like they're using an AI chat app. The agent works in the background and surfaces only what matters.

### Pillars

1. **Stupid simple.** One button. The target user is not tech-savvy, is often outdoors, hurried, wearing gloves. Every screen must be glanceable and obvious.
2. **Private by architecture.** Audio never leaves the device (on-device STT). All data is local-first, on device. The only cloud dependency is the LLM call. v1 is bring-your-own-key so we hold no user data and no liability.
3. **It learns you.** Memory + reflection: the agent accumulates vocabulary, people, projects, and preferences so it needs less from the user and surfaces more of the right things over time.

### Trajectory

iOS + Android now, built on a reusable Rust harness. Later: a web app for heavy artifacts (permit packets, kanban boards) fed by the same data and rendering the same layout protocol; and a subscription tier where we provide inference. **BYOK is the beta posture, not the go-to-market posture** — subscription inference is a launch requirement for the real audience (see §8).

---

## 2. Target user & v1 stories

Persona: general field work, GC-flavored. Core functionality is universal (reports, todos, customer info) rather than trade-specific.

Canonical day-in-the-life (Marco, GC, 3 active jobs):

1. **Morning glance** — app opens instantly to a composed home: today's todos (checkable), latest report, at most one "needs your call." Under 10 seconds to know his day.
2. **Site walk** — one button, 40-minute session, phone locked in pocket. Recording survives lock screen, pocket presses, and interrupting phone calls. Lock screen shows recording state.
3. **The payoff** — local notification when the report is ready: summary, punch list by trade, decisions captured. Trust bar: he never re-listens to audio.
4. **Sending it out** — share the report (or a slice of it) as PDF/text to client and subs. The report is the deliverable; share is also the viral loop.
5. **The correction** — one-tap inline fixes ("Dave" → "Dev") that visibly teach the agent via memory.
6. **Dead zone** — no signal: transcript still completes on-device; session queues and processes when back on signal. Queued state is calm, not an error.
7. **Customer recall** — contact/project cards auto-built from sessions, with history.
8. **Memory transparency** — a screen showing what the agent knows (people, projects, vocabulary, patterns); user can read, edit, delete.
9. **Session library** — reverse-chron browse + text search of past sessions and reports.
10. **Manual parity** — everything the agent produces can be hand-created and hand-edited: todos, contacts, report text. Nothing is agent-only.

**Deferred stories:** estimates & invoices (high value, but a second product), always-on ambient recording, teams/multi-user, speaker diarization.

---

## 3. Product rules (Murmur lessons + mock findings)

Adopted as named, testable rules:

**Recording**
- **R1 — Hidden transcript.** The live transcript is hidden behind a tap during recording. Visible text invites watching and correcting; the user should keep working.
- **R2 — Deliberate stop.** Stopping a session requires a deliberate target (slide/hold/confirm) so a pocket bump can't end a walk.
- **R3 — Background survival is a hard requirement.** Recording + on-device STT must survive OS backgrounding for hour-plus sessions. "Finishes when you're back on signal" is an engineering promise, not copy.
- **R4 — No pre-labeling.** Users won't label a session before recording. The agent infers the project/context from content; the user corrects on the report.

**Agent behavior**
- **R5 — "Needs your call" budget.** At most one open decision surfaced at a time, and only for money/schedule-class decisions. Otherwise the home screen decays into a feed.
- **R6 — Under-extraction bias.** Fewer, confident todos and contacts. One hallucinated assignee costs more trust than three missed todos.
- **R7 — Inspectable & undoable.** Every agent action shows what it did and can be reversed. Tool results reflect real outcomes.

**Keys & cost**
- **R8 — Live key validation.** A key is "working" only after a real test call, not a format check.
- **R9 — Spend visibility.** In-app spend meter and a user-set hard cap. Cost per session is measured and logged from day one.

---

## 4. Architecture

Fresh repo, sapling-style Rust workspace (RMP), Approach chosen over Tauri (Maple-style) and Dioxus/Slint because the two hardest requirements — hour-long background audio and native-feeling generative UI for non-technical users — are where webviews are weakest, and we have in-house prior art (sapling for RMP structure, orator2 for Rust audio + on-device STT).

```
murmur/
  crates/
    harness/        # reusable agent harness — zero Murmur-specific logic
    murmur-core/    # domain: sessions, entries, reports, contacts, projects
    stt/            # on-device transcription engine
    ffi/            # UniFFI layer — thin, types only, no logic
  ios/              # SwiftUI renderer + app shell
  android/          # Jetpack Compose renderer + app shell
```

### The harness crate (reusable)

The OpenClaw/Hermes analog, specialized by consumers. Contains, with zero app-specific logic:

- **Agent loop** — turn management, tool dispatch, retries, cancellation
- **Provider-agnostic LLM client** — Anthropic/OpenAI/OpenRouter-compatible; per-task model routing (cheap model for summarization/reflection, stronger for report generation)
- **Tool registry** — consumers register typed tools
- **Memory store + reflection scheduler** — see §7
- **Session/turn state**
- **Layout protocol** — serde types for the component + layout-op vocabulary (see §5)
- **Context assembler** — explicit token budgets per source (memory N, recent entries M, hierarchical transcript summaries), cost accounting

`murmur-core` is the first consumer: registers vocational tools (`create_report`, `update_todos`, `upsert_contact`, `update_memory`, …), owns storage and session lifecycle, provides deterministic fallback layouts.

### Native UIs are renderers

Each platform implements the same component library — report card, todo list, session timeline, contact card, alert banner, decision card — and applies layout ops from core. UI never invents structure; core never touches pixels. The future web app is renderer number three of the same protocol.

---

## 5. Generative UI contract

1. **The LLM is never in the render path.** App open renders instantly from the last persisted layout or a deterministic fallback. Agent updates arrive as animated diffs when ready.
2. **Diffs only, one primitive.** No whole-screen recompose tool; cold start is a batch of insert ops. One `update_layout` op vocabulary; each op maps to a native animation.
3. **Schema-versioned protocol.** Layout payloads carry a version; renderers degrade gracefully. No silent drift between LLM output and renderers.
4. **Token budget is a first-class constraint** (see context assembler, §4; spend rules R9).
5. **Deterministic fallbacks always exist** — the app is fully usable if the LLM never responds.

---

## 6. Core flow

1. **Record** — one button. Native background-capable audio capture streams into on-device STT in the Rust core. Transcript persists continuously; a dead battery loses nothing.
2. **Process** — on session end (plus periodic checkpoints for very long sessions): chunk-summarize transcript → extract report, todos, contact/project updates → emit layout ops. Runs on the user's key; offline sessions queue and process on reconnect.
3. **Surface** — home shows what the agent composed. Local notification when a queued session finishes ("Johnson walk: report ready. 6 todos, 1 decision").
4. **Learn** — background reflection updates memory (§7).

**STT:** on-device, quality-first. Engine chosen by benchmark (whisper.cpp / sherpa-onnx class) against field audio: jargon, wind, multiple speakers. Accepting ~100–500MB model size. Audio never leaves the phone; only text goes to the LLM.

---

## 7. Learning system

- **Memory file** — structured, on-device: vocabulary/jargon, people & crews, projects, preferences, patterns, corrections. Read into every agent context (budgeted). Capped at 500 words; reflection must compress, not accumulate.
- **Agent tool** — `update_memory` for in-session learning (e.g., a correction).
- **Reflection loop** — background pass triggered by accumulated activity (sessions + user corrections since last reflection); reads recent activity + current memory, rewrites memory (replace, not append).
- **Memory transparency UI** — story 8: user can view, edit, delete memory content.

---

## 8. Privacy & LLM access

- **v1: BYOK.** Key stored in platform keychain. Onboarding treats the key step as the highest-churn moment: plain-language framing, guided provider signup, live test-call validation (R8), spend meter + hard cap (R9).
- **BYOK is explicitly a beta posture.** The target market will not create API accounts. Subscription inference through our keys (or hosted/managed keys) is the launch requirement for general availability. The harness's provider abstraction is the seam; nothing else changes.
- Nothing but LLM requests leaves the device. No analytics in v1 beyond local logs.
- Audio retention is user-controlled (default: keep transcript forever, discard audio after 7 days).

---

## 9. Storage & sync-readiness

SQLite with migrations in core (sapling pattern). Local-first; designed so multi-device sync can be added without a rewrite:

- UUIDv7 ids everywhere
- Every row: `created_at`, `updated_at`, originating device id
- Deletes are tombstones
- All mutations flow through a single writer API in core, so a change-log/CRDT layer can be inserted later

No sync engine, no accounts, no server in v1.

---

## 10. Testing

- **Harness:** Rust unit tests + scenario tests with recorded LLM fixtures (deterministic replay).
- **Layout protocol:** golden tests (ops → known render trees); schema-version compatibility tests.
- **STT:** benchmark suite with field-audio samples; the engine decision is made with evidence.
- **E2E:** transcript-in → report/todos/layout-out integration tests in Rust; per-platform UI smoke tests.
- **Background survival (R3):** explicit device test protocol for hour-long locked-phone sessions.

---

## 11. Division of labor

- **dam:** harness crate, murmur-core, STT, FFI, storage — the backend and agent architecture.
- **sac:** iOS/Android renderers, component library, visual direction. The UI mocks at `docs/superpowers/mocks/2026-07-01-rebuild-ui/` (Jobsite dark/orange vs. Paper light/ink directions) and the UX findings baked into §3 are sac's input; the visual direction is sac's decision, ideally validated outdoors (dark screens glare in direct sun).

---

## 12. Explicitly out of v1

Device sync, accounts/server, subscriptions (mechanism, not the plan), web app, always-on ambient recording, speaker diarization, teams, estimates & invoices, widgets. The design leaves seams for all; none get built now.

## References

- UI mocks: `docs/superpowers/mocks/2026-07-01-rebuild-ui/index.html`
- RMP structure prior art: `~/sapling` (Rust core → UniFFI → native UIs)
- Tauri alternative considered: OpenSecretCloud/Maple
- On-device STT prior art: orator2 (sherpa-onnx streaming)
- Murmur v1 lessons: layout-diff system (`docs/plans/2026-03-04-feat-layout-diff-system-plan.md`), meta-cognitive layer design, credit-cost overrun
