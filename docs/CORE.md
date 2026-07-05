# Sitewalk — Core Feature Set v1

> Companion to `design/BRIEF.md`. Principle: the product is **one loop — Walk → Paper → Sent** — and v1 is only what makes that loop bulletproof. Simple is a quality strategy, not a budget constraint: every feature below strengthens the loop; anything that doesn't is in "Not Yet."

## The quality bar (what "solid core" means, measurably)

| Promise | Target |
|---|---|
| Capture never loses a walk | 100% — crash, dead battery, no signal included |
| Tap → recording | < 2 s |
| DONE → finished document | < 8 s (no spinner; the transformation is the show) |
| Edits before send | median < 3 per document |
| DONE → SENT | < 60 s |

These five numbers are the product. Instrument them from day one (see Commitment 3).

---

## The six core features

### 1. Bulletproof capture
One tap from app launch to recording. Pause/resume instantly (client walks up mid-walk). Audio is persisted locally from the first second — transcription and extraction stream live when there's signal and defer gracefully when there isn't. Job sites have hostile connectivity; the rule is **capture never fails, processing can wait.**
Auto-captured metadata on every walk: timestamp, GPS, site/job. Cheap to build, and it's provenance — the trust layer for anything that becomes a record.

### 2. The live board
Extracted items tick onto the board *as they're spoken* (design screen 02). This is not decoration: it's proof-of-work (the operator sees it understood them) and in-the-moment correction — "scratch that," "make it four yards" (Murmur's multi-turn correction engine, carried over). Correcting during the walk is 10× cheaper than correcting at review.

### 3. Templates as the spine
A template = **extraction schema + document layout + trade vocabulary**, defined as data, not code. Ship three (landscaping estimate, property condition report, inspection/field report), hardcode none — trade #4 must be a file we add, not a feature branch. Template is chosen per business at onboarding (changeable per job). This is the "one engine, swappable outputs" strategy expressed in architecture.

### 4. Honest gaps, never confident guesses
Anything the engine didn't hear — a quantity, a price, a room — renders as a marked blank (`—— needs a number`), never a plausible hallucination. Tap it or say it to fill. The "<3 edits" promise lives here, and its inverse is the product's worst failure mode: a wrong price in a sent estimate. Unheard ≠ invented. Ever.

### 5. The paper
Review screen renders the actual document (design screen 04): every field inline-editable, PDF export pixel-identical to the preview, letterhead from a 2-minute business-profile onboarding (name, logo, license #, contact). Send = iOS share sheet (email/text/whatever they already use). No client portal, no CRM. The branded PDF **is** the marketing — every estimate sent is an ad to the recipient.

### 6. Jobs board + site memory
Walks attach to sites; documents attach to walks; simple chronological history per site (design screen 01). Two quiet payoffs baked into the data model:
- **Self-building price book:** repeat rates and line items auto-suggest from the operator's own past documents ("mulch installed — you charged $95/yd last time"). No management UI in v1 — just autofill with provenance. This is a retention moat that costs almost nothing now and is painful to retrofit.
- The seed of the "your notes become your logs" angle — without building a compliance product yet.

### 6½. Minimal photos (the one scope debate — recommendation: IN)
Two of three launch templates (condition reports, inspections) are evidence-based documents; the design already promises "say photo or tap." Scope ruthlessly: capture button on the walk screen, photo attaches to the item currently being spoken, shows as a `PHOTO ×2` chip, embeds in the PDF. **No** markup, gallery, albums, or cloud photo management. If that's still too much, it's the first thing in v1.1 — but if property management wins the beachhead, it jumps the queue before launch.

---

## Three engineering commitments (how it grows without quality loss)

1. **Local-first, sync later.** Every feature works in airplane mode except final send. Start single-user with iCloud sync/backup; defer accounts + backend until there's a paying cohort that needs them (B2B will eventually — teams, web review — but validation doesn't).
2. **One schema, many renderings.** A document is structured data; the preview, the PDF, and every future export (CSV, QuickBooks, email body) render from the same source. The PDF is never the source of truth.
3. **Instrument the promise.** Log edits-per-document (and which fields), DONE→SENT time, and template used — from the first TestFlight build. This is simultaneously the quality dashboard and the beachhead-picking instrument: the trade with the lowest edit count and fastest sends is the market telling us where to focus.

## Suggested stack posture (for dam to pressure-test)

On-device speech-to-text (modern iOS speech APIs are strong and free — protects margin at $30–60/mo price points and enables offline capture), cloud LLM for extraction/structuring (deferred when offline). Murmur's pipeline already splits along this seam.

---

## Not Yet (deliberate, revisit only after the loop wins)

| Cut from v1 | Why / when it earns entry |
|---|---|
| Teams & multi-user | Solo operator is the beachhead buyer; teams = after first paying cohort |
| Payments, e-sign | Jobber/Square territory; we're a layer, not a suite |
| Scheduling, dispatch, CRM | Same — export *into* what they use, don't replace it |
| Custom template builder | We author templates until ≥3 trades are proven |
| Integrations (QuickBooks, Jobber, CompanyCam) | After beachhead; CSV export is the cheap bridge |
| Photo markup / galleries | Photos stay minimal until a template demands more |
| Android / web app | Web link is a validation artifact, not the product |
| Anything labeled "AI" in the UI | Permanent cut — the mechanism stays invisible |

## Open questions (Isaac + dam)

1. Photos in v1 — minimal version as specced above? (Recommendation: yes.)
2. Local-first + iCloud vs. accounts/backend from day one? (Recommendation: local-first; revisit at first revenue.)
3. Which template do we personally dogfood first? Whoever we can walk a real site with this month wins.
