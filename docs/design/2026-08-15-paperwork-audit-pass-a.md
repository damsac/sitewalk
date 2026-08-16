# Paperwork audit — Pass A findings (structure)

Schema-only pass over the seven built-ins in `domain.rs`, against the reference
checklists in `2026-08-15-paperwork-audit-plan.md`. No API calls, no rendering:
this asks only whether the document DECLARES the parts its trade expects.

## What each one is today

| Kind | Sections | Priced | Total |
|---|---|---|---|
| Estimate | Prepared For (manual) · Scope of Work | yes | sum → total |
| Invoice | Prepared For (manual) · Work Performed | yes | sum → amount_due |
| Work order | Assignment (crew, schedule) · Site Notes (access, safety) · Instructions | no | sum → total |
| Condition report | Summary | no | sum → total |
| Move-out report | Summary | **yes** | sum → deposit_deduction |
| Inspection report | Summary | no | sum |
| Report | Summary | no | sum → total |

Plus, on every kind: `DocumentLayout.termsText = ""` and `showSignature =
false` **by default**. Out of the box, no document carries terms or an
acceptance line unless the operator goes and turns them on.

## P0 — the document is not the document without these

**Move-out has no deposit arithmetic.** It is priced and totals its deductions,
and that is half the sentence. The document exists to say *deposit held −
deductions = balance returned*; a tenant handed "$185 in deductions" cannot
tell what they are owed, and in most states this is the document that runs
against a statutory deadline. Needs a `deposit_held` field (manual — nothing in
a walk knows it) and a total that does the subtraction. This is the single
worst gap found.

**No tax, anywhere.** No kind declares a tax rate or line. Isaac's EST-0001
showed "Tax $35" only because he happened to say it out loud and it became an
item. Every priced kind needs tax as structure, not as something the operator
remembers to speak.

**Invoice has no due date and no payment terms.** It has an issue date (the doc
date) and an AMOUNT DUE total, and no way to say when, to whom, or how. "Net
30", remit-to, and accepted payment methods are what makes an invoice
collectable rather than a receipt for work already done.

## P1 — professionally expected, and their absence shows

**Estimate: no expiry and no acceptance.** An offer with no validity window is
one a client can accept at last year's prices, and with `showSignature` off by
default there is nowhere to accept it. Both are one field each.

**Photographs never reach a PDF.** Condition, inspection and move-out are
evidence documents; photos have been captured since Plan 11 and still print
nowhere. For a condition report this is close to P0 — it is the whole proof.

**Client is one free-text blob.** `prepared_for` is a single manual field, so
an invoice cannot carry a bill-to address and an estimate cannot carry a
client's phone. Splitting it costs nothing structurally.

## P2 — real, but nobody is harmed today

- **Work order has no completion sign-off** — no place to record it was done,
  by whom, when.
- **Inspection has no scope/limitations** — the disclaimer paragraph is
  standard in the trade and legally load-bearing.
- **Condition and inspection have no per-area structure** — one prose summary
  where the trade expects room-by-room or system-by-system.
- **Unpriced kinds still declare `("sum", …)`** — vestigial since the total row
  learned to hide itself. Harmless, but it means the schema says a thing the
  document does not do.

## What Pass A cannot tell us

Whether any of this fills correctly, and whether it prints. Two specific
follow-ups for Pass B/C:

1. `move_out` and `condition` are `trade_key: Some("property")` while the other
   five are universal — worth confirming a property-template walk actually
   resolves them (#331 changed this ranking).
2. The `client` block is `fill: manual` on purpose, but nothing pre-fills it
   from `Job.client`/`Job.site`, which exist and sync and are never collected.
   That is why every estimate is typed by hand.

## Recommended order

Deposit arithmetic, then tax, then invoice due-date/terms. Those three are the
difference between a document a trade can use and one it has to redo by hand —
and all three are schema-shaped, which is the cheapest kind of fix here: the
#321 seeding upgrade delivers them to existing installs with no migration.

---

## Decisions on the P0 fixes (Isaac, 2026-08-15)

**Tax is an optional business-profile setting.** *"There should just be an
option to add tax in the business profile. If it does get set up, then it gets
pulled into the paperwork, if not then it doesn't."*

So tax is absent by default and appears only once configured — which is the
honest posture and matches the app's existing rule that a blank is a real
state, not a defect. Two consequences worth building deliberately: a document
produced BEFORE tax was configured must not retroactively grow a tax line
(documents are snapshots, D7), and the rate has to be stored on the document at
build time rather than read live at render.

**`deposit_held` is manual.** Typed at review, like PREPARED FOR. Nothing in a
walk knows it, it is entered once per move-out, and it is the number a tenant
disputes — so it should be deliberate rather than inferred.

**Invoice terms are manual and off by default.** *"Should be manually turned on
and filled in by the user."* Same shape as `DocumentLayout.termsText` /
`showSignature` today: the operator turns it on, writes their own words, and it
persists. No "Net 30" default — a payment term the operator did not choose is a
promise the app made on their behalf.

**Remit-to stays in the letterhead** for now (Isaac: *"letterhead is ok but
whatever you think"*). Reasoning: `Branding.contactLine` already exists and is
operator-controlled, testers are being paid by cheque and transfer arranged
off-document, and a separate remit-to block would be a second place to maintain
the same facts. Revisit if a tester actually asks for card payment or an ACH
block — that is the signal that it needs its own structure.

### Revised, same day: tax is a manual field, not a profile setting

Isaac: *"let's just leave tax as a manual field the user has the option to
input — that keeps it simple."* And on terms: *"the user needs to turn that on
in settings or wherever, and when they turn it on they have the option to
change the wording."*

So tax drops the profile setting, the stored rate, and the snapshot problem
along with it: the operator types an AMOUNT on the document, once, where they
can see the subtotal it applies to. No rate, no arithmetic the app can get
wrong, nothing to configure before the first invoice. If computed tax is ever
wanted, a rate on the profile is a strict upgrade from here and this field is
where it would land.

**The one thing this forces a decision on: a manual tax field is only worth
building if it reaches the TOTAL.** Today a `fill: manual` field renders inside
a section, and sections sit above the lines — a tax that prints in a paragraph
and is not added up is worse than no tax line, because it looks paid for and
isn't counted. Two ways:

1. **Tax is a manual field with a total-participating slot** — renders as a row
   directly above TOTAL, included in the sum. Gives the trade-standard
   Subtotal / Tax / Total shape. Small, and the schema already distinguishes
   `total_kind`.
2. **Tax is just a line the operator adds** — ADD LINE exists and already does
   this; EST-0001 carried "Tax $35" that way, summed correctly, with no work at
   all. Costs nothing, and gives no subtotal/tax labelling.

Recommendation: (1), because an invoice that shows a total without breaking out
tax is the version a bookkeeper questions — but (2) is genuinely free and worth
knowing we already have.

**Decided: option 1.** The tax field gets a total-participating slot — a row
directly above TOTAL, included in the sum, giving Subtotal / Tax / Total. So
the build is: a manual `tax` field on the priced kinds, a renderer that draws
it between the last line and the total, and a total that adds it. The amount is
typed; nothing computes a rate.

Two things for whoever builds it. The app currently sums `amount_cents` in
`DocumentModel.totalValue` while core computes its own total (#337) — tax must
land in BOTH or they will disagree on the same document, which is exactly the
duplication #337 was opened to end; folding the app's sum into core's number is
the better fix and this is the moment it becomes urgent. And an unfilled tax
field prints nothing at all: no "TAX ——", no zero row. A blank is a real state
here, the same as every other optional field.

---

## Pass B, first finding — and a correction

`FFIDocumentPayload` carries `static_total_cents` (null for every built-in) and
NOT the `total_cents` core started computing in #337. So core computes a
document total that nothing reads: the app still sums `amount_cents` itself in
`DocumentModel.totalValue`, and core's number is used only by the board.

**The correction:** I wrote above that folding the app's sum into core's number
is the better fix. That is wrong, and the reason matters.

The two numbers are not duplicates of each other. Core's total is a SNAPSHOT,
computed at build and frozen into the artifact. The app's sum is LIVE — the
operator can retitle a line, change an amount or delete a row at review, before
anything is sent, and the total on screen has to follow. Replace the app's sum
with core's and an edited estimate would display a total that contradicts its
own visible lines, which is the worst class of defect this document can carry.

So both stay, with distinct jobs:

- **core's `total_cents`** — what this document was worth when it was built.
  Feeds the board, and anything counting money across walks.
- **`DocumentModel.totalValue`** — what the page in front of you adds up to
  right now, edits included.

They agree at build and are allowed to diverge afterwards, because that
divergence IS the operator's unsent edit.

**What this means for tax:** it has to be included in both, in the same way, or
the board and the PDF will disagree about the same document — one counting tax
and one not. That is the actual constraint tax imposes, rather than the
deduplication I claimed.

---

## Terms belong to the document type, not to the letterhead

Turning terms on for the audit renders exposed that `DocumentLayout.termsText`
is GLOBAL — one text across all seven kinds. The estimate wants "50% deposit to
schedule · balance on completion"; the invoice wants "Net 30, 1.5% monthly".
Both are correct and neither belongs on the other document, and an operator can
only have one.

Isaac, 2026-08-16: *"maybe we just make the terms thing a custom thing that
people can add when they edit documents? In the Structure section?"*

That is right, and the mechanism is already built. `fill: "static"` — a schema
field whose value is fixed text printed on every document of that kind — has
been in `VALID_FILL_KINDS` since Plan 19, is handled in the fill pass
(`document.rs:697`), and its own test fixture value is "Valid for 30 days."
Nothing uses it, the same way nothing used `fill: "manual"` until PREPARED FOR
made it load-bearing.

So terms are not a new feature. They are a static field on a schema, and the
only missing piece is the **Document Builder** (#234) — the editor that lets an
operator change a document's structure at all.

**Recommendation: fold terms into #234's scope rather than build a second
mechanism now.** Concretely:

- Leave `DocumentLayout.termsText` alone. One global text beats none, and it
  works today.
- When the Builder ships, a terms field is a static field the operator adds to
  whichever kinds want one — with different wording per kind, by construction.
- At that point `termsText` migrates into the estimate and invoice schemas as
  static fields and the layout setting retires.

Building per-kind terms into the Letterhead Studio first would mean shipping a
second, weaker version of a mechanism that already exists — and then deleting
it when the Builder lands.

**One thing this also resolves:** the new VALIDITY block and the global terms
currently overlap, because "quote valid 30 days" is the natural thing to write
in both. Once terms are per-kind and authored on the estimate, the operator can
say it once, wherever they prefer.
