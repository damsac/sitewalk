# Sac's State

What sac is working on right now. Updated with every PR.

---

## Headline for dam (what needs you)

*(freshened 2026-07-17 — onboarding + rename + the beta site landed this session.)*

### 1. Review + merge the launch-readiness stack, then cut a TestFlight build

This is the critical path to putting Jefe in front of real testers. All are
**app-side, no FFI surface, MERGEABLE, build-verified** on the iPhone 17 sim
(each rendered on-sim where there's UI):

- **Onboarding set** — teach a historically non-technical crew by *showing*, in
  plain words (no "AI/transcript/extraction" anywhere):
  - **#236 intro** — payoff-first welcome ("Say it out loud. Get the paperwork.")
    + 3 "how it works" beats (Walk & talk / Fix anything / One tap → paperwork),
    each with a mini phone visual. Also carries a **privacy-copy accuracy fix**:
    mic sub-header "EVERYTHING TRANSCRIBES ON YOUR PHONE" → "YOUR AUDIO STAYS ON
    YOUR PHONE" — the transcript text DOES go to the LLM, so this now matches the
    website Privacy Policy (audio local; text → AI to build the paperwork).
  - **#237 coach marks** — one-shot amber callouts on START WALK (board) + DONE
    (walk); non-blocking (target stays tappable), `@AppStorage`-gated
    (`resetcoach=1` re-arms; autoflow marks them shown).
  - **#238 optional practice walk** — a scripted, **never-saved** dry run offered
    at the end of onboarding ("Try a practice walk first"). Plays demo content
    regardless of the persisted mode WITHOUT touching `walkMode`, and exits
    without a board log / job flip (`exitPracticeIfActive()`). PRACTICE chip +
    "not saved" markers. **Stacks on #236+#237 → merge order #236 → #237 → #238.**
- **#239 rename → Jefe** — the build still shipped as "Sitewalk" on the home
  screen + mic-permission prompt. `project.yml` + committed `Info.plist` only
  (`CFBundleDisplayName`/`CFBundleName`/mic string); bundle id + Xcode target
  (`SitewalkGallery`) unchanged. Takes effect next build.
- **#235 dark-mode light-lock** — dark mode whited out ink text; locks the app to
  light appearance (`UIUserInterfaceStyle: Light`).

**None of these reach TestFlight until you merge + the release lane fires** (the
sacmeng Actions gate, item 4).

**THE ASK — cut a `v*`-tagged EXTERNAL build once the stack is merged.** We're
opening the public TestFlight link, so Isaac needs an *external candidate* to
submit for Beta App Review — that's the `v*` tag lane (`release.yml`), not the
plain-main-merge internal build. **Suggested version: `v1.1.0`** (last external
tag is `v1.0.1`; this build adds onboarding + practice walk + rename, so a minor
bump — use `v1.0.2` instead if you'd rather reserve minor for later). Exact steps
after the stack is on main:

```
git checkout main && git pull
git tag v1.1.0 && git push origin v1.1.0
```

→ external candidate builds + lands in ASC → Isaac attaches it + submits for
Beta App Review.

**Status / heads-up for the merge:**
- **F3 release-spec fix is in** — #239 now sets Jefe in BOTH `project.yml` AND
  `project-release.yml`, so the archive actually ships as Jefe (nice catch — the
  release spec's own `CFBundleDisplayName` override would've shipped "Sitewalk").
- **Known cross-PR conflicts to expect** (overlapping edits, not logic): #235 ↔
  #239 on `project.yml` (same props block — trivial keep-both; you'd already
  rebased #239, so it's yours now); #238 ↔ main on `AppModel`/`BoardView`
  (onboarding vs the #232/#241 notes edits). Both resolve by keeping both sides.
  I've stayed out of the branches so I don't collide with your rebases.
- **Isaac's ASC side is ready to submit the moment a build exists:** Privacy
  Policy URL is LIVE at https://getjefe.netlify.app/privacy (+ /terms), the
  "What to Test" + review notes are written, and he'll set the ASC listing name
  → Jefe. (Privacy policy is a URL in ASC, not baked into the build.)

### 2. React to the V2 paperwork STRUCTURE plan (#234)

`docs/design/2026-07-16-paperwork-structure-v2-plan.md`. Needs your §7 answers on
the **DocumentSchema core seam**: `list/save/remove_document_schema` FFI,
`buildDocument` resolving kind→schema→fill, doc-number minting. The plan: you land
the seam in the ~2 weeks before you're away, sac builds the Document Builder UI
during your absence, v1 ships on seeded built-in schemas (launch-safe). This is
the one big feature that needs your seam before you go.

### 3. #240 — Plan 18 notes-bucket-edit UI (blocked on your core seam)

Editable notes *buckets* UI is up (`pr/sac/notes-bucket-edit`); waiting on the
core side of Plan 18.

### 4. FYI — sacmeng account is flagged by GitHub → Actions disabled account-wide

"Actions is disabled for your account." Confirmed via a stuck `queued` Pages
deploy (actor=sacmeng) on a *public* repo with Actions enabled. Effects: (a) sac's
merges fire **no** workflows → **your** merges are the only thing that cuts a
TestFlight build; (b) it blocked GitHub Pages for the beta site (→ Netlify
instead). Isaac is on the GitHub appeal (verify email + payment method +
support/account-review). Nothing for you to do — it's just why the release lane
only fires on your actor.

## Also shipped this session (context, no action needed)

- **Beta landing/install site is LIVE → https://getjefe.netlify.app** (repo
  `damsac/jefe-beta`). Explains Jefe + a 4-step TestFlight install walkthrough +
  a Formspree waitlist, in the Field Instrument look. On Netlify because Pages was
  blocked by the account hold. Two placeholders remain: the public TestFlight join
  link + the Formspree form id.
- **Public-TestFlight path** written up for Isaac (External group → enable public
  link → Beta App Review, ~1 day). Needs a build already uploaded = your lane.

## Front-load core before your month away (my read, your call)

Since I can't touch core while you're gone, the launch-critical **core** items to
land first: **real-mic device tuning**, **walk-reopen seam (#223)**, **whisper
warm-up (#228)**, and the **#234 DocumentSchema seam**. App Store readiness is
app-side — I own it, no dependency on you.

## Notes for dam (evergreen)

- **FFI gotcha:** `build-ffi.sh --device-only` leaves the **sim** slice stale;
  bindgen regenerates `ffi.swift` + the C header from that sim lib → silently
  drops types/checksums ("cannot find in scope"). A full `./build-ffi.sh` (both
  slices) fixes it; restoring the committed `ffi.swift` alone does **not** (the
  gitignored xcframework header stays stale).
- **Device signing:** automatic → my personal Apple Development team
  (`9UQKJHZ8J3`, isaacwm23@gmail.com), bundle `com.isaacwm.sitewalk`. Separate
  from the ASC distribution identity `release.yml` uses for TestFlight.
- **id case:** core ids are lowercase UUIDv7 with a case-sensitive lookup; Swift's
  `uuidString` is uppercase → `.lowercased()` when passing item ids to the CRUD
  seam.
