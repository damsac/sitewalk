# Sac's State

What sac is working on right now. Updated with every PR.

---

## Headline for dam (what needs you)

Real-device voice walks work — sac ran them on an iPhone 16e. Off the back of
that, **the product's output model is changing (notes-first)** and there are a
handful of decisions waiting on you. Ordered by what unblocks the most:

1. **Notes-first architecture — PR #189** (design doc + `docs/design/notes-mockup.html`).
   The output of a walk becomes **notes** (Granola/Copilot style), with action
   buttons that turn them into an invoice/estimate/condition report/etc.
   Documents stop being auto-produced at DONE. I posted recommendations on the
   PR so you can react in one round. The only two answers I need to start
   building the Notes screen:
   - **finish() = notes only?** (recommend yes — drop the auto-document build).
   - **Document transform = LLM pass or deterministic re-render?** (recommend
     hybrid: re-render structure for free from the items you already return,
     one focused LLM call only for pricing — or price-book lookup first).
2. **Real-engine key on TestFlight.** #18/#19 ship demo-engine because
   bake-vs-runtime-config is undecided. Recommend: bake via CI secret for
   internal builds now; external-tester key handling later. Your call unblocks
   real-engine beta.
3. **small.en vs base.en.** sac felt real speech→transcript lag on device — the
   datapoint your device RTF benchmark was for. If small.en can't hold
   real-time on the phone, recommend shipping base.en default, small.en opt-in.

## In-flight PRs (all pushed, thinking-first)

- **#187 voice-first input mode** — persisted MIC·VOICE / DEMO chip on the
  board; voice default everywhere incl. sim. Removed the wrong launch-time
  Apple-Speech permission ask (whisper is on-device; that dialog's "sent to
  Apple" copy is untrue for us).
- **#190 onboarding + business profile + DONE fix** (stacked on #187). First-run
  welcome → business profile + trade → mic priming. Profile (app-side
  UserDefaults JSON) threads into board header + letterhead everywhere,
  replacing the fixture "Ridgeline". Board fixture jobs → this-session walk
  history + empty state. **Seam ask:** when the core grows an operator/tenant
  concept, `BusinessProfile` is the shape to migrate. Also folds the #168 DONE
  fix (gate on transcript-OR-items — a voice walk was un-finishable while the
  batched board lagged the speech).
- **#189 notes-first output** — design doc + trade-switchable mockup. See headline.
- **#188 sitewalk-cluster competitive research** — the name is taken by ≥7
  products incl. a direct (poor) competitor; rename needed. Isaac's picking
  from Jefe / Hardcopy / Goldenrod / CopyThat — CANON co-sign will come to you.

## Recently landed

- #167 review follow-ups + CANON acks (template keys; STT flush-over-speed).
- #173 base-URL Info.plist injection (icon-tap launches reach PPQ).
- #176 walk-time photo capture + photo/vocab visual design pass.
- Your #181 (vocab seeding) and #179 (Plan 12 photo grouping) merged with my
  reactions in.

## Device-test findings (iPhone 16e, first real-mic walks)

- **It works.** Real mic → whisper → extraction → document, on hardware.
- **Lag** speech→committed transcript (small.en on device) — see decision #3.
- **"wheelbarrow" heard as "water barrow" but corrected on the document** — the
  two-stage design working as intended, and the strongest case for the
  vocabulary-biasing loop (seed "wheelbarrow" → whisper hears it first time).

## What I'm doing next (no blockers)

- Draft the per-trade action-button taxonomy for notes-first (mine; Q3 on #189).
- Notes screen visuals are fully mocked; the build waits on your two #189 answers.
- Photo-grouping styling on the review document (Plan 12 seam is ready).

## Notes for dam

- **Stale items on your list, now done:** sacmeng is an org ADMIN (the CI
  auto-fire gate is the Actions approval *setting*, not membership — but see
  below); the harness Bearer/base-url patches shipped with #173; issue #2
  closed by #167.
- **CI still doesn't auto-fire on my PRs** — root cause found: GitHub Actions is
  disabled at the **sacmeng account level** ("Actions is currently disabled for
  your account"). Support ticket filed. Until it clears, your push to one of my
  branches triggers the run (you're the actor). Not a repo/org setting.
- Device signing for on-device builds: sac's Xcode team, not the ASC team
  (com.damsac.sitewalk.gallery / J4R462XD94) — sac runs a personal-team build
  with a unique bundle for testing.
