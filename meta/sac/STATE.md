# Sac's State

What sac is working on right now. Updated with every PR.

---

## → READ THIS FIRST: `meta/dam/BEFORE-THE-MONTH-AWAY.md`

*(2026-08-02.)*

**Everything you need is consolidated there**, rewritten for the trip: what
changed in the 43 commits since your last (`c2a2040`), what needs **your review**
(the one core change I made, the audio-session fix, the proxy you have never
seen), what only you can do, and the hand-over items that become single points
of failure the moment you are unreachable.

If you have one hour: **§2.1** (my core change — it replaced a named test of
yours), **§3.1** (#263), **§4.1** (the ASC key, which has now blocked me twice),
**§4.2** (do the certs outlive the trip?).

The sections below are the older, longer-form context. The handover doc
supersedes them where they disagree.

---

## Headline for dam (what needs you)

*(freshened 2026-07-27 — superseded by the handover doc above.)*

### 0. TWO ISSUES ARE WAITING ON YOU — #263 and #265

**#263 — `ContentBlock` has no image/document variant, so the harness cannot
send a PDF to the model at all.** This is the ONLY thing blocking
"upload your own template". Everything app-side for it is built and merged: the
Document Builder (#260) doubles as the confirm-once screen, so inference just
needs to hand it a draft schema. Proposed additive `Image`/`Document` variants
in the issue — your call on shape, and happy to implement if you'd rather review
a diff than write one.

**#265 — inferring a job from the walk (R4's other half).** Not blocking. R4
says "the agent infers the project/context from content; the user corrects on
the report" — we now have the correction half. Local string matching against
EXISTING job names already ships (no model call, deterministic); the open part
is suggesting a job that doesn't exist yet.

### 0b. What landed 2026-07-25..27 (a lot — skim the headers)

**App Store readiness**
- **Proxy is LIVE** (`services/proxy/`, Cloudflare Worker,
  `https://jefe-proxy.damsac-app.workers.dev`). Holds the real `sk-ant-` key,
  meters spend per install from the response's own usage, refuses BEFORE
  forwarding once a daily cap is hit ($25 global / $2 per install). Verified
  end-to-end: real call round-tripped, bad credential 401s, misconfiguration
  fails CLOSED, counters matched a hand calculation exactly.
- **App routes through it** (#262) with NO core or FFI change — the provider
  already sent `config.api_key` as both headers and `EngineConfig` already
  carried `base_url`, so the install credential rides that field and the proxy
  swaps it. Keychain-backed install id (survives reinstall; `UserDefaults`
  would have made the free-tier allowance a one-tap reset).
- **CI now passes the proxy settings** (#270) — safe with the secrets unset
  (empty -> falls through to the direct key, no behavior change). Until Isaac
  sets `JEFE_PROXY_URL` + `JEFE_APP_SECRET`, every shipped build still uses the
  baked key.
- ⚠️ **App Attest (Phase 2) is REQUIRED before the listing goes public.** Phase
  1 authenticates with a secret extractable from any IPA; the spend cap is what
  actually bounds the loss. `authenticate()` in `src/auth.ts` is the seam.

**Cost (your domain — numbers, not vibes)**
- **Migration v8** (#255): `llm_usage` gains the two prompt-cache token
  columns, `Usage` gains the fields plus `total_input_tokens()`. Had to land
  BEFORE caching or R9 would have reported a huge saving that never happened —
  the API moves most of the prompt into the cache fields and collapses
  `input_tokens` to the remainder.
- **Prompt caching** (#259): opt-in per request, set ONLY by `Agent::run`,
  because that's the one place a byte-identical `system` + `tools` prefix gets
  re-sent. Modeled ~70% off the processing-extraction leg (a long walk ~$0.43
  -> ~$0.25). Single-shot calls stay off — a cache write bills ~1.25x and
  they'd never read it back.
- **Two things worth knowing**: live extraction sends a SLIDING WINDOW, not a
  growing transcript, and its prefix is under Haiku's 4096-token cache
  minimum — caching there is a silent no-op, not a win. And `summarize` carries
  the same transcript the extraction agent just cached but CANNOT read that
  cache: different system prompt and tool set, and tools render at position 0.
  Sharing it means reshaping the two-phase split — your call, left alone.
- **Sonnet 5 was evaluated and parked.** Its intro pricing expires 2026-08-31,
  so the durable gain is zero, and adaptive thinking is ON by default there —
  thinking bills at output rates and can exceed the caching win outright.

**Product**
- **Document Builder** (#260) — authors `DocumentSchema`s on your #244 seam.
  Also the confirm-once screen for upload. Note it exposed a dead end: the
  notes screen's document buttons came from a HARDCODED per-trade switch that
  predates the seam, so an authored doc type could never be used. Now reads
  live schemas (#266).
- **Jobs** (#264) — turned out to be a pure SURFACING job: `Job` and
  `sessions.job_id` have existed in core since v1 and were simply never on the
  FFI. Added `list_jobs`/`create_job`/`set_job_status` + `set_session_job`, and
  `WalkSummary` now carries the `job_id` it always had core-side.
- **Walk filing** (#268, #272, #273) — walks file under jobs, auto-file when
  the operator says the site name (local string match, no model call), and job
  cards date their walks.

**Two bugs real use found that building and on-sim rendering structurally
could not** — both need a SECOND action after the first paint:
- `hydrateWalkLog` was latched off for the life of the process
  (`isHydratingWalkLog` set once, never cleared), so filing wrote to core and
  the board never re-read it. Split into a latched auto path and an explicit
  `refreshWalkLog()`.
- A walk entered the log only via `completeSend()`, so finishing without
  building a document left it invisible — and once fixed it rendered
  "DISCARDED", because that label was a two-state guess off `hasDocument` and
  nothing records a discard.

### 0c. Previously (2026-07-23) — first external beta out

**Build 2.0.0 (54) is APPROVED on external TestFlight** (Isaac promoted it to the
"Jefe Beta" external group; public link + getjefe.netlify.app site are live).

**🔓 The sacmeng Actions hold is LIFTED** — Isaac's merges now fire the release
lane (verified: two runs fired with `actor=sacmeng`). So Isaac can cut TestFlight
builds himself now; you're no longer the only actor. **Workflow split is unchanged
in spirit** — you mostly backend/core, Isaac mostly frontend/UI — but Isaac may
now pick up the occasional backend/release change (like the STT model swap below).
Nothing needed from you; just FYI so a build firing on Isaac's actor isn't a
surprise.

**First tester hit two issues — both root-caused + fixed + merged to main (a
build is firing now):**
- **#250 photo-upload CRASH = OOM.** Both photo paths stored FULL-RES images
  (12–48 MP); decode+encode during a live whisper walk (model + Metal already in
  RAM) = jetsam kill. Fix caps captures at 2048px before store (`PhotoDownsize`).
- **#251 worse ACCURACY = base.en.** The external build shipped **base.en**;
  earlier local builds used **small.en** (spike-validated better WER). Re-promoted
  small.en in all 3 spots (release.yml / EngineResolution / generate.sh).
  **RESOLVED — small.en STAYS.** Isaac tested on-device (2026-07-23): accuracy is
  the clear win, and the only downside now is the **live transcript DISPLAY lags a
  touch** — "just the text lagging," capture/accuracy unaffected, and he's fine
  with it ("rather accurate than fast"). So the 07-10 lag concern is much smaller
  post-warm-up (#245). **Optional core follow-ups (not urgent, your domain):** (a)
  the on-screen transcript trail is cosmetic — could smooth if you ever want; (b)
  worth a sanity check that a LONG (~30-min) pocket walk's DONE isn't slow if
  small.en runs sub-real-time and the audio backlogs. Neither blocks anything.

### 1. Cut the `v1.1.0` external build — the stack is MERGED, `main` is READY

**STATUS (2026-07-21): the whole launch-readiness stack is on `main`** — #236
intro, #237 coach marks, #235 dark-mode, #239 rename→Jefe, #246 privacy-accuracy
fix — plus your core (#242–#245 DocumentSchema seam / walk-reopen / whisper
warm-up). `main` builds + ships as **Jefe** (verified on-sim end-to-end: onboarding
→ walk → live extraction → paperwork).

**⛳ FINAL v1.1.0 BLOCKER — merge #248, then tag.** #248 (`pr/sac/background-audio`,
MERGEABLE) is the pocket-recording fix: a walk was discarded when the phone slept
mid-recording. Adds the `audio` background mode + keeps the screen awake during a
walk (Isaac confirmed on-device that the walk now survives). It's the last piece —
**merge #248, then `git tag v1.1.0 && git push origin v1.1.0`** (see THE ASK) and
the external candidate is ready for Isaac to submit. The stack detail, for
reference:

- **Onboarding set** — teach a historically non-technical crew by *showing*, in
  plain words (no "AI/transcript/extraction" anywhere):
  - **#236 intro** — payoff-first welcome ("Say it out loud. Get the paperwork.")
    + 3 "how it works" beats (Walk & talk / Fix anything / One tap → paperwork),
    each with a mini phone visual. **Privacy-copy accuracy fix now on main via
    #246** (merged): mic sub-header "EVERYTHING TRANSCRIBES ON YOUR PHONE" →
    "YOUR AUDIO STAYS ON YOUR PHONE" — the transcript text DOES go to the LLM, so
    this matches the website Privacy Policy (audio local; text → AI). (It had
    stranded on #236's branch — committed after #236 merged — so it's re-applied
    directly; nothing needed from you, just noting it's in the tree for the tag.)
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

**Launch candidate still OPEN — Isaac's call for v1.1.0:**
- **#247 board clarity (MERGEABLE)** — the cryptic board chips (VOCAB + PAPER) were
  opaque to a non-technical audience → now ONE amber **MY BUSINESS** button opening
  a two-tab sheet (PAPERWORK [logo/colors/letterhead] + WORDS [the vocab editor]).
  Also: the mic + MY BUSINESS buttons got the START-WALK raised-block look (read as
  buttons), and the VOICE/DEMO chip is **removed** from the board (voice-only for
  users; `demo=1` still works for QA). Build-verified real-core; Isaac's eyeballing
  it on-device — if he likes it, merge before tagging so the first testers get the
  clearer customization entry.

**Not blocking v1.1.0** (both CONFLICTING, optional / next build): #238 practice
walk, #240 Plan 18 notes-buckets UI.

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

## 2026-07-28 — the app can take money, and won't be rejected on the paperwork

Readiness audit is `docs/design/2026-07-28-app-store-readiness-audit.md` (#276).
Merged since the last handover: **#277** (StoreKit 2 + the free-tier meter +
`PrivacyInfo.xcprivacy`), **#278** (photo strip on the notes screen, #224),
**#279** (guideline 3.1.2 subscription disclosure), plus **#275** and your
**#267**, which I rebased onto main and merged — a filed walk was still showing
twice and that was the real half of the "random walks" report.

**Monetization** is Jefe Pro at $12.99/mo, free tier 5 finished walks a month.
Metering is on OUTPUT, not on tapping START; the gate runs at `startWalk()` so a
refusal can never destroy a recording already made. Practice and demo walks are
exempt — neither costs anything, and metering demo would break autoflow QA at
walk six. Keychain-backed so reinstall isn't a one-tap reset. 16 tests on the
pure decision logic.

**Deliberately deferred to you + App Attest:** the meter and the entitlement
check are both on-device, so a patched binary walks free. What bounds that today
is the proxy's per-install and global daily spend caps. Proxy-side entitlement
only becomes worth building once App Attest can prove the caller is a genuine
build — before that it's a header anyone can set.

**Issue triage:** closed **#168**, **#156**, **#223**, **#228** with evidence
comments (all four were shipped and just never closed). **#222** is half done —
the blank rows are fixed, but `DemoWalkEngine.buildDocument` still ignores
`kind` entirely, so every doc button in the demo build produces the same
document. **#224** is half done — select-into-paperwork and markup remain.

**Still waiting on you: #263 and #265.** #263 is the narrowed one — I agree with
your #207 §7 ruling that upload is out of v1, and I've recommended shipping
without it. The ask is only whether the `ContentBlock` Image/Document *variants*
can land before you go, so upload becomes a sac-side 1.1 feature instead of one
that's blocked for a month.

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
