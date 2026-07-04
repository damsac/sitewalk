# Sitewalk — Design Brief v1

> Working name only ("Sitewalk" = the moment of use: a site walk). Swap freely; nothing below depends on it.

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

1. **Sunlight legibility.** Used outdoors at noon. Light/"paper" mode is the default (dark UIs wash out in direct sun). True ink-on-paper contrast (AAA); no mid-gray-on-white text. Dark mode is the secondary theme, not the identity.
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
- **One accent: safety orange `#E8531F`** (surveyor's flagging tape, hi-vis vests). Used only for the live/recording state and primary actions. Never decorative.
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
