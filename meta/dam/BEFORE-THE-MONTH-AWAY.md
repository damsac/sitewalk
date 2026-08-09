# dam: everything, before the month away

**Rewritten 2026-08-02 by sac.** Replaces the 2026-07-28 version, half of which
is now stale. This is the single pickup point — if you read one file, this one.

**43 commits landed since your last (`c2a2040`, 28 Jul).** All but one are
app-side. You have not seen any of it.

Structure: **§1 orientation** (what changed, so nothing surprises you), **§2
review** (your expertise, ~2 hours), **§3 do** (only you can), **§4 hand over**
(single points of failure), **§5 don't bother**.

If you only have an hour: **2.1, 3.1, 4.1, 4.2.**

---

## 1. What changed while you were heads-down

### 1.1 The one core change — and it replaced a test of yours

**#306.** `resolve_active_schema` now treats a NULL `trade_key` as **universal**
(matches any template) rather than "only a NULL template", ordered so a
trade-specific row still wins.

This **replaced your named parity test**
`resolve_report_only_for_none_template_not_for_landscape`. I left it alone the
first time and said why: its comment — *"it stays illegal on a landscape
session, matching today"* — read as deliberate Plan 19 parity preservation, and
it was. I filed **#284** asking you rather than changing it.

Then Isaac hit it in the field. His own custom document types (Report, RFP) had
vanished from the notes screen, and his call was unambiguous: *"They should come
in regardless of trade!"* That is the product intent I said I was missing, so I
took it as the answer and closed #284.

**§2.1 is the review.** 72 lines, one file. If you disagree it is one `WHERE`
clause to revert.

### 1.2 Monetization exists now

StoreKit 2, Jefe Pro at $12.99/mo, free tier 5 finished walks a month
(#277, #279, #281).

- Metered on **output**, not on tapping START — a walk that produces nothing is
  not charged.
- The gate runs at `startWalk()`, never at finish. Refusing at DONE would
  destroy a recording already made.
- Keychain-backed (`WalkMeter`), so reinstalling is not a one-tap reset.
- **Fails open when no product is purchasable** (#281) — with no ASC product
  live, nobody can subscribe, so a hard gate would brick testers at walk six
  with no way to pay.
- Practice and demo walks are exempt.

Not a security boundary and not meant to be: the meter and the entitlement check
are both on-device. What bounds it is the proxy's spend caps, and App Attest
(§3.2).

### 1.3 The design review — the app looks substantially different

Two design docs (control system + visual review) landed across #293–#301,
#307–#309. Headlines:

- **Control system**: `RaisedBlockStyle` / `WellChipStyle` / `FieldRowStyle`.
  One rule — raised, recessed, or off; **flat means disabled**. `BlockButton`
  is gone; it hand-stacked rectangles inside a `View`, so it structurally could
  not see `isPressed`. Every button also carried `.buttonStyle(.plain)`, which
  threw away press feedback. **Nothing in the app responded to touch before
  this.**
- **Type ramp +30% with an 11pt floor**, applied in the four `Theme.F` helpers
  so ratios survive exactly. `relativeTo:` adopts Dynamic Type, which the app
  ignored entirely.
- **Contrast**: `orangeDeep` was 4.53:1 and `ink35` 2.39:1, both used as text.
  New `amberInk` (7.79:1) and `ink45` (5.09:1). Measured, not eyeballed.
- **BRIEF.md now describes the app** — gold replaces the safety orange it
  specified, renamed to Jefe, contrast exception cleared.
- **Copy pass**: em dashes and AI tells out of the app, the site, and the store
  listing. Caught a real bug — the mic-denied banner still said "SITEWALK".

### 1.4 The crash, and what it cost

Isaac and a tester both hit **SIGABRT on START WALK**, build 93.

`installTapOnBus` raises an ObjC exception when the audio session has not
activated — an `abort()`, not catchable. The crash log's **0.87-second process
lifetime** is what named it: START WALK pressed within a second of launch,
before `setActive` completed. Both `try?`s swallowed the failure, the input node
reported 0 Hz, and the tap raised.

**I diagnosed it wrong first** (a double-tap race, #303) and shipped a latch
before checking process lifetime. The latch is still correct — two walks at once
is a real bug — but it was not the cause. Fixed properly in **#305**, and Isaac
confirms it is no longer crashing.

`AudioCaptureSource` is arguably your territory. **§2.2 is the review.**

### 1.5 Two process failures worth knowing

**I burned the daily TestFlight upload quota** (#304). Ten small PRs merged in
three hours = 23 upload attempts, and Apple 409'd the one build that mattered —
the crash fix. It shipped a day late. Your honesty check from #195 caught the
false green correctly. Proposal in §3.6; I am batching merges now.

**My local `ffiFFI.xcframework` is stale** — predates the jobs FFI and
`pause_pump`/`resume_pump`, so a real-core build fails on missing checksum
symbols. **Everything I verified this week ran against `DemoWalkEngine`**, and
the crash was on the real audio path. That gap is the single biggest weakness in
my testing and it is why §3.3 matters.

---

## 2. Review — where your expertise is worth the most (~2 hours)

Ordered by how much a wrong call costs.

### 2.1 The core change (#306) — 15 minutes

`crates/murmur-core/src/store/schemas.rs`, 72 lines.

```sql
-- was
WHERE kind = ?1 AND (trade_key = ?2 OR (trade_key IS NULL AND ?2 IS NULL))
ORDER BY updated_at DESC LIMIT 1

-- now
WHERE kind = ?1 AND (trade_key = ?2 OR trade_key IS NULL)
ORDER BY (trade_key IS NULL) ASC, updated_at DESC LIMIT 1
```

Three tests replace yours: universal resolves under every template; a named
trade still cannot leak across trades; trade-specific beats universal even when
the universal row is newer.

**What I want your eyes on:** whether "universal" should have been expressed as
schema *scope* at all, or whether `doc_kinds_for_template` should have grown a
notion of shared kinds instead. I picked the smallest change that matched
Isaac's intent. You designed Plan 19 and may see a cleaner seam.

### 2.2 The audio session fix (#305) — 20 minutes

`apps/ios/Sources/Engine/AudioCaptureSource.swift`.

Now retries `setActive` three times over ~80ms, falls back to plain `.record` if
`.duckOthers` is refused (**it is not a documented option for `.record`** — that
`setCategory` may have been failing all along), and surfaces `onUnavailable` so
a dead mic says so instead of recording silence.

**Questions for you:** is 3×40ms the right shape, or should this await a real
session-activation notification? And is the silent-walk banner the right
posture, or should a failed activation abort the walk outright?

### 2.3 The proxy — you have never seen it — 30 minutes

`services/proxy/`. A Cloudflare Worker holding the Anthropic key, because a key
in `Info.plist` is extractable from any downloaded IPA. 17 vitest tests.

- Caps checked **before** forwarding; body forwarded byte-identically.
- Unknown models priced at the most expensive rate.
- Fails closed on misconfiguration.
- **File header rule: nothing may log, store, or forward a request or response
  body.** That rule is what lets the privacy policy say we do not retain walk
  content.
- Per-install $2/day, global $25/day.

**What I want checked:** the spend accounting against your R9 model, and whether
the cap posture is right — a heavy tester hits $2/day at roughly 11 walks and
sees a hard refusal.

### 2.4 Worth a skim, not a review

- **#277** monetization — app-side, but it is a product surface with money in it.
- **#293** the control system — the file everything else now depends on.
- **#305 → #303** in that order if you want the crash story; #303 is a defect I
  shipped and then corrected.

---

## 3. Do — blocking, or month-blocked

### 3.1 Land the `ContentBlock` Image/Document variants (#263)

**The capability only. Not upload, not comprehension, not a revisit of your
#207 §7 ruling.**

`ContentBlock` is `Text | ToolUse | ToolResult | Unknown`. The harness cannot
put a PDF or a photo in front of the model at all.

I agree with shipping v1 without upload and have recommended exactly that. But
the *variants* are core and they are the long pole. With them on main, upload
becomes a sac-side feature that can ship in 1.1. Without them it waits a month.

**This is the single highest-leverage hour of core work available.** If you
think landing a capability with no consumer is wrong, say so on the issue.

### 3.2 App Attest — server half DONE (#315), device half is yours (#316)

Hard prerequisite for a **public** listing. Not for TestFlight.

`JEFE_APP_SECRET` ships in the binary and install ids are client-minted, so
anyone extracting the secret can mint unlimited ids.

State the exposure accurately: the **global $25/day cap holds**, so this is
~$750/month worst case, not unbounded. The real risk is **availability** — an
abuser burning the global cap denies service to paying subscribers mid-job.

**Update 2026-08-08.** I wrote the server half — #315. Chain verification to
Apple's root, the nonce/keyId/rpId/counter/aaguid checks, assertion counter
replay protection, and an hourly token so the app asserts once an hour rather
than once per message. It ships **inert**: `ATTEST_MODE=off`, and nothing on
the device attests yet, so merging changes nothing about the running service.

The device half is #316 — `DCAppAttestService`, the entitlement, token caching,
and the `jefeA.` credential. Yours, because it touches app identity and
provisioning.

**The one thing I could not test.** The sandbox had no network, so I could not
fetch Apple's real root CA, and I was not willing to paste in a certificate I
could not verify. The tests build a synthetic chain with real ECDSA keys — that
proves every check fires on every malformation I could construct (verified by
mutation), but it means **the verifier has never seen a genuine Apple chain.**

That closes only on a device: configure the worker, `ATTEST_MODE=monitor`, one
real attestation from a TestFlight build, confirm `attest_ok` in the logs and
not `chain_signature_invalid`. If it fails, the suspects are all in
`services/proxy/src/der.ts` and all have unit tests to extend.

Rollout is `off` → `monitor` → `enforce`, and `monitor` is not skippable —
`enforce` before builds carry the device half locks out every install we have.

### 3.3 The device session you have been carrying

voiceproc A/B sweep, Plan 09 Task 7 rerun, small.en RTF validation.

**This is the biggest un-managed quality risk in the product and it is entirely
yours.** The whole thing is a voice product. If field transcription is mediocre,
none of the paperwork, jobs, or billing work matters — and nobody else can
validate it. My testing this week ran against the demo engine (§1.5), so real
STT quality is genuinely unmeasured right now.

### 3.4 Walk summaries are verbose and narrate the recording (#298)

Real output from Isaac's device:

> "Field session to discuss mulch work. Only the word "mulch" was clearly
> audible in the recording, with no additional context provided about scope,
> timing, or constraints."

Two problems: **every** summary opens with "Field session to/at", and the model
narrates its own difficulty rather than the job.

I patched the **display** side — first sentence only, boilerplate opener
stripped, 16 tests. That fixes the board. It does **not** fix the notes screen
(full summary, so the apologetic paragraph is the first thing read after a
walk), and my lead-in list is a hardcoded guess at the model's phrasing that
will rot the moment the prompt changes.

**The durable fix is the prompt**, and it may be R6-adjacent: the model fills a
gap with prose instead of declining to. For a quiet walk `Mulch work discussed`
is shorter *and* truer.

### 3.5 Infer the job from the walk (#265) — your half

The client-side matcher shipped: whole-name match only, declines on ambiguity,
declines on names under four characters, 8 tests mostly pinning where it must
refuse.

Open: **what happens when the operator names a site with no job record.**
Options — create silently (manufactures records from a mishearing), suggest on
the notes screen and let them confirm (R6-shaped), or nothing in v1. I lean
suggest-and-confirm but the extraction pass is yours.

### 3.6 The CI upload-quota guard (#304) — written, DRAFT PR #314, your call

Rather than cancelling in-flight runs (`cancel-in-progress` is `false` and I
read that as deliberate), **skip only the ASC upload when `github.sha` is no
longer the tip of main**:

```yaml
- name: Skip upload if superseded
  run: |
    TIP=$(git rev-parse origin/main)
    if [ "$TIP" != "$GITHUB_SHA" ]; then
      echo "superseded by $TIP — skipping upload"
      echo "upload=false" >> "$GITHUB_OUTPUT"
    fi
```

Nothing is ever cancelled mid-`altool`; ten rapid merges cost one slot; tags
unaffected.

**Update 2026-08-08.** I wrote it — **#314, left as a DRAFT on purpose.**
`release.yml` is the one file that strands everyone if it breaks while you are
away, and this is a convenience fix for a problem I caused, so the decision is
yours rather than mine. Nothing changes until you click merge.

Shipped version differs from the sketch above in two ways, both deliberate:
`git ls-remote` instead of `rev-parse origin/main` (one network call, correct
under `actions/checkout`'s shallow clone), and it **fails open** — if
`ls-remote` returns nothing the guard is skipped, because burning a quota slot
is a much better failure than silently not shipping. Guard is scoped to the
`internal` lane only; tags and `workflow_dispatch` always upload.

Rejecting it outright is a defensible answer. I will just keep batching merges.

---

## 4. Hand over — single points of failure with your name on them

### 4.1 The ASC API key — this has now blocked me twice

`~/secrets/apple/` is empty on my machine, so the App Store Connect API is
closed to me. It has cost real time twice:

- Isaac's screenshot feedback — he had to send screenshots by hand.
- **The build-93 crash** — I could not pull the crash log and spent an hour on a
  wrong theory before he exported the `.ips` himself.

Three files (`asc_key_id`, `asc_issuer_id`, `AuthKey.p8`) and I can pull crashes
and beta feedback directly. **With you away for a month and testers on a build
nobody can triage, this is the highest-value ten minutes on the list.**

### 4.2 Do the signing cert and provisioning profile outlive the trip?

You root-caused the cert-cap incident (#219) and moved the archive to manual
signing on shared team `98GXNZ6NKZ`.

**Check the expiry dates against your return date and say so explicitly.** If
either lapses mid-month, every TestFlight build fails and I cannot fix it —
that is the scenario that costs the whole month. If a renewal is due, do it now
rather than leaving a bomb on a timer.

### 4.3 A short "if CI breaks" note

Two or three lines: what fails most often, what you would check first. Not a
runbook. Just enough that I do not burn a day rediscovering what you know.

### 4.4 Regenerate the FFI, or tell me how

My `ffiFFI.xcframework` is stale (§1.5), so I cannot build or test against real
core. If `./build-ffi.sh` is all it takes, say so and I will run it. If there is
a gotcha, write it down.

---

## 5. Explicitly do NOT spend time on these

- **Template upload comprehension** — out of v1 by your #207 §7 ruling; I agree.
- **Proxy-side entitlement enforcement** — deliberately deferred. Before App
  Attest it is trusting a header anyone can set. It pairs with §3.2 or it does
  not happen.
- **Plan 17 correction loop, vocab onboarding v2 (#226), low-confidence
  highlighting (#227)** — all good, none launch-critical.
- **Reviewing the design-review PRs individually.** There are ~15. §2.4 lists
  the two worth skimming.
- **The store screenshots.** They predate the redesign and are mine to
  regenerate; the capture is scripted.

---

## 6. State of the world

**Shipping**: build 101 on TestFlight. Crash fixed and **confirmed by Isaac on
device**. Photos confirmed working. External testers are still on **build 54**,
which is 47 builds and one known crash behind — promoting is gated on Isaac
walking 101 once.

**Closed since you last looked**: #156, #168, #221, #222, #223, #224 (the
"photos vanish" half), #225, #228, #284, #289.

**Isaac's, and they gate submission**: create the ASC subscription
(`com.damsac.jefe.pro.monthly`, $12.99/mo) + Paid Apps agreements; the App
Privacy answers (exact clicks in `docs/store/SUBMISSION-KIT.md` §4); a sandbox
purchase on device. **Nothing about billing has ever been observed working** —
no product exists, so the paywall shows no price and the meter is untested in
practice.

**Mine while you are gone**: App Store submission end to end, screenshot
regeneration, the `Sources/Screens/*` duplicate cleanup, and the P2 design items
(chrome layer, dark-mode unlock). None of it needs core.

---

## The one-line version

**Review #306 and #305, land #263, answer the cert question, and drop me the ASC
key.** Everything else can wait for you.
