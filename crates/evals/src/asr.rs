//! Speech accuracy: did the words that BECOME the paperwork survive the
//! microphone?
//!
//! `binding` and `grounding` measure text → document. Neither can see the
//! layer beneath: if whisper hears "weed eating" as "wheat eating", the
//! transcript says so, the document faithfully reflects the transcript, and
//! every other axis scores perfect on a work order that lost a line. That is
//! the likeliest cause of the intermittent dropout in #357, and until now
//! `crates/stt` had no accuracy eval at all.
//!
//! Isaac, 2026-08-16: *"The number one most important thing about this app
//! should be that what is said is accurately transcribed and then placed into
//! documents the way it's supposed to."* This is the first half of that
//! sentence.
//!
//! ## Why word error rate is not the headline
//!
//! WER weights every word the same, and this app does not. "So we're gonna do
//! about four yards" → "So we're going to do about four yards" is two errors
//! and costs nothing: no item changes, no document changes, the operator
//! never notices. "Weed eating along the fence line" → "Wheat eating along
//! the fence line" is ONE error and deletes a line of work from a crew sheet.
//!
//! So the headline is **critical-term recall**: of the words that become line
//! items, prices, names and codes, how many survived? WER is reported
//! alongside as context — a run where WER moves and recall does not is a
//! model getting chattier, not a model getting worse.

use serde::{Deserialize, Serialize};

/// One reference recording and what must survive it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AsrCase {
    pub id: String,
    /// Path to the audio, relative to the corpus directory.
    pub audio: String,
    /// What was actually said, verbatim.
    pub reference: String,
    /// The words this walk cannot afford to lose — trade terms, names,
    /// numbers, codes. Multi-word terms are matched as a phrase, because
    /// "weed" and "eating" arriving separately is not the same as the term
    /// arriving.
    pub critical: Vec<String>,
    /// Vocabulary terms to bias the decoder with, as the app would from the
    /// operator's memory. Empty means "measure the unbiased baseline".
    #[serde(default)]
    pub vocabulary: Vec<String>,
}

/// One case's score.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AsrScore {
    /// Critical terms present in the hypothesis.
    pub kept: Vec<String>,
    /// Critical terms the microphone lost. Each one is a line of paperwork.
    pub lost: Vec<String>,
    /// kept / (kept + lost). The headline. 1.0 when a case names none.
    pub critical_recall: f64,
    /// Word error rate over the whole utterance, for context only.
    pub wer: f64,
    /// Every critical term survived.
    pub ok: bool,
}

/// Ordered, comparable words, sharing the workspace's number canonicalization
/// so "three hundred" and "$300" are the same transcript. That sharing is not
/// tidiness: the first real baseline reported "three hundred", "two hundred"
/// and "ten dollars" as LOST on a decode that had heard them perfectly and
/// written them as digits. An eval that cries mishearing over a spelling
/// wastes the exact attention it exists to direct.
///
/// Stopwords are KEPT, unlike the extraction grader's set — word error rate
/// has to see "the", and it has to see order.
pub fn words(text: &str) -> Vec<String> {
    crate::normalize::tokens_ordered(&expand_currency(text))
}

/// "$10" → "10 dollars". Whisper writes the symbol; the operator said the
/// word, and a reference written the way it was SPOKEN is the only kind worth
/// keeping. The second false loss of the first baseline, after the hundreds.
fn expand_currency(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        let mut digits = String::new();
        while chars.peek().is_some_and(|d| d.is_ascii_digit() || *d == ',' || *d == '.') {
            digits.push(chars.next().unwrap());
        }
        if digits.is_empty() {
            out.push(c);
        } else {
            out.push_str(&digits);
            out.push_str(" dollars");
        }
    }
    out
}

/// Levenshtein distance over words, two rows at a time.
fn edit_distance(a: &[String], b: &[String]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, x) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(x != y);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Is this term present as a contiguous phrase?
fn contains_phrase(hypothesis: &[String], term: &str) -> bool {
    let needle = words(term);
    if needle.is_empty() {
        return true;
    }
    hypothesis.windows(needle.len()).any(|w| w == needle.as_slice())
}

/// Grades one decode against what was said.
pub fn grade_asr(reference: &str, hypothesis: &str, critical: &[String]) -> AsrScore {
    let reference_words = words(reference);
    let hypothesis_words = words(hypothesis);

    let mut kept = Vec::new();
    let mut lost = Vec::new();
    for term in critical {
        if contains_phrase(&hypothesis_words, term) {
            kept.push(term.clone());
        } else {
            lost.push(term.clone());
        }
    }

    // A case with no critical terms is vacuously perfect on recall — it has
    // nothing to lose. It still reports WER, which is the whole reason to
    // keep such a case in a corpus.
    let total = kept.len() + lost.len();
    let critical_recall = if total == 0 { 1.0 } else { kept.len() as f64 / total as f64 };

    // An empty reference cannot have a rate; an empty hypothesis against a
    // real reference is a total loss, which falls out of the distance.
    let wer = if reference_words.is_empty() {
        0.0
    } else {
        edit_distance(&reference_words, &hypothesis_words) as f64 / reference_words.len() as f64
    };

    AsrScore { ok: lost.is_empty(), kept, lost, critical_recall, wer }
}

/// Aggregate over a corpus run.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AsrSuite {
    pub cases: usize,
    /// Cases that lost nothing critical.
    pub clean: usize,
    /// Every critical term across every case that did not survive, so a
    /// regression names the words rather than moving a decimal.
    pub lost: Vec<String>,
    pub mean_critical_recall: f64,
    pub mean_wer: f64,
}

pub fn summarize(scores: &[AsrScore]) -> AsrSuite {
    if scores.is_empty() {
        return AsrSuite::default();
    }
    let n = scores.len() as f64;
    AsrSuite {
        cases: scores.len(),
        clean: scores.iter().filter(|s| s.ok).count(),
        lost: scores.iter().flat_map(|s| s.lost.clone()).collect(),
        mean_critical_recall: scores.iter().map(|s| s.critical_recall).sum::<f64>() / n,
        mean_wer: scores.iter().map(|s| s.wer).sum::<f64>() / n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAID: &str = "Weed eating along the fence line, about forty feet. \
                        Marcus is stripping the bark. Gate code is 7781.";

    fn critical() -> Vec<String> {
        ["weed eating", "fence line", "marcus", "7781"].map(String::from).to_vec()
    }

    /// The #357 hypothesis: one word wrong, one line of work gone. WER calls
    /// this a 5% error; the operator calls it a missing task.
    #[test]
    fn a_single_misheard_trade_term_is_the_headline_failure() {
        let heard = "Wheat eating along the fence line, about forty feet. \
                     Marcus is stripping the bark. Gate code is 7781.";
        let score = grade_asr(SAID, heard, &critical());
        assert!(!score.ok);
        assert_eq!(score.lost, vec!["weed eating"]);
        assert!(score.wer < 0.1, "and WER barely moves: {}", score.wer);
    }

    /// The inverse, and the reason WER is not the headline: a decode that
    /// rewrites half the filler words loses nothing that reaches paperwork.
    #[test]
    fn filler_differences_cost_nothing_that_matters() {
        let heard = "So, weed eating over along that fence line, it's about forty feet or so. \
                     Marcus, he is stripping the bark there. The gate code is 7781.";
        let score = grade_asr(SAID, heard, &critical());
        assert!(score.ok, "nothing critical was lost: {:?}", score.lost);
        assert_eq!(score.critical_recall, 1.0);
        assert!(score.wer > 0.2, "even though WER is substantial: {}", score.wer);
    }

    /// A gate code one digit off is a crew standing at a locked gate.
    #[test]
    fn a_misheard_number_is_lost_even_though_it_sounds_close() {
        let heard = SAID.replace("7781", "77 81");
        let score = grade_asr(SAID, &heard, &critical());
        assert_eq!(score.lost, vec!["7781"]);
    }

    /// Multi-word terms match as phrases. "Weed" and "eating" landing in
    /// different sentences is not the term arriving.
    #[test]
    fn a_term_is_only_kept_when_its_words_arrive_together() {
        let heard = "Eating lunch by the fence line. Pull the weed by the gate.";
        let score = grade_asr(SAID, heard, &["weed eating".to_string()]);
        assert_eq!(score.lost, vec!["weed eating"]);
    }

    /// The currency symbol is the word. Whisper writes "$10"; the operator
    /// said "ten dollars", and the reference records speech.
    #[test]
    fn a_currency_symbol_matches_the_spoken_words() {
        let score = grade_asr("ten dollars of gas", "$10 of gas", &["ten dollars".into()]);
        assert!(score.ok, "{:?}", score.lost);
        assert_eq!(score.wer, 0.0);
    }

    /// Spelled and digit forms are the same transcript, or every number in
    /// every walk would read as an error.
    #[test]
    fn spelled_numbers_match_digits() {
        let score = grade_asr("about forty feet, four yards", "about forty feet, 4 yards", &[]);
        assert_eq!(score.wer, 0.0);
    }

    /// A perfect decode.
    #[test]
    fn an_exact_decode_scores_clean() {
        let score = grade_asr(SAID, SAID, &critical());
        assert!(score.ok);
        assert_eq!(score.wer, 0.0);
        assert_eq!(score.critical_recall, 1.0);
    }

    /// Silence against a real walk is a total loss, and must not divide by
    /// zero on the way to saying so.
    #[test]
    fn an_empty_decode_loses_everything() {
        let score = grade_asr(SAID, "", &critical());
        assert_eq!(score.critical_recall, 0.0);
        assert_eq!(score.wer, 1.0);
    }

    #[test]
    fn the_suite_names_the_words_that_were_lost() {
        let suite = summarize(&[
            grade_asr(SAID, SAID, &critical()),
            grade_asr(SAID, &SAID.replace("Weed", "Wheat"), &critical()),
        ]);
        assert_eq!(suite.cases, 2);
        assert_eq!(suite.clean, 1);
        assert_eq!(suite.lost, vec!["weed eating"]);
        assert_eq!(suite.mean_critical_recall, 0.875);
    }
}
