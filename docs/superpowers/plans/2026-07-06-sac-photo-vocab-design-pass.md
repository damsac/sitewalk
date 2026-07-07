# Plan (sac): photo capture during the walk + photo/vocab design pass

Takes the two `sac:` handoffs from Plans 10/11 (vocabulary editor visuals,
photo gallery visuals) and closes the walk-time capture gap: the walk screen's
PHOTO button still runs the pre-Plan-11 fake counter (`items[idx].photos += 1`,
never touches the engine). The mockup's promise — *photos pin to the item
being spoken* — becomes real.

## Design decisions

1. **Walk-time capture is one tap, zero confirm.** Gloves and speed rule out a
   confirm step; the operator taps PHOTO, the camera fires, the shot pins to
   `lastCapturedID` (the item being spoken — dam's D3 rule), a chip appears.
   Device = camera (`UIImagePickerController`, the only zero-chrome path);
   simulator = PhotosPicker fallback. Item id goes across the seam lowercased
   (core ids are canonical-lowercase UUIDv7; `UUID.uuidString` is uppercase).
2. **No items yet → session-level photo** (`itemId: nil`). The gallery shows
   it at review; nothing is refused mid-walk. Chip bump is optimistic; the
   next `boardUpdated` carries the core's `photoCount` and self-corrects.
3. **Gallery reads as a contact sheet on the paper**, not an iOS grid: stamped
   `PHOTOS` label, ink-bordered thumbnails with `PH-01` index stamps, square
   ink ✕ remove (no `xmark.circle.fill`), dashed empty-state box, errors in
   the red-tint note bar (same grammar as the yellow RevNote).
4. **Vocabulary editor is a field tool, not a settings page.** Hand-rolled
   sheet: title + mono explainer ("names, plants, part numbers — the mic
   learns them"), `TERMS n / 100` counter, hairline rows with ✕, underlined
   mono add-field + block ADD button, yellow note bar for engine errors
   (full-at-100, dup, too-long), dashed empty state. Board entry point:
   the placeholder gear becomes a stamped `VOCAB` chip (design-language
   consistent; gear said "settings", this is a tool).

## Tasks

1. `AppModel.addPhoto()` → real: route through `capturePhoto(image:itemId:)`
   with optimistic chip bump; keep working on the demo engine (it has a
   session id + photo stubs).
2. `WalkView`: PHOTO button presents camera (device) / PhotosPicker (sim);
   `NSCameraUsageDescription` added to project.yml.
3. `ReviewView.photoGallery` restyle per decision 3.
4. `VocabularyView` rebuild per decision 4; `BoardView` gear → `VOCAB` chip.
5. Verification hooks: `autophoto=1` (autoflow injects a generated JPEG
   mid-walk — proves button-path → FFI → gallery headlessly) and
   `screen=vocab` (design-QA route). Screenshots: walk chip, review gallery,
   vocab editor.

## Out of scope (queued)

- Per-item photo grouping on the review document (needs core item ids on
  `DocRowFixture` rows — seam question for dam).
- Photos in the PDF (CORE.md v1.1 note).
- #168 DONE gating (awaiting dam's call on UI-side vs engine signal).
- Onboarding interview that seeds vocabulary (dam's note in Plan 10).
