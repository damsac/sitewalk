# Design: Notes-First Output — capture becomes notes; documents become actions

**Date:** 2026-07-09 · **Status:** Proposal for dam + sac to react to (not a plan, not code)
**Owner tags:** core mechanics = **dam**, flow/visuals = **sac**, contested = **joint**
**Decision by Isaac (product):** the first-level output of a walk is **notes**, with clear
action buttons to turn them into an invoice, estimate, condition report, inspection report,
or a plain export. Documents stop being the automatic finish output and become explicit
actions off the notes.

---

## The shift

**Today:** walk → live board → **finished document** (estimate/report), auto-produced at
DONE → send. The document is the primary, automatic output.

**Proposed:** walk → **Notes** (primary output, always produced) → a row of **action
buttons** (Estimate · Invoice · Condition Report · Inspection Report · Export · Follow-ups)
→ the chosen artifact. Documents are transforms *off* the notes, chosen deliberately.

Reference: Granola / Copilot — after a meeting you get clean, trustworthy notes first,
then act on them.

## Motivation *(joint)*

1. **Trust ladder.** Jumping straight to a priced invoice is a leap of faith; one wrong
   number craters trust. Notes-first lets the operator confirm "it heard my walk" *before*
   committing to "here's the money document." Verify capture, then monetize.
2. **Not every walk is one document.** A single landscaping walk can yield estimate
   line-items *plus* a mulch-reorder reminder *plus* a crew note. That's not one document —
   it's notes that fan out into different actions.
3. **The engine already produces the notes.** `finish()` extracts structured items,
   contacts, and a summary today — that *is* the notes content. Notes are a lighter
   transform than the document; documents become a second, explicit transform.
4. **A pattern users already understand** (the Granola stickiness noted in the naming
   discussion) — lower learning curve.

## The reframe that protects the wedge *(joint — read this before worrying it's a pivot to vitamin)*

The competitive teardown (`docs/research/2026-07-08-sitewalk-cluster-competitive.md`) found
**every** rival stops at "clean notes of what I saw," with at most a generic PDF export.
The risk of notes-first done lazily is that we become one of them.

We don't, because **the action-button row IS the differentiation, made visible.** Rivals:
notes + generic export. Us: notes + buttons that each produce a real, finished,
trade-specific document — an estimate *with prices*, a condition report *with deductions*,
an invoice. Each button is "the thing you'd have typed tonight." The moat isn't hidden
behind notes; it's displayed on top of them. Notes build the trust; the buttons are the
payoff. **Guardrail:** the document actions must stay prominent and feel magical — never a
buried "…more" menu. If the notes screen ever reads as "a notes app that also exports," we
have lost the thread.

## The Notes screen *(sac owns visuals)*

Field Instrument language, same as the rest of the app. Structure:

- **Summary line** — one glanceable sentence: `ESTIMATE WALK · 1418 ALDER · 5 ITEMS · ~$1,200`.
- **Grouped captured content** — the extracted items, grouped by kind rather than a flat
  list: work/observations, measurements & quantities, flagged issues (red/yellow tags),
  people/contacts, follow-ups.
- **Photos** — inline, pinned to their items (Plan 11 grouping).
- **Transcript** — collapsed by default, expandable ("show what I heard").
- **Action bar** — the document buttons + export (below).

The notes are the durable record of the walk; documents are generated *from* them.

## The action taxonomy *(joint — sac drafts the per-trade sets)*

**Universal (every walk):**
- **Export Notes** — plain text (Granola-style copy/paste) + branded PDF + email/share.
- **Follow-ups → Reminders** — the reminder/task items become actionable (in-app list, or
  export to Apple Reminders — open question).

**Documents (trade-curated prominence; all remain reachable):**
- **Landscape:** Estimate (quote, pre-work) · Invoice (bill, post-work) · Work Order (crew).
- **Property Mgmt:** Condition Report · Move-out Report.
- **Inspection:** Inspection Report.

Note the estimate/invoice distinction Isaac named: an **estimate** is a quote *before* work,
an **invoice** is a bill *after* — different documents, both wanted; they span the job
lifecycle. The trade (from the business profile) curates which buttons lead; the notes
themselves are trade-agnostic capture.

## Architecture / seam *(dam)*

- **`finish()` produces NOTES, not a document.** It already returns items + summary — that's
  the notes. Drop the automatic document build at finish.
- **Each document action is a transform off the notes.** Open question whether that's a
  second LLM pass (pricing, section-mapping) or a deterministic re-render from the already-
  structured items. dam's call; affects latency + cost per action.
- **Notes become the durable session artifact; documents are derived artifacts** off a note
  (regenerable; multiple documents per walk — an estimate *and* a work order from one walk).
- **Doc-number minting moves to document-generation time** (was at finish; core mints them).

## Phased plan

1. **Insert the Notes screen** as the post-walk destination. The current `ReviewView`
   (the paper document) becomes the output of the "Estimate/Report" action, not the direct
   finish screen. Action bar shows the trade's document buttons + Export. (Mostly sac/UI —
   the notes content already exists in what `finish()` returns.)
2. **Full action taxonomy** — estimate vs invoice vs work order, condition/inspection, the
   follow-ups surface, notes export (text + PDF).
3. **Notes as durable record** — wire the board's walk-history rows (onboarding PR) to saved
   notes; multiple documents per note; regenerate.

## Open questions

| # | Question | Owner |
|---|----------|-------|
| 1 | `finish()` = notes only, or still eager-produce the primary document? (recommend notes only) | dam |
| 2 | Document transform: a second LLM pass, or deterministic re-render from the structured items? | dam |
| 3 | Per-trade document button sets — final taxonomy + which leads. sac drafts; dam confirms template mechanics. | joint |
| 4 | Notes export format — plain text (copy/paste) *and* branded PDF? | sac |
| 5 | Follow-ups: in-app list vs export to Apple Reminders vs both. | joint |
| 6 | Persistence: does the core store the "note" as the session artifact and documents as derived artifacts (schema)? | dam |
| 7 | Regeneration: if the operator edits notes, do generated documents update, or are they snapshots? | joint |

## Explicitly out of scope

- **Integrations** (Jobber / QuickBooks / CompanyCam push) — later, after validation.
- **New document *designs*** (Invoice, Work Order layouts) — sac, when Phase 2 lands; the
  Estimate and Condition/Inspection Report designs already exist.
- **The onboarding/profile + board-history work** (separate branch, in flight) — notes-first
  builds on it (walk-history rows link to notes) but doesn't block it.
