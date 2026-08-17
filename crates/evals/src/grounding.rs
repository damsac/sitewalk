//! Grounding: is every number and every name on this page one the operator
//! actually said?
//!
//! `binding` asks whether an attribute landed on the right line. This asks
//! the prior question, of every slot a document prints: does this value come
//! from the walk at all? They are the two halves of the promise the app
//! makes — *what you said, placed where it belongs* — and until both are
//! measured, "accurate" is a feeling.
//!
//! The rule is deliberately narrow, because a narrow rule can be enforced
//! deterministically and a broad one cannot:
//!
//! > **Nothing printed may contain a number or a name that was not spoken.**
//!
//! Numbers and names are the invention that costs money. A gate code the
//! model rounded off, a start date nobody agreed to, a crew member who does
//! not work here, a price the walk never mentioned — these are the failures
//! Isaac said are worth interrupting for, and every one of them is a token
//! that is either in the transcript or is not.
//!
//! What this deliberately does NOT do is grade the prose. A detail line that
//! reads "Strip before laying the mulch" invents nothing even though not one
//! of those words is a quotation, and a grader strict enough to flag it would
//! be measuring writing style under the name of accuracy. Connective English
//! is free; facts are not.
//!
//! ## The layer beneath this one
//!
//! This grades TEXT → DOCUMENT. It cannot see AUDIO → TEXT. If whisper hears
//! "weed eating" as "wheat eating", the transcript says so, the document
//! faithfully reflects the transcript, and every axis here scores perfect.
//! That layer has no eval at all today (`crates/stt` has no fixture corpus),
//! and it is the more likely cause of the intermittent dropout in #357.

use serde::{Deserialize, Serialize};

use crate::normalize::token_set;

/// One printed value that contains something the walk does not.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Ungrounded {
    /// Where on the document this printed — `"field:access"`, `"line:detail"`.
    pub slot: String,
    /// The value as printed, so the finding can be read without the document.
    pub value: String,
    /// The numbers and names in it that the walk never contained.
    pub tokens: Vec<String>,
}

/// A document's grounding score.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GroundingScore {
    pub findings: Vec<Ungrounded>,
    /// Slots carrying at least one number or name — the ones that could have
    /// been wrong. A document of pure prose has nothing to check and says so
    /// rather than claiming a perfect score.
    pub checked: usize,
    /// Nothing on the page states a number or a name the walk did not.
    pub ok: bool,
}

/// Sentence-initial words are excluded from the name check: "Strip the old
/// bark" opens with a capital because it opens a sentence, not because Strip
/// is anybody. Everything after a terminator starts a new sentence.
fn name_candidates(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut sentence_start = true;
    for word in value.split_whitespace() {
        let trimmed: String = word.trim_matches(|c: char| !c.is_alphanumeric()).to_string();
        let capitalized = trimmed.chars().next().is_some_and(char::is_uppercase);
        // An ALL-CAPS token is a heading or an abbreviation ("HOA", "GFCI"),
        // not a name, and trades are full of them.
        let shouty = trimmed.chars().filter(|c| c.is_alphabetic()).all(char::is_uppercase);
        if !sentence_start && capitalized && !shouty && trimmed.chars().any(char::is_alphabetic) {
            out.push(trimmed.to_lowercase());
        }
        sentence_start = word.ends_with(['.', '!', '?', ':', ';']);
    }
    out
}

/// Small numbers SPELLED OUT in prose are English, not claims. "Found one
/// burn, one drip and a torn screen" states no quantity the operator failed
/// to state — "one" there is an article. A real quantity, code or price is
/// printed as digits, so this costs nothing and removes the only false alarm
/// the first real sweep produced.
const SPELLED: [(&str, &str); 9] = [
    ("1", "one"),
    ("2", "two"),
    ("3", "three"),
    ("4", "four"),
    ("5", "five"),
    ("6", "six"),
    ("7", "seven"),
    ("8", "eight"),
    ("9", "nine"),
];

fn is_prose_number(token: &str, value_lower: &str) -> bool {
    SPELLED
        .iter()
        .any(|(digit, word)| *digit == token && value_lower.contains(word) && !value_lower.contains(*digit))
}

/// Every number and name in `value` that the walk does not contain.
///
/// Numbers go through `token_set`, so a walk that says "four yards" grounds a
/// page that says "4 yards" — the same normalization the F0.5 grader uses.
pub fn ungrounded_tokens(value: &str, transcript: &str) -> Vec<String> {
    let spoken = token_set(transcript);
    let value_lower = value.to_lowercase();
    let mut out: Vec<String> = Vec::new();
    for token in token_set(value) {
        if token.chars().all(|c| c.is_ascii_digit())
            && !spoken.contains(&token)
            && !is_prose_number(&token, &value_lower)
        {
            out.push(token);
        }
    }
    for name in name_candidates(value) {
        if !spoken.contains(&name) && !out.contains(&name) {
            out.push(name);
        }
    }
    out.sort();
    out
}

/// True when this value contains anything checkable at all.
fn states_a_fact(value: &str) -> bool {
    value.chars().any(|c| c.is_ascii_digit()) || !name_candidates(value).is_empty()
}

/// Grades every printed slot of a built document against the walk.
///
/// Walks the artifact body as stored — the same JSON the app renders — so
/// this measures what shipped. Gap rows and gap fields are skipped: a blank
/// states nothing, which is the posture R6 asks for and the one the app is
/// built around.
///
/// Amounts are NOT checked here. They are bindings (`binding::document_bindings`),
/// because a price is never wrong merely by existing — it is wrong by sitting
/// on the wrong line, and that needs the item to answer.
pub fn document_grounding(document: &serde_json::Value, transcript: &str) -> GroundingScore {
    let mut findings = Vec::new();
    let mut checked = 0;

    let mut check = |slot: String, value: &str| {
        if value.trim().is_empty() || !states_a_fact(value) {
            return;
        }
        checked += 1;
        let tokens = ungrounded_tokens(value, transcript);
        if !tokens.is_empty() {
            findings.push(Ungrounded { slot, value: value.to_string(), tokens });
        }
    };

    if let Some(fields) = document.get("fields").and_then(|v| v.as_array()) {
        for field in fields {
            if field.get("is_gap").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }
            // `static` fields are the operator's own authored text, not
            // something the walk produced — grading them against a transcript
            // would flag every set of terms they ever wrote.
            if field.get("fill").and_then(|v| v.as_str()) == Some("static") {
                continue;
            }
            let key = field.get("key").and_then(|v| v.as_str()).unwrap_or("?");
            if let Some(value) = field.get("value").and_then(|v| v.as_str()) {
                check(format!("field:{key}"), value);
            }
        }
    }

    if let Some(lines) = document.get("lines").and_then(|v| v.as_array()) {
        for line in lines {
            if line.get("is_gap").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }
            for slot in ["title", "detail", "qty"] {
                if let Some(value) = line.get(slot).and_then(|v| v.as_str()) {
                    check(format!("line:{slot}"), value);
                }
            }
        }
    }

    GroundingScore { ok: findings.is_empty(), checked, findings }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LANDSCAPE: &str = "Front yard at the Kesler place. Four yards of dark mulch in the \
front beds, two bags of compost worked into the rose bed. Marcus is stripping the bark and \
laying mulch, Dana takes the weed eating. Gate code is 7781. Dog's in the back until nine. \
We're starting Tuesday morning.";

    fn field(key: &str, value: &str) -> serde_json::Value {
        serde_json::json!({"key": key, "value": value, "is_gap": false, "fill": "walk"})
    }

    #[test]
    fn a_gate_code_the_walk_states_is_grounded() {
        let doc = serde_json::json!({"fields": [field("access", "Gate code 7781, dog in back until nine")]});
        let score = document_grounding(&doc, LANDSCAPE);
        assert!(score.ok, "{:?}", score.findings);
        assert_eq!(score.checked, 1);
    }

    /// One digit off. This is the failure that would cost a crew an hour at a
    /// locked gate, and it is invisible to every other axis: the field is
    /// present, the sentence reads perfectly, and the number is wrong.
    #[test]
    fn a_gate_code_that_drifted_one_digit_is_caught() {
        let doc = serde_json::json!({"fields": [field("access", "Gate code 7718")]});
        let score = document_grounding(&doc, LANDSCAPE);
        assert!(!score.ok);
        assert_eq!(score.findings[0].tokens, vec!["7718"]);
        assert_eq!(score.findings[0].slot, "field:access");
    }

    /// A crew member who does not work here.
    #[test]
    fn a_name_nobody_said_is_caught() {
        let doc = serde_json::json!({"fields": [field("crew", "Marcus, Dana and Priya")]});
        let score = document_grounding(&doc, LANDSCAPE);
        assert_eq!(score.findings[0].tokens, vec!["priya"]);
    }

    /// A date nobody agreed to — the third thing on Isaac's interrupt list,
    /// after a price and a name.
    #[test]
    fn a_schedule_the_walk_never_set_is_caught() {
        let doc = serde_json::json!({"fields": [field("schedule", "Starting Thursday morning")]});
        let score = document_grounding(&doc, LANDSCAPE);
        assert_eq!(score.findings[0].tokens, vec!["thursday"]);

        let real = serde_json::json!({"fields": [field("schedule", "Starting Tuesday morning")]});
        assert!(document_grounding(&real, LANDSCAPE).ok);
    }

    /// Spelled numbers ground digits, because the operator speaks one and the
    /// document prints the other. Without this every quantity on every
    /// document would read as invented.
    #[test]
    fn spelled_numbers_ground_printed_digits() {
        let doc = serde_json::json!({
            "lines": [{"title": "Dark mulch, 4 yards", "qty": "4", "is_gap": false}]
        });
        assert!(document_grounding(&doc, LANDSCAPE).ok);
    }

    /// Connective prose is free. The compose pass is supposed to write, and a
    /// grader that flagged "Strip before laying the mulch" for the word
    /// "Strip" would be measuring style and calling it accuracy.
    #[test]
    fn written_prose_that_invents_nothing_passes() {
        let doc = serde_json::json!({
            "lines": [{
                "title": "Strip old bark, front beds",
                "detail": "Strip before laying the mulch. Comes out first.",
                "is_gap": false
            }]
        });
        let score = document_grounding(&doc, LANDSCAPE);
        assert!(score.ok, "{:?}", score.findings);
    }

    /// A blank states nothing, so it cannot mis-state anything — and it must
    /// not be counted as a slot that passed, either. R6's posture, scored
    /// honestly in both directions.
    #[test]
    fn gaps_are_not_graded_and_not_credited() {
        let doc = serde_json::json!({
            "fields": [{"key": "access", "value": null, "is_gap": true, "fill": "walk"}],
            "lines": [{"title": "Compost", "detail": "", "is_gap": true}]
        });
        let score = document_grounding(&doc, LANDSCAPE);
        assert!(score.ok);
        assert_eq!(score.checked, 0, "nothing on this page was checkable");
    }

    /// From the first real sweep, which produced exactly one false alarm:
    /// a summary reading "Found one burn, one drip, and one torn screen"
    /// against a walk that said "there's a burn". Nothing was invented — the
    /// document wrote English.
    #[test]
    fn a_spelled_out_count_in_prose_is_not_an_invented_number() {
        let doc = serde_json::json!({
            "fields": [field("summary", "Found one burn, one drip and one torn screen.")]
        });
        assert!(document_grounding(&doc, LANDSCAPE).ok);
    }

    /// And the exemption is only for prose. A digit is a claim, always —
    /// otherwise a wrong gate code could hide behind the word "one".
    #[test]
    fn a_printed_digit_is_always_a_claim() {
        let doc = serde_json::json!({"fields": [field("access", "Unit 1 lockbox 4419")]});
        let score = document_grounding(&doc, LANDSCAPE);
        assert_eq!(score.findings[0].tokens, vec!["1", "4419"]);
    }

    /// Trade abbreviations are not names. A document full of HOA, GFCI and
    /// PSI would otherwise report a page of inventions.
    #[test]
    fn abbreviations_are_not_treated_as_names() {
        let doc = serde_json::json!({
            "lines": [{"title": "Replace GFCI outlet", "detail": "Per HOA rules", "is_gap": false}]
        });
        let score = document_grounding(&doc, LANDSCAPE);
        assert!(score.ok, "{:?}", score.findings);
    }

    /// The operator's own authored boilerplate is not something the walk
    /// produced, so it is not graded against the walk.
    #[test]
    fn authored_static_text_is_not_graded_against_the_walk() {
        let doc = serde_json::json!({
            "fields": [{"key": "terms", "value": "Net 30. Late after 30 days.",
                        "is_gap": false, "fill": "static"}]
        });
        assert!(document_grounding(&doc, LANDSCAPE).ok);
    }
}
