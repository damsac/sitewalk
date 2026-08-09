# Positioning & GTM — what four research streams found

> **SUPERSEDED IN PART, 2026-08-09.** Isaac clarified the goal: **a side hustle
> at $1–5k MRR**, not a venture-scale business. That inverts several conclusions
> below. **Read §8 (appended at the end) before acting on §3–§5.** The
> competitive facts in §1–§2 stand; the strategy built on them assumed an
> ambition Isaac never had.

**2026-08-08 · sac.** Commissioned by Isaac. Four parallel research streams:
direct voice competitors, incumbent FSM threat, segment selection, GTM channels.

**Read §1 first. It corrects things I told Isaac in July that turned out to be
wrong**, and those errors are load-bearing — one of them is in his résumé.

---

## 1. Corrections to what I said before

### 1.1 "The market is barbelled at $19 typing and $150 voice" — WRONG

I told Isaac on 2 Aug that competitors either made you type ($19–49/mo) or
charged $150/mo for voice, and that $14.99 voice was an open gap.

**There is no gap.**

- **VoxTrade** — voice-first, **£14.99/mo**, free tier of **5 quotes/month**,
  lists landscapers, ships on iOS + Android + web. A price twin, a free-tier
  twin, and a workflow twin. It even publishes *"VoxTrade vs Jobber: do you need
  a £300/mo platform or a £15/mo quoting app?"* — the exact wedge argument I
  drafted for us.
- **Jobber shipped voice in September 2025** — eleven months ago. $24/mo annual,
  $49 monthly, **bundled on all plans**, and their press release explicitly
  names *"creating quotes"* and *"sending invoices"* hands-free. Landscaping is
  one of their core verticals. 300k+ pros.
- **Housecall Pro** shipped voice-activated invoicing the same month.
- **ServiceTitan Atlas** added natural-language voice in their tech mobile app.

I ran the V2E search in July, found one expensive competitor, and generalised
from it. I should have searched the incumbents' 2025 changelogs before telling
Isaac the category was open.

**Résumé impact:** the bullet *"Found competitors either made contractors type
or charged $150/mo to talk; launched voice at $14.99"* no longer holds. If it
has gone out, the defensible version is about **where audio is processed**, not
about price. Replacement in §6.

### 1.2 "On-device audio is a differentiator" — RIGHT, and stronger than I knew

Handoff's own privacy policy says, verbatim, that audio is *"sent to our
third-party speech-to-text API providers (currently Google's Speech to Text API
and OpenAI's Whisper API)"*, that they may *"train AI models"* on it, and that
retention is at least 12 months.

**No direct competitor makes an on-device claim.** VoxTrade, V2E, Invoyce,
TradieNotes and Voice to Invoice disclose nothing at all about audio handling.
The only on-device speech products that exist are generic dictation utilities
that produce no paperwork.

Jefe appears to be the **only product at the intersection of on-device audio and
finished trade documents.** That is now the single most defensible thing we own.

---

## 2. The competitive picture, stated plainly

### 2.1 The window is closing, measurably

Apple's live App Store autocomplete surfaced **six near-identical apps shipped in
the last 90 days**: BuildWalk (Voice Estimates), BidWalk, SiteBrief, Walkthrough,
Punch List AI, Rafter. Plus Estimio, Forge, Paidkit, QuoteCat, Fairbid in
autocomplete.

All have **roughly zero ratings**. Nobody has traction. But "voice estimates" as
a search phrase already resolves to BuildWalk, and "walk"-based naming is now
crowded.

### 2.2 Three tiers of competitor

| Tier | Who | Threat |
|---|---|---|
| **Platform incumbents with voice bolted on** | Jobber ($24–49), Housecall Pro ($59–79), ServiceTitan | **High.** They own the customer list, price book, scheduling and payments. Voice is free inside a product they already pay for. |
| **Voice-first direct clones** | VoxTrade (£14.99), V2E ($150), CountBricks, Skava, + 6 new App Store entrants | **High.** Same idea, same price, no moat between us. |
| **Vertical AI-report tools** | FieldScribe AI (adjusters), InspectMind AI (YC W24, structural), Worksmith Marine, Spectora/Verispec/SwiftReporter (inspection), SnapInspect (PM), Cleri (arborists) | **High per-segment.** Each vertical already has a funded voice product. |

**The uncomfortable conclusion: "talk through a walk, get paperwork" is not a
product position any more. It is a feature description that a dozen companies
share.**

### 2.3 But everyone is bad at the same four things

From App Store, Capterra and G2 review corpora:

1. **The numbers are wrong.** The dominant complaint everywhere. Handoff
   reviewers: *"I end up having to change most numbers"*, *"inconsistent price
   quotes on same products"*, *"off by approximately 25% low"*. The universal
   coping strategy is manual override.
2. **Setup burden.** *Contractor Magazine*: field-service software has "a
   notoriously high abandonment rate", ~20 hours of owner time to implement.
   *"Half your techs are still texting customers from their personal phones."*
3. **Metered minutes.** VoxTrade caps Pro at **120 min/month**. Handoff meters
   AI credits. Every cloud competitor must meter, because ASR costs them per
   minute.
4. **Nothing helps you find the walk again** six months later, in a dispute.

**Three of those four are things on-device architecture answers structurally,
not with effort.** Uncapped minutes is free for us and impossible for them.

---

## 3. Positioning

### 3.1 Stop selling voice. Sell what voice costs everyone else.

The claim that survives contact with this research is not "talk instead of type."
It is:

> **Your walk never leaves your phone. Talk as long as you want.**

That single line does three jobs at once: it names the privacy position nobody
else can claim, it names the uncapped-minutes benefit that follows from it, and
it implicitly attacks every competitor's cost structure.

### 3.2 The legal peg makes privacy urgent rather than nice

*In re Otter.AI Privacy Litigation* is a live consolidated federal class action
alleging Otter recorded meetings and **trained models on them** without all-party
consent. NPR covered it; employment and privacy firms are publishing client
advisories.

A contractor recording inside a customer's home, in a **two-party-consent
state**, on a job that may become a litigated insurance claim, has a concrete
legal exposure — and Handoff's policy says they ship that audio to two US cloud
vendors and train on it.

**This converts "privacy" from a nerd feature into a liability argument a
contractor understands in one sentence.**

### 3.3 The one thing still unverified

Jobber Voice appears to be **command-driven** ("create a quote for Jane Smith")
against an already-configured account — not **narration-driven** (talk
continuously for twenty minutes, get a structured multi-line document). Their
help centre 403s to automated fetch and no hands-on review exists.

**If that distinction is real, it is the product moat. If it is not, we have
one fewer.** Someone has to install Jobber and test it. This is the highest-value
hour of work available right now.

---

## 4. Segment: the recommendation is a pivot

### 4.1 All three current segments score badly

| Segment | Verdict |
|---|---|
| **Landscaping** | **Avoid.** Best solo density in America (~81% of 692k are nonemployer) but the *document is too trivial* — most estimates are two lines ("mow, $45/wk"), which is exactly where voice saves nothing. Jobber owns the vertical and shipped voice. Buys only Feb–April. |
| **Home inspection** | **Avoid as beachhead.** Best product fit in the study (4–6 narrative reports/wk, 3–4 hrs of writing each, 70% solo) — but **five** voice-enabled incumbents already ship, only ~25k people exist, and the switching cost is the 8,000-comment library and E&O archive, not the transcription. |
| **Property management** | **Avoid.** Wrong buyer — software is chosen by the office, not the person walking the unit. No solo base. Unreachable without a sales motion. |

### 4.2 Recommended beachhead: small water/fire damage restoration shops

**60,020 US businesses.** The category leader, Encircle, starts at **$270/mo**
and has **~3,000 customers — about 5% penetration.** Roughly 57,000 restoration
businesses are documenting jobs with a phone camera and a notepad because the
tool built for them costs $3,240/year.

Six reasons this is the right beachhead:

1. **Documentation is per-job-per-day, not per-week.** A four-day dry-out
   produces a work authorization, a scope narrative, a cause-of-loss statement,
   and **daily moisture/psychrometric logs per room**. Our 5-free-walks tier is
   consumed in week one of a single job. That is the fastest natural conversion
   trigger of any segment studied.
2. **No voice incumbent.** Encircle Scope is *photo* → scope. Xactimate is
   desktop line-item entry. Nobody is doing walk-and-talk here.
3. **Year-round.** No seasonal dead zone — unlike landscaping (Feb–Apr) or
   chimney (Sep–Dec). With two founders we cannot afford an eight-month wait.
4. **Privacy is load-bearing here specifically.** Techs record inside flooded
   homes, beside distressed homeowners, on claims that frequently end up
   litigated. This is where "audio never leaves the phone" stops being a feature
   and becomes an answer to a discovery request.
5. **IICRC S500/S520 gives the DocumentSchema work a real target** — a
   published, non-negotiable format. A far better first proof than "a
   landscaping estimate."
6. **Reachable free.** Dense Facebook groups, r/restoration, an active podcast
   circuit, RIA. And the pain already has a name they use: *"the second shift"* —
   the 9pm write-up.

### 4.3 The go/no-go test that decides this

**A live mitigation site runs 4–8 air movers and a dehumidifier: a sustained
70–80 dB broadband noise floor. That is precisely the condition that breaks
on-device ASR.**

Before any restoration-specific code is written, we need to know whether whisper
holds accuracy next to running air movers. **If it doesn't, the beachhead is
dead and we need to know in week two, not month six.**

This is the single most important experiment on the list, and it is cheap: buy or
borrow an air mover, run it, walk and talk.

### 4.4 Segments to abandon outright

Not "deprioritise" — **abandon**, because a funded direct clone already serves
them: insurance adjusters (FieldScribe AI), structural/AEC (InspectMind AI, YC
W24), marine survey (Worksmith Marine, launching now), tree service (Cleri),
moving (HomeSurvey.ai, Yembo), agronomy (Tellia).

Also note **QuoteIQ ($29.99/mo) is running a vertical-by-vertical SEO landgrab** —
publishing "top 10 software for [trade] 2026" for essentially every trade, with
voice-command AI estimating. Any home-services trade we pick, they are already
there or will be within a quarter.

---

## 5. GTM

### 5.1 What actually worked for comparable companies

- **Jobber:** one year in, **three customers**. First one came from a personal
  introduction. Now 300k+ pros.
- **QuoteIQ:** bootstrapped to **40,000 contractors** — because the founders
  already had 580k and 743k YouTube subscribers and built the product from their
  audience's comments. *Distribution was the product.*
- **ServiceTitan:** rode in the back of service vans to design a UI usable with
  greasy hands. First customer got six months of custom development.
- **Joist:** free mobile app → 115,000 contractors → 30,000 converted to paid.
  **This validates our 5-free-walks shape.**
- **Spectora:** owns a 4,700-member Facebook group. Not "posts in" — *owns*.

**The pattern: 0→100 in this market does not come from a channel. It comes from
about ten people you personally sit with.** Every single one of these companies
started that way.

### 5.2 Ranked channels

| # | Channel | Cost | Time to first customer | Note |
|---|---|---|---|---|
| 1 | **Ten ride-alongs** | $0 | 1–3 wks | The only thing that worked for all five comps |
| 2 | **App Store long tail** | $0 | 2–8 wks | Head terms unwinnable (Invoice Simple has 122,658 ratings). But "Estimate Maker for Contractors" ranks on 2,063 ratings and **hasn't updated since May 2023** |
| 3 | **Restoration Facebook groups + r/restoration** | $0 | 1–4 wks | Most community-vocal trade found |
| 4 | **Podcast guest sweep** | $0 | 2–6 wks | Dozens of shows at 1k–10k listeners, desperate for guests. ~15 appearances ≈ 45k targeted impressions at zero cost |
| 5 | **The legal explainer** (§3.2) | $0 | 4–8 wks | *"Can you legally record a customer on a job site? A state-by-state map."* Publish as a free tool, not a product page |
| 6 | **InterNACHI vendor program** | **$49/mo** | 1–3 wks | 27,253 inspectors; founder personally announces you; forum posting rights. Only if we keep inspection as segment two |
| 7 | **Equip Exposition**, Louisville, Oct 20–23 | **$30 badge** | Same week | 20,000 contractors. Booths sold out — *attend, don't exhibit*. Demo is 90 seconds and needs no booth |
| 8 | Creator rev-share | 20% of referred | 3–8 wks | Top channels are spoken for (Jamison→Jobber, Fullerton→LMN, Andes→his own CRM). Mid-tier only |
| 9 | Distributor partnerships | $0 | 6–18 mo | Real precedent, but every deal hinges on catalog/pricing integration we don't have. **Year 2** |
| ✗ | Franchise networks | — | — | **Closed.** All mandate their own software |
| ✗ | Product Hunt | — | — | Recent indie privacy-tool launches report 84 visitors / 0 sales |

### 5.3 ASO note

"Jefe" has **zero keyword value**, so the subtitle and keyword field carry 100%
of the ASO load. Target string clusters, not words: `contractor estimate`,
`estimate maker for contractors`, `home inspection report`, `work order`,
`punch list`. Note "voice estimate" already resolves to BuildWalk.

---

## 6. What to do, in order

**This week**

1. **Install Jobber and VoxTrade. Run a real walk through both.** Resolves the
   command-vs-narration question (§3.3) and tells us whether the price twin is
   any good. One hour, highest value on the list.
2. **The air-mover test** (§4.3). Buy or borrow one. This is the go/no-go on the
   entire beachhead recommendation.
3. **Rewrite the App Store subtitle and keywords** around privacy + uncapped,
   not around voice. Voice is now table stakes and the phrase is taken.

**Next two weeks**

4. Ten ride-alongs with restoration techs. Not interviews — ride-alongs.
5. Publish the state-by-state recording-consent explainer.
6. Buy two Equip Expo badges before the 10 Sept price rise ($30 → $60 → $120).

**Product, if the air-mover test passes**

7. **Numeric capture reliability.** Restoration documentation is *numbers spoken
   in sequence* — "north wall, 18 inches up, 42 percent, GPP 68." Generic
   transcription mangles these. Highest-value differentiator over a dictation app.
8. **Jobs as multi-day containers** — many walks, one job, documents accumulating
   per day. Our walk-reopen work (#223) is the right primitive; the container is
   missing.
9. **Photos attached to spoken observations.** Restoration documentation is
   photo-mandatory; carriers will not accept a voice-only artifact. This is the
   largest product gap.
10. **IICRC-shaped schemas** on the #234 seam: moisture log, scope in Xactimate
    line-item language, cause-of-loss narrative, work authorization.
11. **Reconsider the free tier.** 5 walks ≈ one restoration job. Great for
    forcing conversion, useless for evaluation. Consider 14 days unlimited.

**Résumé replacement** (§1.1):

> Built the only voice-to-document field app processing audio on-device, while
> competitors upload client recordings to third-party AI vendors.

---

## 7. What we could not verify

- **Whether Jobber Voice supports continuous narration** — the deciding question
  for our moat. Help centre 403s; no hands-on review exists.
- **Whether Housecall Pro's voice invoicing actually reached general
  availability** — the Sept 2025 press release says yes, but their own Fall 2025
  product-update page doesn't mention it.
- **VoxTrade's real pricing** — their site says £14.99/mo, Capterra says
  $9.99/mo flat. Unreconciled.
- **V2E's $150/mo** — posted on their site, but one search snippet describes them
  as "free while in private beta." Treat as posted-but-unproven.
- **Whether any of the six new App Store entrants actually work.** Worth 30
  minutes downloading all six.
- Facebook group member counts (blocked to automation) and Reddit rule text
  (403). Several cited counts are from a stale 2023 roundup.


---

# 8. Refit for a $1–5k side hustle (2026-08-09)

Isaac: *"Im not trying to create a massive business, just a side hustle and id
honestly be happy if I was making 1-2k mrr"* — later widened to **$1–5k**, with
*"the easiest path"* as the explicit optimisation target.

**Everything above optimised for winning a market. That was never the goal, and
I filled in an ambition rather than asking.** What follows replaces §3–§5.

## 8.1 The arithmetic that actually governs this

After Apple's 15% and roughly $0.18/walk in API cost, at ~20 walks/month:

| Price | Net per sub | Subs for $3k MRR |
|---|---|---|
| $14.99 | ~$9 | 333 |
| **$19.99** | **~$13** | **224** |
| $29.99 | ~$22 | 137 |
| $39.99 | ~$30 | 100 |

At this scale the bottleneck is **finding humans**, not earning margin. Price is
the only lever that moves the human count without any marketing spend.

## 8.2 Why the market analysis stops mattering

$1–5k MRR is **100–330 subscribers**. Against 692,777 landscaping businesses
alone, that is 0.03%. Every competitive dynamic in §2 — Jobber's 300k pros,
VoxTrade's price parity, six new App Store entrants — is a fight over share, and
we do not need share.

**Corollary: the §4 restoration pivot is withdrawn.** It optimised TAM and
whitespace, which is the right instinct for a company and the wrong one here. It
would cost a new domain, IICRC schemas, photo-mandatory workflows and an
air-mover noise risk, to reach a segment we would need 0.2% of. Not worth it.

## 8.3 What we honestly sell, and for how much

The feature comparison, stated plainly:

| | Jefe | VoxTrade £14.99 | QuoteIQ $29.99 | Spectora $149 |
|---|---|---|---|---|
| Voice → document | yes | yes | yes | yes (template-matched) |
| Estimates / invoices | yes | yes | yes | — |
| Payments | **no** | yes | yes | yes |
| Scheduling / CRM | **no** | no | yes | yes |
| Expense & profit tracking | **no** | yes | yes | — |
| Comment library / state templates | **no** | no | no | yes |
| On-device audio | **yes** | no | no | no |
| Uncapped walk length | **yes** | no (120 min) | no | no |
| Custom document types | **yes** | no | no | partial |

**We are differentiated, not more featured.** VoxTrade matches us at the same
price and adds payments. Against Spectora we are roughly 15% of the product.

**Therefore $19.99, not the $39 I first recommended.** That was over-confident:
it assumed a parity that does not exist, and it ignored that an App Store buyer
anchors on consumer pricing however the ROI reads. $19.99 clears the $20
psychological line, delivers **45% better unit economics than $14.99**, and is
honest about a tool that does one job.

Keep the ceiling open: ASC can raise the price for **new** subscribers while
grandfathering existing ones. Launch at $19.99; if churn is low after 60 days,
move new signups to $29.99.

## 8.4 The buyer, corrected

I pitched "home inspectors on Spectora." **That is a bad buyer** — Spectora
already ships voice, so the pitch is an add-on to something they have.

The real buyer is **a small operator with no real software**, who walks a job and
types it up at 9pm. They are not choosing between us and Spectora; they are
choosing between us and a notes app.

That buyer exists in every trade, which is what the widened trade catalog
(#312) now serves. **Market narrow, accept wide.**

## 8.5 The plan

1. **Price at $19.99** when the ASC product is created. One field, biggest lever.
2. **App Store organic is the centrepiece** — our buyer searches for a tool and
   has no vendor relationship to defend. Cheapest customer available, and it
   compounds unattended. Head terms are unwinnable (Invoice Simple has 122,658
   ratings); the long tail is not — "Estimate Maker for Contractors" ranks on
   2,063 ratings and has not shipped since May 2023.
3. **Subtitle and keywords carry the whole ASO load**, because "Jefe" has zero
   keyword value. Write them for privacy + uncapped, not for "voice estimates"
   (taken by BuildWalk).
4. **Retention over acquisition.** At 200 subs, 10% monthly churn means replacing
   20 people every month forever.
5. **InterNACHI ($49/mo) is demoted to a test**, not the centrepiece — good
   channel, but it points at buyers who already own tools.

## 8.6 The two product gaps that would move the price

- **Payments.** VoxTrade has it at the same price; we do not.
- **Corrections that stick.** The dominant complaint across *every* competitor's
  reviews is that the AI's numbers are wrong and the fix is forgotten. Nobody has
  solved it. Our editable-output work is aimed at the right wound; the missing
  half is **memory** of the correction. This is the one feature that would make
  us better rather than merely different.
