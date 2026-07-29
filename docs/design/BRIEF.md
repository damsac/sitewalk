# Jefe — Design Brief v1

> **Named Jefe** (CANON #202, 2026-07-14). The working name "Sitewalk" collided
> with at least seven products including a direct competitor; the repo is still
> `damsac/sitewalk` and that is deliberate — renaming it buys nothing.

## What this is

A voice-first field app for small operators — landscapers, property managers, inspectors — that turns a spoken walk-and-talk into their **finished paperwork**: an estimate, a condition report, a field report. Ready to send, not a transcript.

Built on Murmur's voice→structured-data engine (~70% reuse). iOS-native (SwiftUI), with a shareable web mockup used for design + operator validation.

**One engine, swappable output templates.** The pipeline is identical for every trade; only the document at the end changes. The UI must make that template switch feel native to each trade, not generic.

## The design thesis

Field software today sits at two poles, and both are wrong for us:

- **The market dialect** (Jobber, CompanyCam, SafetyCulture): utilitarian, trusted by trades — but visually crude. Bootstrap-blue, cramped forms, zero craft.
- **The craft pole** (Flighty, Halide, Things): genuinely designed — but precious, consumer, indoor.

The gap we own: **a professional field instrument.** The design vocabulary of surveying equipment, airport departure boards, carbon-copy work orders, and DIN-labeled tool cases — executed with Flighty-level craft. It should feel like a Milwaukee tool: rugged, precise, obviously built for work. A landscaper should feel *respected* by it, not marketed to.

Flighty's core lesson applies directly ([Behind the Design](https://developer.apple.com/news/?id=970ncww4)): dense data made calm by borrowing a visual language the audience already trusts. Theirs was airport signage. **Ours is the paperwork itself** — the estimate, the inspection form, the job ticket. The generated document isn't displayed in an app card; it *looks like a document*, because "that's the paperwork I'd have typed tonight" is the entire pitch.

## Physical constraints (non-negotiable, and where generic AI design always fails)

1. **Sunlight legibility.** Used outdoors at noon. Light/"paper" mode is the default (dark UIs wash out in direct sun). True ink-on-paper contrast (AAA) for body copy; no mid-gray-on-white text.
   **Known exception, not yet fixed:** deep amber `#9A6A00` on paper is ~4.4:1 —
   AA for large text, short of the AAA claimed here — so it must not be used
   below 14pt. `ink35` is weaker still (~2.4:1) and is doing real work in job
   cards. Either the tokens change or this line does; today the line is
   aspirational for those two. Dark mode is the secondary theme, not the identity.
2. **Gloves and one hand.** Primary actions ≥ 56pt, bottom-anchored in the thumb zone. The mic control is the biggest thing on the screen.
3. **Glanceable at arm's length.** User is walking with phone at hip. Recording state must be unmistakable from 3 feet: full-screen state change, not a small red dot. One line per captured item — airport-board density discipline.
4. **Interruptible.** Client walks up mid-recording → pause is instant and obvious; state never ambiguous; nothing lost.

## Aesthetic direction: "Field Instrument"

**Committed direction, not a mood board.** Every choice below is a decision the mockup follows.

### Typography (where "not another AI app" is won or lost)
- **Banned:** Inter, Roboto, Arial, Space Grotesk, SF-default-everywhere.
- **UI type: Barlow** (incl. Semi Condensed for data rows) — drawn from California highway signage; utilitarian-signage DNA that matches the concept, wide weight range, free.
- **Data/metadata: IBM Plex Mono** — timestamps, GPS, site IDs, line-item quantities, prices. Stamped, machine-logged character; tabular numerals for money.
- **Generated documents: Source Serif 4** headings on the letterhead + Plex Mono line items — the output must read as *finished paperwork*, distinct from the app chrome around it.

### Color
- Base: paper white `#FAFAF7` / ink black `#141412` — document heritage, maximum sun contrast.
- **One accent: hard-hat gold `#FFBB26`** on deep amber `#9A6A00`, ink on gold
  (`#141412`). Used only for the live/recording state and the ONE primary action
  per screen. Never decorative.
  - *This replaces the safety orange `#E8531F` this brief originally specified.*
    The app shipped gold and the app is right: **black-on-amber is caution-label
    language** — the contrast pairing every piece of real site equipment already
    uses — and it holds up in direct sun where a mid-value orange goes muddy.
    `#E8531F` on paper cannot carry ink-black text at all, which would have
    forced white-on-orange and lost the whole read. The brief is what changed.
  - **"One accent" means one FILL per screen.** The implementation drifted here:
    amber ended up on START WALK *and* MY BUSINESS, the active tab, coach marks,
    section counters, SHOW ALL, ADD JOB and the FILE chip at once. Everything
    except the live state and the primary action is ink.
- Status = job-site tag language: red tag (issue), yellow tag (follow-up), green tag (good). Muted, ink-adjacent versions — not candy.
- **Banned:** purple (AI cliché *and* Murmur's consumer skin — the engine carries over, the skin does not), gradients, glassmorphism blobs, sparkle/✨ iconography, chat bubbles.

### Texture & detail
- Hairline rules and form ruling like a carbon-copy work order; section headers as small-caps stamped labels (`SITE`, `FINDINGS`, `LINE ITEMS`).
- A metadata strip on every capture — date, time, GPS, site — set in mono, like a field-log header. This is also a trust feature: inspection records need provenance.
- Document preview rendered as actual paper: sheet edge, letterhead with the operator's business name, doc number (`EST-0047`).

### Motion
- Mechanical and quick (150–250ms), no bounce, no playfulness.
- **One showpiece:** the transformation. Spoken words → extracted items ticking onto the board → document fields filling in. This animation *is* the product demo; everything else stays still.

### iOS 26 note (for the native build)
Adopt Liquid Glass only where the HIG puts it — the floating navigation/control layer above content ([HIG](https://developer.apple.com/documentation/technologyoverviews/liquid-glass)). Content layer (the board, the documents) stays paper. Character lives in content typography/color; glass is chrome, not identity.

## The mockup: 4 screens, trade-switchable

Trade switcher (Landscaping / Property Mgmt / Inspection) swaps template, terminology, and sample content — same bones.

1. **Capture** — full-screen recording state: waveform, running transcript, extracted items ticking onto the board live, metadata strip, giant pause/done.
2. **The transformation** — the processing beat where speech becomes a document. Short, confident, showpiece.
3. **Document review** — the finished paper (estimate / condition report), inline-editable line items, one fix, then **Send**.
4. **Jobs board** — home screen: today's sites, one line each, status tags. Airport-board discipline.

Canned walk-through (landscaping): *"front beds need mulch, about three yards… trim the four boxwoods… irrigation zone 2 head is broken, replace it… quote the whole thing around twelve hundred."*

## Anti-goals (the "another AI app" checklist — if any appear, revise)

Chat interface as primary UX · purple/gradient hero · glassy floating orbs · ✨ sparkles meaning "AI" · Inter/Space Grotesk · dark-hero-with-glow marketing look · rounded-corner sameness with no ruling or structure · "magic" copywriting. The word "AI" ideally appears nowhere in the UI — the user talks, paperwork comes out; the mechanism is invisible.
