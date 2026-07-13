# Customizable Paperwork — design study

**Owner:** sac · **Needs dam's read on:** the Structure/Upload core questions (§7) · **Mockup:** `docs/design/letterhead-studio-mockup.html` (open in a browser; trade/template switchers + live document).

Let operators make the exported document *theirs* — brand it, decide what's in it, or bring their own document entirely. The walk fills it in.

---

## 1. Thinking

"Voice → paperwork" is good; **"voice → *your* paperwork"** is the wedge. Rivals hand back a generic export; we hand back the crew's exact branded form. This is also where the free/paid line lives naturally.

The trap is treating "customization" as one feature. It's **two axes with very different costs**, and conflating them is how you end up promising "upload anything" and shipping something that mangles a customer's bill.

- **STYLE** — *how the document looks* (letterhead, logo, color, fonts). A pure presentation layer over data the core already returns. App-side, sac, **no LLM**. Cheap and safe.
- **STRUCTURE** — *what the document contains* (which sections/fields; whole new doc types). This is where the LLM enters, because the model has to *fill* whatever structure exists.

**The one rule that makes structure tractable:** the LLM can only reliably fill a **named schema**. As long as the document is a set of named fields/sections — whether from our presets, a form the operator builds, or a schema *inferred from an upload and confirmed once* — the fill is a solved mapping. A raw uploaded PDF has **no schema** until something extracts one. So the hard part of "upload your own" isn't rendering it; it's **comprehending it**, which is an LLM capability (dam's core), costs tokens, and must never be trusted to guess a stranger's document live on every walk.

## 2. The spectrum

| | What the operator does | Owner | LLM difficulty | Tier |
|---|---|---|---|---|
| **A. Style presets** | logo / color / font / footer | sac, app-side | none | Free (Pro removes footer) |
| **B. Section/field editor** | toggle/reorder sections + add fields on *our* doc types | sac + light core | easy — target stays a named schema | Free basics / Pro custom fields |
| **C. New doc type** | define a doc from a form (fields = a schema) | sac + core | medium — still a named schema | Pro |
| **D. Upload arbitrary doc → LLM fills it** | drop in their own PDF/form | **dam / core** | hard — target unknown until inferred | **Premium** (token cost) |

## 3. The reliability principle (D, done right)

"Upload whatever you want" is **not** freeform magic. It's:

> **upload → Jefe infers the fields → operator confirms the mapping *once* → every future walk fills it automatically.**

The one-time human-confirmed schema turns D back into B (a solved mapping). Without the confirm step, we'd trust the LLM to correctly read a stranger's document *every walk* — which will silently produce wrong paperwork, the worst failure for this product. The confirm-once pattern also localizes the expensive comprehension pass to setup, not every send.

## 4. Monetization

Three clean tiers, and the cost structure motivates them:

- **Free** — presets + basic branding (logo/color/font) + the **"Prepared with Jefe"** footer.
- **Pro** — remove the footer, custom fields, extra fonts, all presets.
- **Premium** — **upload your own document.** Justified by real marginal cost: document comprehension + mapping burns tokens per uploaded template. This is the first feature whose price is defensibly tied to our unit economics.

## 5. Architecture

**Style (A).** The document already renders from shared components (`Letterhead` → `DocRowView` → `TotalRow` → `PDFPageView` → `ImageRenderer` → PDF). Today those hardcode the theme; the change is to **thread a `Branding` object** (logo, accentHex, fontKey, contact, showWatermark) through them, defaulting to today's theme so the demo/gallery path is untouched. `DocumentPDF.render` already takes `biz/bizSub/docDate` — this generalizes it. *Known tradeoff:* the pipeline **rasterizes** the page (ImageRenderer, scale 3) — fine for logos/uploaded backgrounds; the con is non-selectable PDF text. Acceptable for v1; vector-text drawing is a later isolated swap.

**Structure (B/C).** A document type = an ordered list of **named sections**. Two kinds of section matter:
- *Static* (e.g. a fixed "Terms & deposit" block, signature line) — app-side, no LLM.
- *LLM-filled* (e.g. a custom "HOA approval #" field) — the field name must reach the **document-build prompt** so the walk populates it. This is the one place B touches the core: `buildDocument(kind:)` needs to know the active schema (including custom fields), not just a fixed template.

**Upload (D).** Render the uploaded PDF's first page (PDFKit → image) as the page **background**, lay the walk data into a positioned **content band** — which fits the existing rasterize-to-PDF pipeline exactly. The comprehension pass (infer fields) + the fill (map walk data → confirmed fields) are the core/LLM work.

## 6. Data model & flow

- **`DocumentTemplate`** (Codable, UserDefaults JSON like `BusinessProfile`, with a `schemaVersion` migration seam): `presetKey`, `logoFilename?`, `accentHex`, `fontFamilyKey`, `contact{…}`, `showWatermark`, `uploaded{fileFilename?, contentInsets}?`. Binary assets (logo, uploaded PDF) live in the app container, referenced by filename.
- **`DocumentSchema`** per doc type: ordered `[Section{key, kind: static|filled, label, customFieldSpec?}]`. For B/C this is authored; for D it's inferred-then-confirmed.
- **Where in the flow:** a **Letterhead Studio** and a **Document Builder** sheet reached from the **board header** (same pattern as the Vocabulary sheet, `BoardView.swift:46/181`) — setup tools, not walk-time steps, so onboarding stays minimal. **Live preview** inside each editor (reuse `DocumentSheet`). At **export (`ReviewView`)**, a small "Letterhead: *Acme* ▸" affordance to preview/switch before Send.

## 7. Open questions for dam (the core half)

1. **Document comprehension (D).** Can the harness reliably infer a field schema from an uploaded PDF/image (one-time, human-confirmed)? What's the token cost per template, and does it fit a Premium price? This gates D entirely.
2. **Schema into the fill (B/C).** `buildDocument(kind:)` is engine-keyed today. To support custom fields / new doc types, the fill needs the **active schema** passed in (or stored core-side). Do we thread a schema through the FFI call, or does the core own doc-type definitions? This is the main boundary decision.
3. **Doc-number minting for custom types.** New doc types need numbering (EST-/INV-/…); today that's template-bound. Core or app?
4. **Extraction awareness.** When an operator adds a filled custom field, should it influence the *walk-time* extraction prompt, or only the finish-time document build? (Recommend build-time only for v1 — keep the live board terse.)

## 8. Phasing

- **v1 (mostly sac, app-side):** Style (A) fully + Structure basics (B) — static section toggle/reorder + the shared body. Ships the "make it yours" value immediately.
- **v2:** LLM-filled custom fields (B) + new doc types (C) — needs the §7.2 schema decision + a small core change.
- **v3 / north-star (dam-led):** Upload (D) via infer→confirm-once, gated Premium — pending §7.1.

## 9. Boundary

Style + Structure-basics are sac's to build now with no core dependency. Everything LLM-touching (filled custom fields, doc comprehension) is a **joint** decision — which is why this doc exists before code. Reactions in-line, please; I'll fold them into a build plan.
