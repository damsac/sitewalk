# App Store submission kit

**2026-07-29 · sac.** Everything for the listing that could be produced without
Isaac's App Store Connect login. Copy the text below into ASC field by field; the
screenshots are ready to upload as-is.

Two things here are **decisions, not drafts** — §4 (App Privacy) must match
`PrivacyInfo.xcprivacy` exactly or Apple flags the mismatch, and §6 (subscription
metadata) has to match what the paywall already says in the binary.

---

## 1. Screenshots — ready to upload

`docs/store/screenshots-6.9/` — four PNGs at **1320 × 2868**, which is Apple's
6.9" iPhone requirement (the only size that's mandatory; Apple scales it down for
smaller devices).

| # | Screen | Why it's in the set |
|---|--------|---------------------|
| 1 | Board — three walks, two jobs | The first thing anyone sees. Shows walks *and* jobs, which is what makes this more than a recorder. |
| 2 | Recording — live transcript, 5 items captured | **The one that sells it.** Speech on the left becoming structured line items with tags, in real time. If only one screenshot gets looked at, it's this. |
| 3 | Walk notes — summary + buckets | Proves the output is organized, not a transcript dump. |
| 4 | Estimate — priced, letterheaded | The payoff: paperwork a contractor would actually send a client. |

Captured headlessly on iPhone 17 Pro Max via the `seedwalks=1` / `autoflow=1` /
`doc=<kind>` QA hooks, so the set is **reproducible** — rerun the same commands
after any UI change rather than re-shooting by hand.

**Deliberately excluded:** anything with the synthetic orange test photo the
`autophoto=1` hook injects. It looks like a rendering bug in a store listing.

**Worth adding before launch, if there's time:** a real photo on a walk, and the
Document Builder. Both need taps, so they need a hand or a UI test.

---

## 2. Listing copy

**App name (30 max):**
```
Jefe — Walk It, Send It
```

**Subtitle (30 max):**
```
Talk the job, get the paperwork
```

**Promotional text (170 max — editable without review):**
```
Walk the site and talk it through. Jefe writes the estimate before you're back in the truck. No typing, and no catching up on paperwork after dinner.
```

**Description:**
```
Jefe turns a site walk into paperwork.

Start a walk, put the phone in your pocket, and talk the job the way you'd
explain it to a helper. Jefe listens, picks out the line items and quantities as
you go, and writes it up when you tap DONE.

WALK AND TALK
Press record and get to work. Jefe keeps listening with the screen off and the
phone in your pocket. Take photos as you go and they attach to whatever you were
talking about.

THE WRITE-UP, NOT A TRANSCRIPT
You get organized notes. Scope of work, constraints, conditions, follow-ups. Not
a wall of text you have to read back. Anything it got wrong, you tap and fix.

YOUR PAPERWORK, YOUR WORDS
Turn the notes into an estimate, invoice, work order or report. Put your business
name, license and terms on it. Build your own document types if ours don't match
how you work.

JOBS THAT REMEMBER
Walks file under the job they belong to. When a client emails four months later
asking what you found, it's there, with the photos.

BUILT FOR THE FIELD
Big buttons that work with gloves on. Keeps recording with the screen locked.
Your audio never leaves your phone. Jefe transcribes it right on the device.

FREE AND PRO
Five walks a month, free. Jefe Pro takes the limit off for $19.99/month.

Made for landscapers, property managers and inspectors, and anyone who'd rather
be on site than at a desk.
```

**Keywords (100 chars, comma-separated, no spaces):**
```
estimate,invoice,contractor,landscaping,jobsite,voice,dictation,punchlist,inspection,quote,bid,field
```

**Category:** Business · secondary Productivity

**Support URL:** `https://getjefe.netlify.app`
**Marketing URL:** `https://getjefe.netlify.app`
**Privacy Policy URL:** `https://getjefe.netlify.app/privacy.html`

---

## 3. Review notes — paste into "Notes for the Reviewer"

```
WHAT THIS APP DOES
Jefe records a contractor talking through a job site and turns the speech into
paperwork (estimate, invoice, work order, report). Speech-to-text runs on the
device; the resulting TEXT is sent to an AI provider to extract line items and
write the document.

HOW TO TEST WITHOUT A JOB SITE
The app includes a built-in practice walk that needs no microphone input and
saves nothing:
  1. Launch the app and complete the short onboarding (business name, trade).
  2. On the welcome screens, choose "Try a practice walk."
  3. Tap START WALK. A scripted walk plays out — captured items appear live.
  4. Tap DONE to see the written notes.
  5. Tap ESTIMATE to generate the finished document.
Nothing from a practice walk is stored.

MICROPHONE
Required for real walks. The app also keeps recording with the screen locked
(background audio) because the phone is in the operator's pocket while they walk
the property. Without it, iOS suspends the app and the walk is lost mid-job.

SUBSCRIPTION
Jefe Pro, $19.99/month auto-renewing. The free tier is 5 completed walks per
calendar month. The paywall shows the price, the renewal terms, and links to the
Terms of Use and Privacy Policy. Restore Purchase is on the same screen.

NO ACCOUNT
There is no sign-up and no login. Nothing to provide credentials for.
```

---

## 4. App Privacy answers — MUST match `PrivacyInfo.xcprivacy`

Apple compares these against the bundled manifest. Answer exactly.

**"Do you or your third-party partners collect data from this app?"** → **Yes**

Then check exactly **two** data types and nothing else:

| Data type | Used for | Linked to identity | Used for tracking |
|---|---|---|---|
| **User Content → Other User Content** (walk transcripts) | App Functionality | **No** | **No** |
| **Identifiers → Device ID** (the per-install UUID) | App Functionality | **No** | **No** |

Everything else: **not collected.** No contact info, no financial info, no
location, no contacts, no browsing or search history, no purchases, no usage
data, no diagnostics.

### Why each answer is what it is

**Transcripts are collected.** They leave the device for the model provider.
Our proxy is forbidden to log, store or forward request bodies — but the
provider is still a third party receiving operator content, so it's declared.

**The install ID is collected.** It leaves the device on every request and the
proxy stores it in daily spend counters (`spend:install:<id>:<day>`), which
outlives the request it served. That's collection under Apple's definition.

**Photos are NOT collected — this corrects an earlier over-declaration.** Apple
defines "collect" as *transmitting off the device*. Photos never leave:
`ContentBlock` has no image variant (#263), so the harness cannot send one, and
nothing else uploads them. Declaring them would be inaccurate and would make the
privacy story look worse than it truthfully is. **If #263 lands and photos start
being sent, this table and the manifest both have to change.**

**Audio is NOT collected.** Whisper runs on device; that claim holds literally.

### The one genuine judgement call

**"Linked to identity" on Other User Content.** Apple sometimes treats data
collected alongside a persistent identifier as linked. **No** is the
recommendation here: the install ID is randomly generated on device, never tied
to a name, email or account, and cannot be resolved to a person — Apple's
description of not-linked.

Answering **Yes / Linked** to both is also defensible and costs only a
worse-looking label. Prefer **No**; it's the accurate one.

## 5. Age rating

**4+.** No objectionable content of any kind. Answer "None" to every
questionnaire item. (No user-generated content that's shared between users — a
contractor's own notes on their own device don't trigger the UGC rules.)

---

## 6. Subscription setup — must exist before the paywall works

Until this is created, `Product.products(for:)` returns empty, the paywall shows
no price, and the free-tier limit deliberately **does not** apply (#281 — we
don't refuse someone's money and their work at the same time).

- **Product ID:** `com.damsac.jefe.pro.monthly` — must match
  `Entitlement.proMonthlyID` **exactly** or the lookup silently returns nothing.
- **Type:** Auto-renewable subscription
- **Subscription group:** `Jefe`
- **Duration:** 1 month
- **Price:** $19.99 USD
- **Display name:** `Jefe Pro`
- **Description:** `Unlimited walks. Talk through as many jobs as you want and
  Jefe writes up the paperwork.`
- **Also required:** Paid Apps agreement signed, and banking + tax details
  completed under Business. Products stay in "Missing Metadata" until these are
  done.

**Flip this deliberately.** The moment it goes live the 5-walk limit starts
enforcing for everyone, including testers who have been walking freely.

---

## 7. Still blocking, and not mine

- **App Attest** (dam) — hard prerequisite for a public listing. `JEFE_APP_SECRET`
  ships in the binary and install ids are client-minted; the global $25/day cap
  bounds the cost, but an abuser burning it denies service to paying subscribers.
  See `meta/dam/BEFORE-THE-MONTH-AWAY.md` §A2.
- **First release version must be ≥ 2.0.0** — ASC already holds 2.0.0 builds, so
  a `v1.0.0` tag will be rejected as a duplicate.
- **A real device pass on the current build** — nothing here has been through a
  sandbox purchase.
