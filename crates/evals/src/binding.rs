//! Attribute binding: did the thing the operator said about ONE item stay on
//! that item?
//!
//! Extraction quality (`grade`, F0.5) asks whether the right items came out.
//! Summary voice (`summary`) asks how the narrative reads. Neither can see the
//! defect that produced this module, because in both cases every item was
//! correct and the DOCUMENT was still wrong:
//!
//! - Isaac's work order, 2026-08-16 (#357): "Dana takes the weed eating" —
//!   the weed-eating item never arrived, and Dana was attached to the compost
//!   instead. A crew sheet confidently assigning work nobody asked for.
//! - His move-out and condition reports the same day (#359): "scuffs down the
//!   hallway are normal wear" came out as "kitchen faucet drips — normal
//!   wear". On a deposit document that phrase decides what may lawfully be
//!   withheld, so it exempted the wrong item and left the scuffs deductible.
//!
//! Both are one shape: an attribute spoken about item A landing on item B.
//!
//! Lexical and deterministic on purpose, exactly like `summary` — no judge
//! model, no network, runs under `cargo test --workspace`, and comparable
//! across prompt variants. It cannot know what the operator MEANT; what it can
//! know is whether the transcript ever put the attribute and the item in the
//! same breath. That is a weaker claim than correctness and a much stronger
//! one than nothing, because the failures above are all cases where they were
//! never said together at all.

use serde::{Deserialize, Serialize};

/// Words too common to prove two phrases are about the same thing. Matching on
/// "the" would make every binding look supported.
const STOPWORDS: [&str; 24] = [
    "the", "a", "an", "and", "or", "of", "in", "on", "at", "to", "for", "with", "is", "are",
    "was", "were", "be", "it", "that", "this", "all", "from", "by", "into",
];

/// One attribute the document attached to one line.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    /// The line's title, as printed.
    pub item: String,
    /// The attribute attached to it: an assignee name, or a phrase such as
    /// "normal wear" found in the line's detail.
    pub attribute: String,
}

impl Binding {
    pub fn new(item: impl Into<String>, attribute: impl Into<String>) -> Self {
        Binding { item: item.into(), attribute: attribute.into() }
    }
}

/// Harvests every binding a built document makes, from the artifact body as
/// stored — the same JSON the app renders, so this grades what shipped rather
/// than an intermediate the operator never sees.
///
/// Two kinds come out:
///
/// - **assignees** — `lines[].assignee`, written only on directive documents.
/// - **classifications** — any phrase in `phrases` that appears in a line's
///   `detail`. Passed in rather than inferred because the phrases that MATTER
///   are trade terms with consequences ("normal wear", "tenant damage",
///   "code violation"), and a grader that guessed at them would drift.
///
/// Gap rows are skipped: a line with nothing filled in has claimed nothing.
pub fn document_bindings(document: &serde_json::Value, phrases: &[&str]) -> Vec<Binding> {
    let mut out = Vec::new();
    let Some(lines) = document.get("lines").and_then(|v| v.as_array()) else {
        return out;
    };
    for line in lines {
        let title = line.get("title").and_then(|v| v.as_str()).unwrap_or_default();
        if title.trim().is_empty() {
            continue;
        }
        if let Some(assignee) = line.get("assignee").and_then(|v| v.as_str()) {
            if !assignee.trim().is_empty() {
                out.push(Binding::new(title, assignee.trim()));
            }
        }
        let detail = line.get("detail").and_then(|v| v.as_str()).unwrap_or_default().to_lowercase();
        for phrase in phrases {
            if detail.contains(&phrase.to_lowercase()) {
                out.push(Binding::new(title, *phrase));
            }
        }
    }
    out
}

/// One binding's verdict.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BindingVerdict {
    pub item: String,
    pub attribute: String,
    /// The span of transcript that carries the attribute, if any — the whole
    /// sentence, or the clause within it when the sentence names more than
    /// one graded attribute (see `clause_for`).
    pub sentence: Option<String>,
    /// Content words shared between that span and the item title.
    pub shared: Vec<String>,
    /// The attribute and the item were spoken in the same sentence.
    pub supported: bool,
}

/// A document's binding score.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct BindingScore {
    pub verdicts: Vec<BindingVerdict>,
    /// Bindings the transcript supports.
    pub supported: usize,
    /// Bindings it does not — each one an attribute moved onto a line the
    /// operator never said it about.
    pub unsupported: usize,
    /// Every binding is supported. An empty document is `ok`: nothing was
    /// claimed, so nothing was mis-claimed.
    pub ok: bool,
}

fn content_words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2 && !STOPWORDS.contains(w))
        .map(str::to_string)
        .collect()
}

/// Splits on sentence terminators. A binding is judged within one sentence
/// because that is the unit an operator speaks a qualifier in: "the scuffs are
/// normal wear" binds; "there are scuffs. the faucet drips. that's normal
/// wear" deliberately does not, and is exactly the input that produced #359.
fn sentences(transcript: &str) -> Vec<&str> {
    transcript
        .split(['.', '!', '?', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// The comma-clause of `sentence` that carries `attribute`.
///
/// Only used when the sentence carries more than one of the graded
/// attributes, because that is the only time a whole sentence is too coarse
/// to answer the question. Real field speech assigns a crew in one breath —
/// "Marcus is stripping the bark and laying mulch, Dana takes the weed
/// eating" — and against the whole sentence, EVERY name matches EVERY task.
/// The three real runs of this eval all produced exactly that sentence, so
/// without this the axis would have scored the #357 document a clean 3/3.
///
/// Commas only, never "and": "stripping the bark and laying mulch" is one
/// person doing two things, and splitting it would invent a failure.
fn clause_for<'a>(sentence: &'a str, attribute: &str) -> &'a str {
    sentence
        .split(',')
        .map(str::trim)
        .find(|c| c.to_lowercase().contains(attribute))
        .unwrap_or(sentence)
}

/// Grades every binding a document makes against what was actually said.
pub fn grade_bindings(bindings: &[Binding], transcript: &str) -> BindingScore {
    let sentences = sentences(transcript);
    let attributes: Vec<String> = bindings.iter().map(|b| b.attribute.to_lowercase()).collect();
    let mut verdicts = Vec::new();

    for binding in bindings {
        let attribute = binding.attribute.to_lowercase();
        let item_words = content_words(&binding.item);

        // The sentence that carries the attribute, and the item words it also
        // carries. Best match wins: an attribute repeated across sentences is
        // supported if ANY of them also names the item.
        let mut best: Option<(String, Vec<String>)> = None;
        for sentence in &sentences {
            let lower = sentence.to_lowercase();
            if !lower.contains(&attribute) {
                continue;
            }
            // Narrow to a clause only when this sentence is genuinely
            // ambiguous — it names someone or something else the document
            // also bound. Otherwise the whole sentence is the honest scope,
            // and narrowing would fail "Dana, who's on the trailer, takes
            // the weed eating" for punctuation.
            let contested = attributes
                .iter()
                .any(|other| *other != attribute && lower.contains(other.as_str()));
            let scope = if contested { clause_for(sentence, &attribute) } else { sentence };
            let scope_lower = scope.to_lowercase();
            let shared: Vec<String> = item_words
                .iter()
                .filter(|w| scope_lower.contains(w.as_str()))
                .cloned()
                .collect();
            let better = best.as_ref().is_none_or(|(_, b)| shared.len() > b.len());
            if better {
                best = Some((scope.to_string(), shared));
            }
        }

        let (sentence, shared) = match best {
            Some((s, shared)) => (Some(s), shared),
            None => (None, Vec::new()),
        };
        verdicts.push(BindingVerdict {
            item: binding.item.to_string(),
            attribute: binding.attribute.to_string(),
            supported: !shared.is_empty(),
            sentence,
            shared,
        });
    }

    let unsupported = verdicts.iter().filter(|v| !v.supported).count();
    BindingScore {
        supported: verdicts.len() - unsupported,
        unsupported,
        ok: unsupported == 0,
        verdicts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The transcript from Isaac's landscape walk, 2026-08-16.
    const LANDSCAPE: &str = "Front yard at the Kessler place. Four yards of dark mulch in \
the front beds and two bags of compost worked into the rose bed. Strip the old bark out \
first. Weed eating along the fence line, about forty feet. Marcus is stripping bark and \
laying mulch, Dana takes the weed eating. Gate code is 7781.";

    /// The property walk, same day.
    const PROPERTY: &str = "Unit four at 220 Bell, move-out walk. There's a burn in the \
bedroom carpet near the window, that's damage. Scuffs down the hallway are normal wear. \
The kitchen faucet drips. Screen on the back door is torn.";

    /// #357, as it shipped: Dana on the compost. She was never said in the
    /// same breath as compost, and this is the check that says so.
    #[test]
    fn an_assignee_moved_to_another_line_is_unsupported() {
        let score = grade_bindings(
            &[Binding::new("Compost, 2 bags, worked into rose bed", "Dana")],
            LANDSCAPE,
        );
        assert!(!score.ok);
        assert_eq!(score.unsupported, 1);
        assert!(score.verdicts[0].shared.is_empty());
    }

    /// The same assignee on the line she was actually given.
    #[test]
    fn an_assignee_on_the_work_it_was_spoken_with_is_supported() {
        let score = grade_bindings(
            &[Binding::new("Weed eating, fence line", "Dana")],
            LANDSCAPE,
        );
        assert!(score.ok, "{:?}", score.verdicts);
        assert!(score.verdicts[0].shared.contains(&"weed".to_string()));
    }

    /// #359: "normal wear" arrived on the faucet. The operator said it about
    /// the scuffs, in a different sentence.
    #[test]
    fn a_classification_moved_to_another_item_is_unsupported() {
        let score = grade_bindings(
            &[Binding::new("Kitchen faucet drip repair", "normal wear")],
            PROPERTY,
        );
        assert!(!score.ok);
        assert_eq!(score.verdicts[0].shared, Vec::<String>::new());
    }

    #[test]
    fn a_classification_on_the_item_it_was_said_about_is_supported() {
        let score = grade_bindings(
            &[Binding::new("Scuff repair, hallway", "normal wear")],
            PROPERTY,
        );
        assert!(score.ok, "{:?}", score.verdicts);
        assert!(score.verdicts[0].shared.contains(&"scuff".to_string()));
    }

    /// An attribute the walk never contains at all cannot be supported — and
    /// the verdict says WHY (no sentence carried it) rather than just failing.
    #[test]
    fn an_attribute_never_spoken_reports_no_sentence() {
        let score = grade_bindings(
            &[Binding::new("Compost, 2 bags", "Priya")],
            LANDSCAPE,
        );
        assert!(!score.ok);
        assert_eq!(score.verdicts[0].sentence, None);
    }

    /// The whole-crew sentence, which three real runs of this eval all
    /// produced: one breath, two names, three tasks. Against the sentence
    /// entire, every name matches every task — so this is the case that
    /// decides whether the axis measures anything on real output.
    ///
    /// Marcus keeps both of his (one clause, two jobs, joined by "and").
    /// Dana keeps the weed eating and is refused the mulch.
    #[test]
    fn one_sentence_naming_two_people_still_separates_them() {
        let score = grade_bindings(
            &[
                Binding::new("Strip old bark, front beds", "Marcus"),
                Binding::new("Dark mulch, 4 yards, front beds", "Marcus"),
                Binding::new("Weed eating, fence line", "Dana"),
                Binding::new("Dark mulch, 4 yards, front beds", "Dana"),
            ],
            LANDSCAPE,
        );
        let supported: Vec<bool> = score.verdicts.iter().map(|v| v.supported).collect();
        assert_eq!(
            supported,
            vec![true, true, true, false],
            "the mulch belongs to Marcus's clause, not Dana's: {:?}",
            score.verdicts
        );
    }

    /// The narrowing is conditional, and this is why: an aside between commas
    /// would otherwise strip the item words out of the attribute's clause and
    /// invent a failure. With only one attribute in play there is nothing to
    /// disambiguate, so the whole sentence stands.
    #[test]
    fn an_aside_does_not_break_an_unambiguous_binding() {
        let score = grade_bindings(
            &[Binding::new("Weed eating, fence line", "Dana")],
            "Dana, who's on the trailer today, takes the weed eating.",
        );
        assert!(score.ok, "{:?}", score.verdicts);
    }

    /// A document that claims nothing mis-claims nothing.
    #[test]
    fn a_document_with_no_bindings_is_ok() {
        assert!(grade_bindings(&[], LANDSCAPE).ok);
    }

    /// The harvest, over an artifact body shaped exactly as `document.rs`
    /// writes one: an assignee is a binding, a listed phrase found in a
    /// line's detail is a binding, and an untouched gap row claims nothing.
    #[test]
    fn the_harvest_reads_assignees_and_listed_phrases_off_a_document() {
        let doc = serde_json::json!({
            "lines": [
                {"title": "Weed eating, fence line", "detail": "about 40 ft", "assignee": "Dana"},
                {"title": "Scuff repair, hallway", "detail": "Normal wear — not deductible",
                 "assignee": null},
                {"title": "Dark mulch, 4 yards", "detail": "", "assignee": null},
            ]
        });
        let found = document_bindings(&doc, &["normal wear"]);
        assert_eq!(
            found,
            vec![
                Binding::new("Weed eating, fence line", "Dana"),
                Binding::new("Scuff repair, hallway", "normal wear"),
            ],
            "a line with no assignee and no listed phrase claims nothing"
        );
    }

    /// End to end on the shape that shipped in #357: harvest the document,
    /// grade it against the walk, and the misplaced assignee is the finding.
    #[test]
    fn a_harvested_document_grades_against_its_walk() {
        let doc = serde_json::json!({
            "lines": [
                {"title": "Strip old bark, front beds", "detail": "", "assignee": "Marcus"},
                {"title": "Compost, 2 bags, rose bed", "detail": "", "assignee": "Dana"},
            ]
        });
        let score = grade_bindings(&document_bindings(&doc, &[]), LANDSCAPE);
        assert_eq!(score.supported, 1, "Marcus was said with the bark");
        assert_eq!(score.unsupported, 1, "Dana was not said with the compost");
        assert_eq!(score.verdicts[1].attribute, "Dana");
    }

    /// Stopwords cannot carry a binding: "the" appearing in both is not
    /// evidence that a sentence is about an item.
    #[test]
    fn common_words_do_not_prove_a_binding() {
        let score = grade_bindings(
            &[Binding::new("The and of", "normal wear")],
            PROPERTY,
        );
        assert!(!score.ok, "stopwords made an unsupported binding look supported");
    }
}
