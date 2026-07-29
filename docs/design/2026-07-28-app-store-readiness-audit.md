# App Store readiness — what's actually left

**2026-07-28 · sac**

Isaac asked three questions: can users upload their own document yet, do the
mechanics work, and is the UI as good as it can be. This is the audit, then the
plan. Companion to `2026-07-25-app-store-v1-tiers-and-template-upload.md`, which
holds the tier/pricing/proxy decisions — this doc does not restate them.

Everything below was verified against the tree at `c2a2040`, not recalled.

---

## 1. Can users upload their own document to be filled out?

**No — and it is further away than "unbuilt."** Two independent reasons:

**Technical.** `harness::ContentBlock` is `Text | ToolUse | ToolResult | Unknown`.
There is no image or document variant, so the harness cannot put a PDF or a photo
in front of the model at all. That is issue **#263**, filed against core, still
unanswered.

**Governance.** dam already ruled on this. From the §7 answers posted on #207:
*"upload-comprehension is Premium/v3 gated on a harness document-input
capability; extraction stays build-time-only v1."* So upload is not an oversight
we can quietly close — it is a standing decision that v1 ships without it.

**What shipped instead**, and it is not nothing: the **Document Builder** (#260)
lets an operator author their own document type — name it, define sections and
fields, choose which fields the walk fills. That covers *"I need MY paperwork,
not your three templates."* It does not cover *"here is my existing invoice PDF,
learn it."*

### Recommendation: ship v1 without upload

Isaac's original ask was to have this before the store. I think that is the wrong
trade now, and I'd rather say so than build toward a date:

- It is **entirely dam's work** (#263 is a core/harness change), and dam is about
  to be away for a month. Starting the highest-risk feature immediately before
  the longest gap in review coverage is how a launch slips by two months instead
  of two weeks.
- Upload-comprehension is the **least predictable feature in the product**.
  Arbitrary operator PDFs — scans, photos of paper, spreadsheets exported wrong —
  against a model that must infer field semantics. Every failure is silent and
  lands on a document a contractor sends a client.
- Per-parse cost is unbudgeted. §2 of the tiers doc set break-even at ~74
  walks/month at $12.99; a document-vision pass is a new cost line nobody has
  priced.

**What to do instead:** get **#263 answered and merged before dam leaves**, even
if nothing consumes it. The capability is the long pole; with it on main, upload
becomes a sac-side feature that can ship in 1.1 without waiting a month for core.

---

## 2. Do the mechanics work?

Mostly. Four real gaps, in severity order.

### 2.1 Monetization is 0% built — the largest gap

There is **no StoreKit code in the app**. No product definitions, no paywall, no
entitlement check, no restore-purchases, no walk counter. The tiers doc decided
$12.99/mo and a 5-walks/month free tier; none of it exists in the binary.

The proxy has **dollar** caps (`PER_INSTALL_DAILY_CAP_USD` default $2,
`DAILY_SPEND_CAP_USD` default $25) — a cost fuse, not tier logic. It cannot tell
a subscriber from a free user because nothing tells it.

This is the difference between an app that is ready and an app that cannot take
money.

### 2.2 Photos are invisible on the path operators actually take (#224)

Capture, persistence, orphan-sweep grace, and rehydration all work. But
`model.photos` renders in exactly one place — `ReviewView` — and `ReviewView` is
reachable only after `buildDocument` succeeds (`phase = .review`,
AppModel.swift:885). `NotesView` calls `loadPhotos` on appear and then draws none
of them.

So an operator who finishes a walk and saves notes — **the primary use case Isaac
named** ("they get an email months after the fact asking for details about a
job") — never sees the photos they took. Not during the walk's aftermath, not on
reopen. The bytes are safe; there is no surface.

A gallery strip on the notes screen closes it. Select-into-paperwork and markup
stay future work.

### 2.3 The demo engine ignores the requested document kind (#222)

`DemoWalkEngine.buildDocument(sessionId:kind:)` takes `kind` and never reads it —
it returns `trade.rows` unconditionally. Every document button in the demo build
produces the same document.

Real-core is correct (#275 fixed the analogous bug there). This matters because
demo is the no-credential fallback and the path a fresh clone takes — including
any run used to produce **store screenshots**.

### 2.4 Fixed this week, listed so nobody re-reports them

Walks now file under jobs and auto-file when the site is named (#268); a finished
walk no longer reads "DISCARDED" (#269); job cards carry dates (#273); custom
document types build, and saving a built-in no longer overwrites it (#275); the
board scrolls (#275); a filed walk no longer appears twice (#267).

---

## 3. Is the UI the best it can be?

Better than it was on Monday, and the remaining problems are the two above (§2.2,
§2.3) plus one structural note.

The board now scrolls, collapses to three unfiled walks, groups them honestly by
day, and files each walk in exactly one place. That was four separate bugs
compounding into "jobs are crammed at the bottom."

**What I would not add before launch:** more surface. The board is now carrying
walks, jobs, filing, and a document-type editor behind it. The next honest UI
work is a field session, not a feature — nearly every fix this week came from
Isaac using the app, and none came from me looking at it.

---

## 4. App Store compliance — three items nobody has logged

These are not in the tiers doc and not in any issue. Two are rejection risks.

### 4.1 `PrivacyInfo.xcprivacy` is missing — likely rejection

There is no privacy manifest anywhere in `apps/ios`. Apple requires one for apps
that call **required-reason APIs**, and we call at least two:

- `UserDefaults` (category CA92.1) — coach-mark one-shots, practice-walk state
- **File timestamp APIs** (C617.1) — `photoFileAge()` in the orphan sweep

This is a standard, cheap fix and a standard rejection when omitted.

### 4.2 The App Privacy label must say transcripts leave the device

The audio genuinely never leaves the device — whisper is local, and that claim is
defensible. **The transcript does leave**: it goes to Anthropic through our
proxy, because that is how extraction and summarization work.

The nutrition label has to disclose that, and the beta-site copy has to not imply
otherwise. Getting this wrong is both a rejection risk and, worse, exactly the
kind of thing that destroys trust with contractors who talk about client property
all day. We should say it plainly and in our own words before anyone discovers it.

The proxy is built to support the honest version of that claim: its file header
rule is that nothing may log, store, or forward a request or response body, and
it forwards bodies byte-identically. We can truthfully say we do not retain
walk content.

### 4.3 Subscription disclosure — guideline 3.1.2 (FIXED, #279)

An auto-renewable subscription must state, **in the binary** and on the purchase
screen: title, period length, price per period, and functional **Terms of Use**
and **Privacy Policy** links. The first three were present in #277; the renewal
terms and both links were not. Fixed in #279 and verified — both URLs return 200.

### 4.4 Listing prerequisites — two of these were my error

- **Privacy policy + support URL already exist** and are already accurate.
  `getjefe.netlify.app/privacy.html` and `/terms.html` are live, and the policy
  already says audio never leaves the device *and* that transcript text goes to a
  third-party processing provider that does not retain it. §4.2's "site copy"
  half was never a gap — I hadn't read the pages. What remains is the ASC
  answers, which need Isaac's login.
- **Version reconciliation was also not a real problem.** The `1.0` I found is
  the demo `Info.plist`; `project-release.yml` sets `MARKETING_VERSION 2.0.0` and
  CI overrides it from the tag. The only genuine note: ASC already holds 2.0.0
  builds, so the first public release must be **≥ 2.0.0** — `v1.0.0` will be
  rejected as a duplicate/older version.
- **App Review demo path**: reviewers get a device with no crew and no site. The
  practice walk (#238) is exactly the right asset — a scripted, never-saved dry
  run. Name it in the review notes.
- **No accounts means no account-deletion requirement.** The minimal-proxy
  decision pays off here.

---

## 5. App Attest — what the exposure actually is

The proxy moved the Anthropic key out of the binary. **`JEFE_APP_SECRET` is still
in the binary**, and install ids are client-minted (`InstallIdentity.swift`), so
anyone who extracts the secret can mint unlimited install ids.

The honest bound: the **global** daily cap ($25/day) holds, so this is not
unlimited financial exposure — it is roughly $750/month worst case. The real risk
is different and worse: **an abuser burning the global cap denies service to
paying subscribers**, who then see "service daily spend cap reached; try again
tomorrow" on a job site.

That is survivable at TestFlight scale, where the only installs are people we
know. It is not survivable on a public listing. App Attest is what ties a request
to a genuine install of our app; it stays a hard prerequisite for public listing,
and it needs a device session because the attestation path cannot be tested in
the simulator.

---

## 6. The plan

Ordered by what blocks what, not by size.

### Tier 0 — cannot submit without these

| # | Item | Owner | Status |
|---|------|-------|--------|
| 0.1 | `PrivacyInfo.xcprivacy` with CA92.1 + C617.1 | sac | **done (#277)** — verified at the built bundle's root |
| 0.2 | Guideline 3.1.2 subscription disclosure | sac | **done (#279)** |
| 0.3 | Privacy policy + support URL | — | **already existed** (§4.4) |
| 0.4 | Version reconciliation | — | **not a real issue** (§4.4); first public release must be ≥ 2.0.0 |
| 0.5 | App Privacy answers in ASC | **Isaac** | needs ASC login; must match `PrivacyInfo.xcprivacy` |
| 0.6 | Review notes naming the practice walk | Isaac | at submission |

### Tier 1 — cannot take money without these

| # | Item | Owner | Status |
|---|------|-------|--------|
| 1.1 | StoreKit 2 products + purchase + **restore** | sac | **done (#277)** |
| 1.2 | 5-walks/month meter | sac | **done (#277)** — keychain-backed, 16 tests |
| 1.3 | Paywall at the limit | sac | **done (#277, #279)** |
| 1.4 | Create the ASC subscription product | **Isaac** | `com.damsac.jefe.pro.monthly`, $12.99/mo, + Paid Apps agreements. Until this exists the paywall shows no price |
| 1.5 | Sandbox purchase on a device | **Isaac** | the one path no simulator can prove |
| 1.6 | Proxy-side entitlement enforcement | sac/dam | **deliberately deferred** — v1 gates on-device. Pairs with App Attest (§5); the dollar caps bound the abuse until then |

### Tier 2 — ships feeling unfinished without these

| # | Item | Owner | Notes |
|---|------|-------|-------|
| 2.1 | Photo strip on the notes screen (#224) | sac | **done (#278)** — strip + full-size viewer |
| 2.2 | Demo engine honors doc kind (#222) | sac | Before screenshots |
| 2.3 | A field session on the next build | Isaac | Has out-yielded every audit |

### Tier 3 — dam's lane, and the two-week clock is the constraint

| # | Item | Owner | Notes |
|---|------|-------|-------|
| 3.1 | **#263** `ContentBlock` image/document variants | dam | The only thing that ever unblocks upload — get it merged before the month away even if nothing consumes it |
| 3.2 | **#265** infer the job from the walk | dam | R4's other half |
| 3.3 | App Attest server half | dam | Hard prerequisite for public listing; needs a device session |

### Explicitly deferred

**Template upload**, per dam's #207 §7 ruling and §1 above. Revisit for 1.1 once
#263 is on main.

---

## 7. What I need from dam

Both blockers have been open with no reaction, and the window is two weeks:

- **#263** — the ruling stands, but is the *capability* worth landing now so
  upload is a sac-side feature in 1.1 rather than a month-blocked one?
- **#265** — LLM inference of a job that does not exist yet. The client-side
  matcher shipped (`AppModel.jobMatching`, deliberately conservative: whole-name
  match only, declines on ambiguity, declines on names under four characters —
  a walk filed to the wrong job is worse than an unfiled walk). The open question
  is what happens when the operator names a site with no job record.
- **App Attest** — confirming §5's framing: the exposure is availability, not
  runaway cost, and the fix is a hard gate on public listing.
