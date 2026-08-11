//! Deterministic summary-voice grader (#298).
//!
//! The extraction grader in `grade.rs` reads exactly one thing about the
//! summary — whether there IS one. So the failure Isaac hit on TestFlight was
//! invisible to the suite: a summary can narrate the recording, apologise for
//! the audio, and open every walk with the same preamble while scoring a
//! perfect `summaries ok: 4/4`.
//!
//! This grader scores the summary TEXT for the two defects named in #298,
//! both of which are lexical and therefore deterministic — no judge model, no
//! network, comparable across prompt variants exactly like F0.5:
//!
//!   1. **preamble** — the summary opens by naming the session ("Field session
//!      to discuss mulch work"). The operator knows it was a field session;
//!      that is what the app is.
//!   2. **narration** — the summary is about the RECORDING rather than the job
//!      ("Only the word 'mulch' was clearly audible in the recording"). This is
//!      R6-adjacent: the model fills a gap with prose instead of declining to
//!      fill it.
//!
//! Length is measured and reported but deliberately does NOT gate `ok`: a
//! whole-house walk earns more words than a one-bed mulch job, and a threshold
//! picked to split them would be a number pretending to be a rule. The word
//! count moves visibly in the report; the two boolean defects are the pins.

use serde::{Deserialize, Serialize};

/// Openers that name the session instead of the job. Matched at the START of
/// the summary only — "we walked the site" mid-sentence is ordinary English,
/// while leading with it spends a third of a board row saying nothing.
const PREAMBLES: [&str; 12] = [
    "field session",
    "a field session",
    "field walk",
    "a field walk",
    "field-work session",
    "a field-work session",
    "a brief field",
    "site visit",
    "a site visit",
    "session recorded",
    "the operator",
    "the user",
];

/// Vocabulary of the recording itself. Any occurrence, anywhere, is the
/// defect: these words describe the medium, and the summary is a record of the
/// job. Kept to nouns/adjectives that cannot plausibly describe field work —
/// "recorded" is excluded, since a client can genuinely authorise a recording
/// of a meeting and that IS a job fact.
const NARRATION_TERMS: [&str; 10] = [
    "recording",
    "transcript",
    "audio",
    "audible",
    "inaudible",
    "discernible",
    "ambient noise",
    "speech",
    "microphone",
    "no additional context",
];

/// One summary's voice score. `ok` is the headline boolean: this summary reads
/// as a record of the job.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SummaryScore {
    /// The preamble the summary opened with, if any.
    pub preamble: Option<String>,
    /// Every recording-narration term found, in `NARRATION_TERMS` order.
    pub narration: Vec<String>,
    /// Sentence count (terminator runs), min 1 for any non-empty text.
    pub sentences: usize,
    /// Whitespace-separated word count.
    pub words: usize,
    /// No preamble AND no narration. Length is reported, never gated.
    pub ok: bool,
}

/// Grades one summary string. Empty/absent text is not `ok` — there is nothing
/// for the operator to read.
pub fn grade_summary(summary: &str) -> SummaryScore {
    let text = summary.trim();
    let lower = text.to_lowercase();

    // Longest match first: "a field session" must win over "field session" so
    // the reported preamble is the phrase actually used.
    let mut candidates: Vec<&str> = PREAMBLES.to_vec();
    candidates.sort_by_key(|p| std::cmp::Reverse(p.len()));
    let preamble = candidates
        .into_iter()
        .find(|p| lower.starts_with(p))
        .map(str::to_string);

    let narration: Vec<String> = NARRATION_TERMS
        .iter()
        .filter(|t| lower.contains(**t))
        .map(|t| (*t).to_string())
        .collect();

    let words = text.split_whitespace().count();
    let sentences = count_sentences(text);
    let ok = !text.is_empty() && preamble.is_none() && narration.is_empty();
    SummaryScore { preamble, narration, sentences, words, ok }
}

/// Counts sentences as runs of terminators, so "3 sq. ft." style abbreviations
/// inflate the count a little but a trailing "." never adds a phantom one. A
/// non-empty summary with no terminator at all is one sentence.
fn count_sentences(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut in_run = false;
    for c in text.chars() {
        if matches!(c, '.' | '!' | '?') {
            if !in_run {
                count += 1;
                in_run = true;
            }
        } else if !c.is_whitespace() {
            in_run = false;
        }
    }
    count.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim from a device, quoted in #298. Both must fail — if they pass,
    /// this grader is measuring nothing.
    const DEVICE_MULCH: &str = "Field session to discuss mulch work. Only the word \"mulch\" was \
                                clearly audible in the recording, with no additional context \
                                provided about scope, timing, or constraints.";
    const DEVICE_SILENT: &str = "No actionable session content was captured. The transcript \
                                 contained only ambient noise with no discernible speech.";

    #[test]
    fn the_device_summaries_from_the_issue_fail() {
        let mulch = grade_summary(DEVICE_MULCH);
        assert!(!mulch.ok);
        assert_eq!(mulch.preamble.as_deref(), Some("field session"));
        assert!(mulch.narration.contains(&"audible".to_string()));
        assert!(mulch.narration.contains(&"recording".to_string()));

        let silent = grade_summary(DEVICE_SILENT);
        assert!(!silent.ok);
        assert!(silent.narration.contains(&"transcript".to_string()));
        assert!(silent.narration.contains(&"ambient noise".to_string()));
    }

    #[test]
    fn a_record_of_the_job_passes() {
        for s in [
            "North wall: water damage below the window, about 3 sq ft.",
            "Mulch and compost, three beds in the front yard; Juan starts Thursday.",
            "Unit twelve punch list: faucet cartridge, dead bedroom outlet, sprung closet hinge.",
        ] {
            let score = grade_summary(s);
            assert!(score.ok, "{s} should read as a record of the job: {score:?}");
        }
    }

    /// The honest short answer #298 asks for on a quiet walk.
    #[test]
    fn a_short_true_line_passes_and_is_short() {
        let score = grade_summary("Mulch work discussed.");
        assert!(score.ok);
        assert_eq!(score.sentences, 1);
        assert_eq!(score.words, 3);
    }

    #[test]
    fn an_empty_summary_is_not_ok() {
        let score = grade_summary("   ");
        assert!(!score.ok);
        assert_eq!(score.words, 0);
        assert_eq!(score.sentences, 0);
    }

    #[test]
    fn the_preamble_is_only_a_defect_when_it_leads() {
        // Same words, mid-sentence: ordinary English, not a board-row tax.
        assert!(grade_summary("Roof tear-off scheduled; site visit set for Tuesday.").ok);
        assert!(!grade_summary("Site visit to the Alder Ct roof.").ok);
    }

    #[test]
    fn the_longest_matching_preamble_is_the_one_reported() {
        let score = grade_summary("A field session to procure materials.");
        assert_eq!(score.preamble.as_deref(), Some("a field session"));
    }

    #[test]
    fn sentence_and_word_counts_measure_verbosity() {
        let score = grade_summary("First thing. Second thing. Third thing.");
        assert_eq!(score.sentences, 3);
        assert_eq!(score.words, 6);
    }
}
