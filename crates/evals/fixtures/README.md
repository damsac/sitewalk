# Corpus extension recipe

Each scenario is a paired `<id>.txt` (transcript) + `<id>.json` (typed ground
truth) sharing a stem. The loader (`corpus::load_corpus`) errors loudly on any
orphan file, so the two halves cannot silently drift.

Five seeds ship today: `punch_list_short`, `deck_walk_contacts`,
`rambling_long_walk`, `empty_session`, `quiet_mulch_walk`. Grow the corpus to
**8–12 total** fixtures following these rules:

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
- Give each expected item's `text` **≥3 content tokens** (after stopword
  removal) — a 2-token item can Dice-match a wrong candidate on a single
  shared token at exactly the 0.5 threshold, an accidental match rather than
  a real one.
- Distractors must not Dice-match (≥0.5) any expected item's `text` in the
  same fixture — `load_corpus` enforces this and errors loudly if violated,
  since an overlapping distractor would wrongly count a correct extraction as
  an R6 false positive.

## What a scenario measures

Two independent axes, both reported per scenario by the runner:

- **extraction** — F0.5 over the board, plus the distractor false-positive
  rate. Driven by the ground truth above.
- **summary voice** (#298) — does the summary read as a record of the JOB?
  `summary::grade_summary` is lexical and deterministic: it flags a preamble
  that names the session ("Field session to discuss…") and any narration of
  the recording ("only the word 'mulch' was clearly audible"), and reports the
  word count. It needs no ground truth, so it applies to every scenario for
  free — but a quiet fixture like `quiet_mulch_walk` is where it bites, since
  a walk with little in it is where a model reaches for prose about the audio.
