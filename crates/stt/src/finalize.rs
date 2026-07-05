use crate::decoder::RawSegment;

/// A finalized-or-pending word with absolute time. When `time_precise` is
/// `false` (empty or count-mismatched `RawSegment.words`) all words expanded
/// from one whisper segment share that segment's coarse span; when `true`
/// (Plan 09 word-anchored path) each word carries its own token-derived span.
/// `time_precise` is what the coarse-seam drop rule branches on (D1); `lib.rs`
/// only reads `text`/`start_ms`/`end_ms`, so `FinalizedSegment`/FFI are
/// unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub time_precise: bool,
}

/// Incremental, time-anchored overlap-merge finalizer — the productionized
/// `reassemble_dedup` + `finalize` from `spikes/stt-whisper/src/stream.rs`
/// (`RESULTS.md` Table 2: 19% WER at ≤3 s latency, vs 80% for naive segment
/// finalize). `pending` is bounded to ~one chunk; the emitted stream is
/// append-only (a finalized word is never revised).
pub struct Finalizer {
    pending: Vec<Word>,
    flushed: bool,
    /// Segments with `no_speech_prob` above this are dropped at ingest (Plan 08
    /// Task 11b, R3). Default keeps the pre-Plan-08 behavior for scripted
    /// segments (which carry `no_speech_prob = 0.0`).
    no_speech_threshold: f32,
}

impl Default for Finalizer {
    fn default() -> Self {
        Self { pending: Vec::new(), flushed: false, no_speech_threshold: 0.6 }
    }
}

impl Finalizer {
    /// Construct with an explicit no-speech drop threshold (from `SttConfig`).
    pub fn with_no_speech_threshold(no_speech_threshold: f32) -> Self {
        Self { no_speech_threshold, ..Self::default() }
    }

    /// Merge one decoded window (`window_start_ms` + its segments) into `pending`
    /// via the spike's suffix/prefix text overlap, then finalize every word whose
    /// segment ends at/before `horizon_ms` (= next window's start for a normal
    /// window; `u64::MAX` for the flush window). Returns newly finalized words.
    pub fn ingest(&mut self, window_start_ms: u64, segs: &[RawSegment], horizon_ms: u64) -> Vec<Word> {
        let new_words = words_from_segments(window_start_ms, segs, self.no_speech_threshold);
        self.merge(new_words);
        self.finalize_before(horizon_ms)
    }

    /// Final window with no successor: commit the entire remaining tail.
    pub fn flush(&mut self) -> Vec<Word> {
        if self.flushed {
            return Vec::new();
        }
        self.flushed = true;
        self.finalize_before(u64::MAX)
    }

    /// Volatile preview (un-finalized tail) for greyed UI. Never persisted.
    pub fn preview(&self) -> String {
        self.pending.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
    }

    /// Merge one decoded window's words into `pending`, deduping the re-decoded
    /// overlap. Only ever appends — existing `pending` words stand (append-only).
    /// TWO seams:
    ///   • **Precise (text) seam** — spike `reassemble_dedup`: the largest *k*
    ///     where `pending`'s last *k* word texts equal the new window's first *k*.
    ///     When the overlap re-decoded identically, this stitches it exactly.
    ///   • **Coarse (time) seam** — first-decode-wins fallback when the text match
    ///     fails (`best == 0`). An all-or-nothing text merge that finds no match
    ///     would append the ENTIRE new window, so a *partially disagreeing* overlap
    ///     (e.g. "needs work" re-decoded as "needs word") would duplicate the
    ///     overlap phrase into the finalized (committed) stream. Instead, use the
    ///     absolute timestamps we deliberately kept: drop the prefix of `new_words`
    ///     whose `end_ms` ≤ the max `end_ms` already in `pending` — those are
    ///     re-transcriptions of audio the finalizer already holds — keeping the
    ///     FIRST decode of the disputed overlap and appending only the genuinely-new
    ///     suffix. Stays O(overlap).
    ///
    /// CAVEAT — RESOLVED when word timing is present (Plan 09 D1). The coarse
    /// seam's drop now branches on `Word.time_precise`, keying on DIFFERENT
    /// fields for the two modes because no single field serves both:
    ///   • **Word-precise** (`time_precise = true`) drops by `start_ms`: a word
    ///     STARTING at/inside already-committed audio (`start_ms ≤
    ///     pending_max_end`) is a re-decode of held audio (first-decode-wins),
    ///     boundary-equality resolving to DROP (R6/R3 under-commit — a word
    ///     starting exactly where the last committed word ended, in a seam where
    ///     the decodes already disagree on text, is presumed a re-decode).
    ///     `end_ms` is unusable here: a divergent re-decode can INFLATE an early
    ///     disputed word's `end_ms` past the old boundary (the legacy `end_ms ≤`
    ///     rule would then keep it → duplication — the leak this fixes).
    ///   • **Segment-coarse** (`time_precise = false`, empty/count-mismatched
    ///     `words`) keeps the LEGACY `end_ms ≤ pending_max_end` rule verbatim:
    ///     all words in a segment share the segment span, so a coarse start is
    ///     unreliable while a coarse genuinely-new segment ends well past the
    ///     boundary. This is now the explicit DEGRADED path — byte-for-byte the
    ///     pre-Plan-09 behavior, so every existing coarse test passes unchanged.
    /// Append-only holds by construction either way: the seam only ever drops a
    /// PREFIX of NEW words; no committed (`pending`) word is revisited.
    fn merge(&mut self, new_words: Vec<Word>) {
        if self.pending.is_empty() {
            self.pending = new_words;
            return;
        }
        let maxk = self.pending.len().min(new_words.len()).min(40);
        let mut best = 0;
        for k in (1..=maxk).rev() {
            let tail = &self.pending[self.pending.len() - k..];
            if tail.iter().map(|w| &w.text).eq(new_words[..k].iter().map(|w| &w.text)) {
                best = k;
                break;
            }
        }
        if best > 0 {
            self.pending.extend(new_words.into_iter().skip(best)); // precise seam
            return;
        }
        // Coarse seam: no text match → drop the covered prefix, keep first decode.
        let pending_max_end = self.pending.iter().map(|w| w.end_ms).max().unwrap_or(0);
        self.pending.extend(new_words.into_iter().skip_while(|w| {
            if w.time_precise {
                // Word-precise: a word STARTING at/inside already-committed audio
                // is a re-decode (first-decode-wins; boundary-equality drops — R6
                // under-commit). Its `end_ms` is untrustworthy (divergent inflation).
                w.start_ms <= pending_max_end
            } else {
                // Segment-coarse: legacy rule — coarse starts are segment-shared
                // and unreliable, ends aren't.
                w.end_ms <= pending_max_end
            }
        }));
    }

    /// Drain and return the front run of words whose segment ends ≤ horizon
    /// (spike `finalize`: `seg.end <= chunk_end − overlap`).
    fn finalize_before(&mut self, horizon_ms: u64) -> Vec<Word> {
        let cut = self.pending.iter().position(|w| w.end_ms > horizon_ms).unwrap_or(self.pending.len());
        self.pending.drain(..cut).collect()
    }
}

fn words_from_segments(window_start_ms: u64, segs: &[RawSegment], no_speech_threshold: f32) -> Vec<Word> {
    let mut out = Vec::new();
    let to_ms = |cs: i64| window_start_ms + (cs.max(0) as u64) * 10;
    for s in segs {
        // Task 11b (R3): drop segments whisper flags as probably-not-speech
        // (machinery drone it hallucinated text over) before they can reach the
        // committed transcript. Scripted/Plan-06 segments carry 0.0 → kept.
        // Runs FIRST — upstream of word expansion (Plan 08 basis unchanged).
        if s.no_speech_prob > no_speech_threshold {
            continue;
        }
        let seg_start = to_ms(s.start_cs);
        let seg_end = to_ms(s.end_cs);
        let split: Vec<&str> = s.text.split_whitespace().collect();
        if !s.words.is_empty() && s.words.len() == split.len() {
            // Word-anchored (D4): authoritative text from the split, timing from
            // the aligned per-word entries via the identical cs→ms formula.
            // start/end clamped non-decreasing (defends a stray out-of-order
            // token timestamp; whisper is monotonic in practice, not assumed).
            let mut last_end = seg_start;
            for (tok, w) in split.iter().zip(&s.words) {
                let start = to_ms(w.start_cs).max(last_end); // non-decreasing start
                let end = to_ms(w.end_cs).max(start); // end ≥ start
                out.push(Word { text: (*tok).to_string(), start_ms: start, end_ms: end, time_precise: true });
                last_end = end;
            }
        } else {
            // Coarse fallback (empty words OR count mismatch, D4) — pre-Plan-09
            // behavior verbatim: every word shares the segment's coarse span.
            for tok in split {
                out.push(Word { text: tok.to_string(), start_ms: seg_start, end_ms: seg_end, time_precise: false });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::RawSegment;

    fn seg(cs0: i64, cs1: i64, t: &str) -> RawSegment {
        RawSegment { start_cs: cs0, end_cs: cs1, text: t.into(), no_speech_prob: 0.0, words: vec![] }
    }
    fn words(ws: &[Word]) -> Vec<&str> {
        ws.iter().map(|w| w.text.as_str()).collect()
    }
    fn words_full(ws: &[Word]) -> &[Word] {
        ws
    }
    fn seg_words(cs0: i64, cs1: i64, words: &[(&str, i64, i64)]) -> RawSegment {
        RawSegment::with_words(
            RawSegment { start_cs: cs0, end_cs: cs1,
                text: words.iter().map(|(t, _, _)| *t).collect::<Vec<_>>().join(" "),
                no_speech_prob: 0.0, words: vec![] },
            words.iter().map(|(t, a, b)| crate::decoder::WordTiming {
                text: (*t).into(), start_cs: *a, end_cs: *b }).collect(),
        )
    }

    #[test]
    fn word_timing_gives_each_word_its_own_end_ms() {
        // One phrase-level segment [0,4800ms] but per-word timing: "needs" ends 4000,
        // "work" ends 4800. Coarse expansion would stamp BOTH at 4800.
        let mut f = Finalizer::default();
        let out = f.ingest(0, &[seg_words(0, 480, &[("needs", 0, 400), ("work", 400, 480)])], u64::MAX);
        assert_eq!(out[0].end_ms, 4000, "word-precise, not segment-coarse 4800");
        assert_eq!(out[1].end_ms, 4800);
    }

    #[test]
    fn word_anchored_coarse_seam_drops_lumped_divergent_overlap_without_duplication() {
        // FLAGSHIP (see worked arithmetic above). W1 re-decodes the held overlap
        // "needs work" as "needs word" LUMPED with new text into one long segment; the
        // divergent word "word" is INFLATED to end 5600 (past pending_max_end 4800), so
        // a legacy end-based drop would leak it. The word-anchored start-based rule drops
        // it (start 4800 ≤ 4800) and keeps only the genuinely-new suffix.
        let mut f = Finalizer::default();
        let e0 = f.ingest(0, &[
            seg_words(0, 360, &[("the", 0, 100), ("french", 100, 240), ("drain", 240, 360)]),
            seg_words(360, 480, &[("needs", 360, 440), ("work", 440, 480)]),
        ], 4_000);
        assert_eq!(words(&e0), vec!["the", "french", "drain"]);
        let e1 = f.ingest(4_000, &[seg_words(0, 400, &[
            ("needs", 0, 80), ("word", 80, 160), ("before", 160, 260), ("the", 260, 330), ("pour", 330, 400),
        ])], 8_000);
        let all: Vec<&str> = words(&e0).into_iter().chain(words(&e1)).collect();
        assert_eq!(all, vec!["the", "french", "drain", "needs", "work", "before", "the", "pour"]);
        assert!(!all.contains(&"word"), "divergent re-decode dropped (start ≤ boundary)");
        assert_eq!(all.iter().filter(|w| **w == "work").count(), 1, "first decode wins, no duplication");
        // append-only: start_ms non-decreasing across the whole committed stream.
        let mut prev = 0;
        for w in words_full(&e0).iter().chain(words_full(&e1).iter()) { assert!(w.start_ms >= prev); prev = w.start_ms; }
    }

    #[test]
    fn inflated_early_disputed_word_does_not_leak_past_the_seam() {
        // DEDICATED inflated-early-word case (reviewer's finding): the disputed word's
        // end is inflated FAR past the boundary; end-based drop would keep it (dup),
        // start-based drop removes it.
        let mut f = Finalizer::default();
        // W0: "pour" finalized (end 2000 ≤ 4000), "footing" held (2000..4800).
        let e0 = f.ingest(0, &[seg_words(0, 480, &[("pour", 0, 200), ("footing", 200, 480)])], 4_000);
        assert_eq!(words(&e0), vec!["pour"]);
        // W1: overlap re-decoded DIFFERENTLY ("footings") and INFLATED to end 6000 (≫4800),
        // then genuinely-new "now"(6000..7000).
        let e1 = f.ingest(4_000, &[seg_words(0, 300, &[("footings", 0, 200), ("now", 200, 300)])], 8_000);
        let all: Vec<&str> = words(&e0).into_iter().chain(words(&e1)).collect();
        // "footing" (W0 first decode) survives once; divergent "footings" dropped; "now" kept.
        assert_eq!(all, vec!["pour", "footing", "now"]);
        assert!(!all.contains(&"footings"), "inflated divergent re-decode dropped by start-based rule");
    }

    #[test]
    fn empty_word_timing_degrades_to_segment_coarse() {
        let mut f = Finalizer::default();
        let out = f.ingest(0, &[seg(0, 480, "needs work")], u64::MAX);
        assert_eq!(out[0].end_ms, 4800, "coarse: both share segment end");
        assert_eq!(out[1].end_ms, 4800);
    }

    #[test]
    fn existing_segment_coarse_disagreement_still_keeps_new_suffix() {
        // The pre-Plan-09 test's scenario, restated to guard the mode-awareness: in
        // COARSE mode the genuinely-new segment "before the pour" has segment-start
        // 4800 == pending_max_end. The start-based rule would wrongly DROP it; the
        // legacy end-based rule (time_precise=false) keeps it. This must stay green.
        let mut f = Finalizer::default();
        let e0 = f.ingest(0, &[seg(0, 180, "the french drain"), seg(180, 480, "needs work")], 4_000);
        assert_eq!(words(&e0), vec!["the", "french", "drain"]);
        let e1 = f.ingest(4_000, &[seg(0, 80, "needs word"), seg(80, 400, "before the pour")], 8_000);
        let all: Vec<&str> = words(&e0).into_iter().chain(words(&e1)).collect();
        assert_eq!(all, vec!["the", "french", "drain", "needs", "work", "before", "the", "pour"]);
        assert!(!all.contains(&"word"));
    }

    #[test]
    fn mismatched_word_count_falls_back_to_coarse() {
        // Count disagrees with text split → coarse fallback (time_precise=false), no panic,
        // text still matches the split, spans segment-coarse.
        let mut f = Finalizer::default();
        let bad = RawSegment::with_words(
            RawSegment { start_cs: 0, end_cs: 300, text: "alpha beta gamma".into(),
                no_speech_prob: 0.0, words: vec![] },
            vec![crate::decoder::WordTiming { text: "alpha".into(), start_cs: 0, end_cs: 100 }], // 1 ≠ 3
        );
        let out = f.ingest(0, &[bad], u64::MAX);
        assert_eq!(words(&out), vec!["alpha", "beta", "gamma"]);
        assert!(out.iter().all(|w| w.end_ms == 3000), "coarse fallback: all share segment end");
    }

    #[test]
    fn no_speech_gate_still_drops_before_word_expansion() {
        // Plan 08 R3 gate untouched: a high-nsp WORD-TIMED segment is still dropped.
        let mut f = Finalizer::with_no_speech_threshold(0.6);
        let mut noisy = seg_words(0, 200, &[("phantom", 0, 100), ("words", 100, 200)]);
        noisy.no_speech_prob = 0.9;
        let out = f.ingest(0, &[noisy, seg_words(200, 320, &[("order", 200, 320)])], u64::MAX);
        assert_eq!(words(&out), vec!["order"], "drone dropped, speech kept (R3)");
    }

    #[test]
    fn finalizes_incrementally_across_time_shifted_windows() {
        let mut f = Finalizer::default();
        // window 0 [0,5s], horizon 4000: last segment straddles 4s → held.
        let e0 = f.ingest(0, &[seg(0, 180, "order twelve"), seg(180, 360, "two by tens"),
                               seg(360, 480, "for the")], 4_000);
        assert_eq!(words(&e0), vec!["order", "twelve", "two", "by", "tens"]);
        assert_eq!(f.preview(), "for the", "the straddling tail is held, not emitted");
        // window 1 [4s,9s], horizon 8000: head re-says the "for the" overlap.
        let e1 = f.ingest(4_000, &[seg(0, 80, "for the"), seg(80, 300, "deck framing"),
                                   seg(300, 480, "today now")], 8_000);
        assert_eq!(words(&e1), vec!["for", "the", "deck", "framing"]);
        // starvation guard: incremental progress, not one end-of-session dump.
        assert!(e0.len() + e1.len() >= 9, "words finalize as windows arrive");
    }

    #[test]
    fn overlap_word_is_finalized_exactly_once() {
        let mut f = Finalizer::default();
        let e0 = f.ingest(0, &[seg(0, 180, "hello there"), seg(360, 480, "friend")], 4_000);
        // "friend" ends 4800 > horizon 4000 → held for the overlap.
        let e1 = f.ingest(4_000, &[seg(0, 80, "friend"), seg(80, 300, "good day")], 8_000);
        let all: Vec<&str> = words(&e0).into_iter().chain(words(&e1)).collect();
        assert_eq!(all.iter().filter(|w| **w == "friend").count(), 1, "overlap emitted once");
    }

    #[test]
    fn append_only_holds_under_overlap_disagreement() {
        let mut f = Finalizer::default();
        let e0 = f.ingest(0, &[seg(0, 180, "the french drain"), seg(180, 480, "needs work")], 4_000);
        assert_eq!(words(&e0), vec!["the", "french", "drain"]); // ends ≤4000; "needs work" held
        // Window 1 re-decodes the overlap "needs work" DIFFERENTLY as "needs word":
        // the all-or-nothing text merge finds no match (best=0), so the TIME-ANCHORED
        // fallback drops the re-decoded overlap (end_ms ≤ pending's max end 4800) and
        // keeps W0's first decode, appending only the genuinely-new suffix.
        let e1 = f.ingest(4_000, &[seg(0, 80, "needs word"), seg(80, 400, "before the pour")], 8_000);

        let all: Vec<&str> = words(&e0).into_iter().chain(words(&e1)).collect();
        // Committed stream is exactly the first-decode reading with the overlap
        // present ONCE — no "needs work needs word" duplication (the bug this fixes).
        assert_eq!(all, vec!["the", "french", "drain", "needs", "work", "before", "the", "pour"]);
        // First decode of the disputed word wins; the divergent re-decode is gone.
        assert!(!all.contains(&"word"), "divergent second decode never reaches committed output");
        assert_eq!(all.iter().filter(|w| **w == "work").count(), 1, "disputed overlap not duplicated");
        // Genuinely-new content still finalizes.
        assert!(all.contains(&"before") && all.contains(&"pour"));
    }

    #[test]
    fn no_speech_segments_are_dropped_and_append_only_still_holds() {
        // Threshold 0.6: a mid-stream machinery-drone segment (0.9) is dropped;
        // the real-speech segments around it finalize normally and in order.
        let mut f = Finalizer::with_no_speech_threshold(0.6);
        let mut noisy = seg(180, 300, "phantom words"); // whisper hallucination
        noisy.no_speech_prob = 0.9;
        let e0 = f.ingest(0, &[seg(0, 180, "pour the footing"), noisy], 4_000);
        assert_eq!(words(&e0), vec!["pour", "the", "footing"], "drone segment dropped, speech kept");
        // A later window's real speech still appends after — append-only intact.
        let e1 = f.ingest(4_000, &[seg(0, 120, "before noon")], 8_000);
        let all: Vec<&str> = words(&e0).into_iter().chain(words(&e1)).collect();
        assert_eq!(all, vec!["pour", "the", "footing", "before", "noon"]);
        assert!(!all.contains(&"phantom"), "hallucinated words never committed (R3)");
    }

    #[test]
    fn flush_emits_only_the_bounded_tail() {
        let mut f = Finalizer::default();
        let e0 = f.ingest(0, &[seg(0, 180, "alpha beta"), seg(360, 480, "gamma delta")], 4_000);
        assert_eq!(words(&e0), vec!["alpha", "beta"]);
        assert_eq!(f.preview(), "gamma delta", "tail bounded to the straddling segment");
        let tail = f.flush();
        assert_eq!(words(&tail), vec!["gamma", "delta"], "flush finalizes only the held tail");
        assert!(f.flush().is_empty(), "flush is idempotent");
    }
}
