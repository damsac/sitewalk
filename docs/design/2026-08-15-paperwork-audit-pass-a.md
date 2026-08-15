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
