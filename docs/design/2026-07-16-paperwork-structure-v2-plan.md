# Paperwork Structure v2 — build plan

**Owner split:** core `DocumentSchema` seam = dam · authoring UI + rendering = sac · **Status:** folds dam's #207 §7 answers into the concrete plan the ROADMAP was gated on. **Timing:** dam lands the *seam* in his pre-departure window; sac builds the *UI* over his month away.

Operators define what's *in* their documents — reorder/rename sections, add fields, spin up doc types we don't ship — and the walk fills them. This is the customization moat ("voice → *your exact document*"). It's the STRUCTURE axis of #207; STYLE (Letterhead Studio) already shipped.

---

## 1. Thinking

The design is settled — dam answered §7 on #207. What's missing is a plan concrete enough that dam can build the core seam decisively in a two-week window, and that lets **sac build the whole authoring UI solo while dam is away** (the seam is the only serial dependency). So this plan optimizes for exactly one thing: **a small, complete `DocumentSchema` seam dam can finish fast, with everything else falling to app-side.**

The load-bearing invariant (from §7 + the editable-notes work): **the LLM only ever fills a *named schema*.** A custom field is safe to fill because it's a named slot the fill prompt knows about — never a freeform guess. That's what keeps "customizable documents" from becoming "silently wrong paperwork."

## 2. What's already decided (dam's §7 answers — the foundation)

- **§7.2 — core owns doc-type definitions.** `DocumentSchema` in murmur-core SQLite (sync-ready, versioned, tombstoned) + FFI CRUD; `buildDocument(kind:)` stays kind-keyed and resolves kind → active schema core-side, because the fill prompt is assembled in `pipeline/prompts.rs` — the schema must live where the prompt lives.
- **§7.3 — core mints doc numbers.** Custom types carry a `numberPrefix`; core mints from its counter. App never invents numbers.
- **§7.4 — build-time fill only (v1).** Custom fields influence the *finish-time* document build, not the walk-time extraction prompt (keeps the live board terse + the extraction prompt stable across operators). Vocab→STT biasing already carries operator jargon into the transcript, so the *terms* usually arrive even without prompt awareness.
- **The "confirm-once" rule** (§3) is for uploads (v3) — not needed here; authored schemas are named by construction.

## 3. Data model — `DocumentSchema`

A doc type is an ordered list of named sections; sections carry fields. Minimal, additive to today's document (letterhead + line-items + total + terms/signature):

```
DocumentSchema {
  id            // UUIDv7
  kind          // stable key: builtin ("estimate"/"invoice"/…) OR custom ("hoa_addendum")
  label         // "Estimate", "HOA Addendum"
  number_prefix // "EST", "HOA" — core mints <prefix>-NNNN
  trade_key     // which trade this doc type belongs to (or null = all)
  sections: [Section]   // ordered
  version, updated_at, tombstone   // same sync story as items/vocab
}
Section { key, label, fields: [Field] }   // ordered
Field {
  key, label
  type: line_items | text | long_text | currency | quantity | date | static
  fill: walk | manual | static
  static_value?   // for fill=static (fixed boilerplate, e.g. terms)
}
```

- `type=line_items` is the existing captured-items table (one per doc, usually). `fill=walk` fields are LLM-populated at build; `fill=manual` are blank for the operator to complete at review (like today's amount edit); `fill=static` is fixed text (this is where the current app-side `DocumentLayout` terms/signature **migrate in** — they become `static` fields on the schema).
- Built-in kinds ship as **seeded default schemas** (estimate/invoice/inspection/etc.), so the whole document path routes through the schema uniformly — no "custom vs built-in" fork in the renderer.

## 4. The core seam (dam — the ~1-week target)

Mirror the vocab/item CRUD he's already shipped twice:

```
// FFI on MurmurEngine (engine-keyed, works on any session; schemas are operator-scoped, not session-scoped)
list_document_schemas(trade_key?) -> [DocumentSchema]
save_document_schema(schema) -> DocumentSchema        // upsert by id
remove_document_schema(id)                            // tombstone
// buildDocument stays kind-keyed; internally: kind -> resolve active schema -> fill
build_document(session_id, kind) -> DocumentPayload   // unchanged signature; now schema-driven
```

Core responsibilities:
1. **Storage** — `document_schemas` table, migration, seed the built-in defaults, sync/tombstone (same as items).
2. **Fill** — `pipeline/prompts.rs`: build the document by walking the schema's fields — `line_items` from the session items, `walk` fields from the session (one focused structured-output pass, per §7.4), `static`/`manual` pass through. This is the real core work.
3. **Numbering** — mint `<number_prefix>-NNNN` per kind from the existing counter (§7.3).

## 5. The app side (sac — builds over dam's absence, once the seam exists)

1. **Document Builder editor** — a sheet off the board (same pattern as the Letterhead Studio; the Structure band in `docs/design/letterhead-studio-mockup.html` is the visual): pick/duplicate a doc type, reorder/toggle sections, add fields (label + type + fill), name a new doc type (+ prefix). Authors a `DocumentSchema`, hands it to the core via `save_document_schema`.
2. **Rendering** — the review/PDF renders from the schema's sections/fields (generalizes the current fixed layout). `manual` fields become tap-to-fill (reuse the amount-edit + editable-notes interaction). `static` fields render as blocks (the `DocumentLayout` terms/signature fold in here).
3. **Migration** — the app-side `DocumentLayout` (terms/signature) becomes `static` fields on the schema once the seam lands; keep it app-side until then.

## 6. Phasing (fits the constraint)

- **Now (sac):** this plan + a Document-Builder UI mockup refinement.
- **dam's 2 weeks:** the §4 seam — schema storage + CRUD FFI + schema-driven fill + numbering. *Only* this is serial.
- **dam away (sac):** the Document Builder editor + schema-driven rendering + `DocumentLayout` migration. Ships customization **without dam present.**
- **v1 launch:** ships on the *seeded built-in schemas* (no user authoring exposed yet) — so the schema refactor is in but the app looks identical; customization flips on when the editor lands. Zero launch risk.
- **v3 (dam's return):** upload → infer-schema → confirm-once, on top of this same schema surface.

## 7. Open questions for dam

1. **Fill pass shape** — one structured-output call over the whole schema's `walk` fields, or reuse/extend `write_notes`? (Recommend one focused call; cost is negligible, one/doc-build.)
2. **Line-items cardinality** — assume exactly one `line_items` section per doc for v1? (Recommend yes.)
3. **Schema ↔ trade** — schemas scoped per `trade_key`, or global with a trade filter? (Recommend per-trade, matches how doc kinds already key off trade.)
4. **Seeded defaults location** — seed built-ins in core migration, or ship them app-side and `save` on first run? (Recommend core — one source of truth for the fill.)

## 8. Boundary

Core owns the schema record, the fill, numbering, sync (dam). The authoring UI + schema-driven rendering are sac's. The seam is the one serial dependency — everything after it is app-side, which is what makes this shippable across dam's absence.
