# dam: what we need before the month away

**Written 2026-07-28 by sac, for dam and dam's Claude.**

You're gone for a month in a few days. **sac cannot touch core** — that's the
constraint everything here follows from. Anything in `crates/` that isn't on main
when you leave is blocked until you're back, and the App Store submission is
being worked in the meantime.

So this isn't a wish list. It's sorted by one question: **what breaks, or stalls
for a month, if you don't do it before you go?**

If you only do three things, do **A1, A2, A3**. Everything in section D is
explicitly *don't bother*.

---

## Context you may not have

Five things changed since your last pass, and two of them change what you should
prioritize.

1. **The Anthropic key is out of the app.** There's a Cloudflare Worker at
   `services/proxy/` holding it; the app sends an install id + app secret
   instead. Per-install cap \$2/day, global \$25/day, checked *before*
   forwarding. The worker is forbidden to log, store, or forward request or
   response **bodies** — that rule is the file header, and it's what lets the
   privacy policy say we don't retain walk content.

2. **The app can take money.** StoreKit 2, Jefe Pro at \$12.99/mo, free tier of
   5 finished walks a month (#277, #279, #281). Metering is on-device.

3. **Jobs are on the board** (#264, #266, #268) and walks file under them, with
   client-side auto-filing when the operator speaks a job name.

4. **Your merges are no longer special.** The account-level Actions gate is gone
   — sac's merges to main now fire the Release workflow and upload to TestFlight
   (verified: builds 74–78, all `event: push`, internal lane, `UPLOAD=true`).
   **This is why your month away is survivable at all.** Please don't change it.

5. **Template upload is deliberately out of v1**, per your own #207 §7 ruling.
   sac agrees and has recommended shipping without it. See A1 — the ask is much
   narrower than "build upload."

---

## A. Blocking or month-blocked — please do these

### A1. Land the `ContentBlock` Image/Document variants (#263)

**The narrowed ask: the capability only. Not upload, not comprehension, not a
design.**

`harness::ContentBlock` is `Text | ToolUse | ToolResult | Unknown`. There is no
image or document variant, so the harness cannot put a PDF or a photo in front of
the model at all.

Your #207 §7 ruling stands and nobody is asking you to revisit it — upload
comprehension is Premium/v3. But the *variants* are the long pole, and they're
core. With them on main, upload becomes a sac-side feature that can ship in 1.1.
Without them it's blocked on your return, which puts it two months out.

If you think landing a capability with no consumer is wrong, say so on the issue
and we'll drop it — that's a legitimate call. But it's the single highest-leverage
hour of core work available before you leave.

### A2. The App Attest server half

**Hard prerequisite for a public App Store listing.** Not for TestFlight.

`JEFE_APP_SECRET` ships inside the binary and install ids are minted client-side
(`InstallIdentity.swift`), so anyone who extracts the secret can mint unlimited
install ids.

State the exposure accurately, because it's not what it first looks like: the
global \$25/day cap holds, so this is roughly **\$750/month worst case, not
unbounded**. The real risk is **availability** — an abuser burning the global cap
denies service to paying subscribers, who then see "service daily spend cap
reached; try again tomorrow" standing on a job site.

Survivable at TestFlight scale, where every install is someone we know. Not
survivable on a public listing.

**It needs a device session** — attestation can't be exercised in the simulator.
That's why it's here and not on the "when you're back" list.

### A3. The device session you've been carrying

From your own STATE: voiceproc A/B sweep, Plan 09 Task 7 rerun, small.en RTF
validation. Still pending a dedicated device session.

**This is the biggest un-managed quality risk in the product and it is entirely
yours.** The whole thing is a voice product. If field transcription is mediocre,
none of the paperwork, jobs, or billing work matters — and nobody else can
validate it, because sac has no device build path into core STT and Isaac's
reports are necessarily "it felt off" rather than an RTF number.

A month of TestFlight feedback with no mic tuning behind it is a month of
feedback we can't act on.

---

## B. Hand-over — things only you can currently do

These aren't features. They're single points of failure with your name on them.

### B1. Signing and certificates — will anything expire while you're away?

You root-caused the cert-cap incident (#219) and moved the archive to manual
signing with an imported cert and downloaded profile, on shared team
`98GXNZ6NKZ` (Jefe + Athanor + Weave).

**Please check the expiry dates on the cert and the provisioning profile against
your return date**, and say so explicitly on the tracking issue. If either lapses
mid-month, every TestFlight build fails and sac cannot fix it — that's the
scenario that costs the whole month.

If a renewal is due, do it now rather than leaving a bomb on a timer.

### B2. The ASC API key for feedback

`asc-feedback.py` pulls TestFlight feedback with a team key. Confirm sac can run
it, or write down exactly what's needed. Field feedback is the highest-value
input we have — it produced issues #220–#228 and nearly every fix this week — and
losing it for a month would be a real cost.

### B3. A short "if CI breaks" note

Two or three lines: what fails most often, and what you'd check first. Not a
runbook. Just enough that sac doesn't burn a day rediscovering something you
already know.

---

## C. Worth doing if there's time, in this order

### C1. Infer the job from the walk (#265) — the half that's yours

The client-side matcher shipped (`AppModel.jobMatching`): whole-name match only,
declines on ambiguity, declines on names under four characters, 8 tests mostly
pinning where it must refuse. That covers "the operator said a job that exists."

The open half is **what happens when they name a site with no job record.**
Options as sac sees them: create it silently (fast, but manufactures records from
a mishearing), suggest it on the notes screen and let the operator confirm
(R6-shaped), or nothing in v1. sac leans toward suggest-and-confirm but the
extraction pass is yours.

Genuinely nice-to-have. Do A1–A3 and B first.

### C2. The demo engine ignores the requested doc kind (#222, second half)

`DemoWalkEngine.buildDocument(sessionId:kind:)` takes `kind` and never reads it —
it returns `trade.rows` unconditionally, so every document button in the demo
build produces the same document.

**This is app-side, so sac can do it** — flagged here only because it's a
one-line-ish fix you might knock out while you're in there, and it matters before
store screenshots (demo is the path a fresh clone and any screenshot run takes).

---

## D. Explicitly do NOT spend time on these

Stated so you don't feel obliged.

- **Template upload comprehension.** Out of v1 by your ruling; sac agrees.
- **Proxy-side entitlement enforcement.** Deliberately deferred. Before App
  Attest exists it's just trusting a header anyone can set, so it would be
  security theatre. It pairs with A2 or it doesn't happen.
- **Plan 17 correction loop, vocab onboarding v2 (#226), low-confidence
  highlighting (#227).** All good, none launch-critical.
- **Reviewing sac's app-side PRs.** #275, #277, #278, #279, #281 are all
  app-side, tested, and merged. Read them if you're curious, not out of duty. If
  you only read one, read **#281** — it's a defect sac shipped and then caught
  (the free-tier gate could lock a tester out of the product with no way to pay).

---

## What sac is doing while you're gone

App Store submission end to end: App Privacy answers, screenshots, review notes,
the listing. Plus the app-side backlog — #222, the rest of #224
(select-into-paperwork, markup), and whatever the field sessions turn up.

Nothing on that list needs core. That's deliberate.

## What we need from Isaac (not you)

Creating the ASC subscription product, signing the Paid Apps agreements, the App
Privacy answers, and a sandbox purchase on a device. Listed here only so you know
it isn't waiting on you.

---

## The one-line version

**#263 variants, App Attest, and the device mic session — plus tell us whether
your certs outlive the trip.** Everything else can wait for you.
