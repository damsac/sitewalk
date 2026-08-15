# Paperwork audit — do the seven documents hold up?

Isaac, 2026-08-15: *"audit all of the different types of paperwork we have to
make sure they fit what they're supposed to. Do they have all critical parts
based on industry standard? Are they missing anything? Do they fill in
correctly from the walk?"*

Three questions, and they fail in three different places, so the audit runs as
three passes rather than one read-through.

| Pass | Question | Where the answer lives | Cost |
|---|---|---|---|
| **A · Structure** | Does the schema declare the parts the trade expects? | `domain.rs` built-ins | hours, no API |
| **B · Render** | Does a real document PRINT those parts, on one page, to a client? | `DocumentPDF` + a generated PDF per kind | hours, no API |
| **C · Fill** | Do the parts fill from a walk that states them — and stay blank when it doesn't? | `document_smoke.rs`, real API, 3× per kind | ~$2, half a day |

They run in that order because each one invalidates the next: there is no point
testing whether a field fills if the schema never declares it, and no point
checking fill fidelity on a field that never prints.

---

## Pass A — structure against the trade's expectations

For each of the seven built-ins, compare the schema's sections and fields
against what the document is understood to contain in the trade. The reference
below is the checklist; the audit produces a have/missing table per kind.

**Estimate** — an offer to a named person: business identity and licence,
client name and address, date, an expiry ("valid 30 days"), itemized scope with
quantities, subtotal, tax, total, deposit or payment schedule, terms, and an
acceptance line. *Suspected missing today: expiry, tax, deposit terms, an
acceptance signature by default.*

**Invoice** — a demand for payment: invoice number, issue date **and due date**,
bill-to, remit-to and accepted payment methods, itemized work, subtotal, tax,
total, payment terms (Net 30), late-fee policy, PO reference where one exists.
*Suspected missing: due date, payment terms, remit-to, late fee, PO.*

**Work order** — the crew's instruction sheet: WO number, date, crew, schedule,
site address and access, safety, the task list, materials, and a completion
sign-off. *Assignment, site notes and instructions exist as of #334; suspected
missing: materials called out separately, and a completion/sign-off block.*

**Condition report** — evidence of state at a moment: property and unit, date,
who walked it, per-area observations, photographs, overall condition, and
signatures. *Suspected missing: photographs (deferred since Plan 11), per-area
grouping, signature block.*

**Inspection report** — findings: the standard being inspected against, scope
and limitations, findings by severity, photographs, recommendations, inspector
identity. *Suspected missing: limitations/disclaimer, severity grouping,
inspector identity.*

**Move-out report** — the deposit document, and the one with a legal clock:
unit, tenant, move-out date, deductions itemized **with cause**, damage
separated from normal wear, and the deposit arithmetic — *deposit held −
deductions = balance returned*. *Suspected missing: the arithmetic. Today the
total is the deduction sum, which is half of the sentence a tenant needs.*

**Report** — the general record; least standardized, audited last.

Cross-cutting, all seven: business licence number, tax handling, and whether
the letterhead carries phone/email (Branding has a contact line — is it on by
default?).

---

## Pass B — what actually prints

Generate one PDF per kind from a single deliberately rich transcript, and read
them side by side. Three specific things to check, beyond the checklist:

**Overflow.** `DocumentPDF.render` draws **one US-Letter page** and never
paginates. A twenty-line estimate is an ordinary walk. If it clips, that is the
most severe defect available — a document that silently loses lines is worse
than one that lacks a field — and nothing in the suite would catch it today.
Test with a 25-item walk.

**Empty-block behaviour.** Blocks with no value are omitted (correct), but the
audit should confirm no document renders as a letterhead over a total with
nothing between.

**The totals row.** After #337 core computes a total; the app still sums
`amount_cents` itself in `DocumentModel.totalValue`. Two implementations of one
number is a defect waiting for a rounding difference — check they agree, and
plan to delete one.

---

## Pass C — does it fill from the walk

Extend `document_smoke.rs` from three tests to seven, one per kind, each with a
transcript written to state every field the schema declares — and to
deliberately omit two, so the audit tests silence as well as speech.

Each kind asserts, on **three consecutive runs** (variance is the whole reason
the smoke tests exist — the work-order instructions block came back empty two
runs in three before its brief was rewritten):

1. Every field the transcript states arrives filled.
2. Every field it doesn't state arrives as a **gap**, not an invention.
3. Money appears only where the schema is priced.
4. No crew name reaches a client document; no price reaches a crew document.
5. The prose adds to the lines rather than restating them.

---

## Deliverable

A findings table — kind × part × have/missing/broken — plus a prioritized list
split three ways, because the fixes live in different places and carry
different risk:

- **Schema gaps** (`domain.rs` + a compose brief) — cheap, additive, seeded by
  the #321 upgrade path.
- **Render gaps** (`DocumentPDF`) — pagination is the one with real risk.
- **Fill defects** (prompts) — cheapest to change, hardest to keep fixed;
  every one needs 3× real-API confirmation.

## Non-goals

Not this pass: new document types, the Document Builder, per-trade variants, or
anything requiring a document status past SENT. If the audit finds that
estimates need acceptance tracking, that is a finding, not a fix.

## Risk worth stating up front

The likeliest outcome is that Pass A returns a long list and most of it is real
— tax, due dates, expiry and deposit arithmetic are not nice-to-haves, they are
what makes a document usable in the trade. Sequencing matters more than volume:
**a document that clips lines and a move-out report that doesn't do the deposit
math are worse than five missing fields**, and should land first.
