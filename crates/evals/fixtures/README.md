# Corpus extension recipe

Each scenario is a paired `<id>.txt` (transcript) + `<id>.json` (typed ground
truth) sharing a stem. The loader (`corpus::load_corpus`) errors loudly on any
orphan file, so the two halves cannot silently drift.

Four seeds ship today: `punch_list_short`, `deck_walk_contacts`,
`rambling_long_walk`, `empty_session`. Grow the corpus to **8–12 total**
fixtures following these rules:

- Each scenario's ground truth must be traceable to a literal span in its
  transcript — no inferred items.
- Cover the kind space: every `VALID_KINDS` value (`todo, decision, note,
  safety, part, price`) appears in **≥2** scenarios.
- Every scenario except pure punch-lists carries **≥2** `distractors` — R6 is
  only measured where there's chatter to resist.
- Vary length: **≥2** short (<150 words), **≥2** medium, **≥2** long (>500
  words).
- Vary trades: framing, plumbing, electrical, concrete, roofing vocabulary.
- Include **≥1** STT-garble scenario (misrecognized jargon/names the model
  should still normalize via memory — e.g. "french drain" heard as "trench
  rain").
- Target 8–12 fixtures; the grader and runner are corpus-size-agnostic.
