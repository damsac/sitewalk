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
//! - His flagstone estimate (2026-08-10): "three hundred for the flagstone"
//!   landed as $300 on the GRADING task. Same defect with money on it — a
//!   client would have paid that line.
//!
//! All three are one shape: an attribute spoken about item A landing on item
//! B. So this grades all three through one mechanism — a name, a
//! classification, and an amount are the same question asked of different
//! slots, and an amount only needs `normalize` in the middle so that "three
//! hundred" in the walk can support `$300` on the page.
//!
//! Lexical and deterministic on purpose, exactly like `summary` — no judge
//! model, no network, runs under `cargo test --workspace`, and comparable
//! across prompt variants. It cannot know what the operator MEANT; what it can
//! know is whether the transcript ever put the attribute and the item in the
//! same breath. That is a weaker claim than correctness and a much stronger
//! one than nothing, because the failures above are all cases where they were
//! never said together at all.

use serde::{Deserialize, Serialize};

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
/// Three kinds come out — everything a document attaches to a line:
///
/// - **assignees** — `lines[].assignee`, written only on directive documents.
/// - **amounts** — `lines[].amount_cents`, as spoken dollars. This is the one
///   with money on it: Isaac's flagstone estimate put his $300 of stone on
///   the grading task, and the client would have paid it.
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
        if let Some(cents) = line.get("amount_cents").and_then(|v| v.as_i64()) {
            out.push(Binding::new(title, dollars(cents)));
        }
        // Title AND detail. A classification lands in either — once the
        // extraction fix gave a no-work condition its own line, "normal wear"
        // moved into the TITLE ("Hallway scuffs — normal wear"), and a
        // harvester that read only details would have reported a clean sweep
        // by looking in the one place the phrase no longer was.
        let detail = line.get("detail").and_then(|v| v.as_str()).unwrap_or_default();
        let text = format!("{title} {detail}").to_lowercase();
        for phrase in phrases {
            if text.contains(&phrase.to_lowercase()) {
                out.push(Binding::new(title, *phrase));
            }
        }
    }
    out
}

/// Cents as the operator would have said them: `30000` → `"300"`, `4720` →
/// `"47.20"`. A whole-dollar amount drops the cents entirely, because nobody
/// says "three hundred point zero zero" and the walk will not contain it.
pub fn dollars(cents: i64) -> String {
    let cents = cents.abs();
    if cents % 100 == 0 {
        (cents / 100).to_string()
    } else {
        format!("{}.{:02}", cents / 100, cents % 100)
    }
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

/// True when `attribute` is a bare amount — "300", "47.20". Amounts are the
/// third thing a document binds to a line, after names and classifications,
/// and they are the one that cannot be matched as text: the document prints
/// `$300` and the operator said "three hundred". Routing them through
/// `normalize::token_set` (which already turns spelled numbers into digits
/// for the F0.5 grader) is what lets ONE mechanism cover all three.
fn is_amount(attribute: &str) -> bool {
    !attribute.is_empty()
        && attribute.chars().all(|c| c.is_ascii_digit() || c == '.')
        && attribute.chars().any(|c| c.is_ascii_digit())
}

/// Does this span carry the attribute? Text matches literally; an amount
/// matches against the span's normalized tokens, so "three hundred" in the
/// walk supports `$300` on the page.
fn span_carries(span_lower: &str, attribute: &str) -> bool {
    if is_amount(attribute) {
        let dollars = attribute.split('.').next().unwrap_or(attribute);
        crate::normalize::token_set(span_lower).contains(dollars)
    } else {
        span_lower.contains(attribute)
    }
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
        .find(|c| span_carries(&c.to_lowercase(), attribute))
        .unwrap_or(sentence)
}

/// Grades every binding a document makes against what was actually said.
pub fn grade_bindings(bindings: &[Binding], transcript: &str) -> BindingScore {
    let sentences = sentences(transcript);
    let attributes: Vec<String> = bindings.iter().map(|b| b.attribute.to_lowercase()).collect();
    let mut verdicts = Vec::new();

    for binding in bindings {
        let attribute = binding.attribute.to_lowercase();
        // Normalized so the page and the walk can disagree about spelling and
        // still be talking about the same thing: "4 yards" ↔ "four yards",
        // "beds" ↔ "bed".
        //
        // The attribute's own words are excluded, because an attribute cannot
        // be evidence for itself. Once a classification prints in the title
        // ("Bedroom carpet burn — normal wear"), "normal" and "wear" appear on
        // BOTH sides, and the span "Scuffs down the hallway are normal wear"
        // would match itself into a pass — the exact defect, scored clean.
        let attribute_words = crate::normalize::token_set(&binding.attribute);
        let item_words: Vec<String> = crate::normalize::token_set(&binding.item)
            .into_iter()
            .filter(|w| w.len() > 2 || w.chars().all(|c| c.is_ascii_digit()))
            .filter(|w| !attribute_words.contains(w))
            .collect();

        // The sentence that carries the attribute, and the item words it also
        // carries. Best match wins: an attribute repeated across sentences is
        // supported if ANY of them also names the item.
        let mut best: Option<(String, Vec<String>)> = None;
        for sentence in &sentences {
            let lower = sentence.to_lowercase();
            if !span_carries(&lower, &attribute) {
                continue;
            }
            // Narrow to a clause only when this sentence is genuinely
            // ambiguous — it names someone or something else the document
            // also bound. Otherwise the whole sentence is the honest scope,
            // and narrowing would fail "Dana, who's on the trailer, takes
            // the weed eating" for punctuation.
            let contested = attributes
                .iter()
                .any(|other| *other != attribute && span_carries(&lower, other));
            let scope = if contested { clause_for(sentence, &attribute) } else { sentence };
            let scope_tokens = crate::normalize::token_set(scope);
            let shared: Vec<String> =
                item_words.iter().filter(|w| scope_tokens.contains(*w)).cloned().collect();
            let better = best.as_ref().is_none_or(|(_, b)| shared.len() > b.len());
            if better {
                best = Some((scope.to_string(), shared));
            }
        }

        let (sentence, shared) = match best {
            Some((s, shared)) => (Some(s), shared),
            None => (None, Vec::new()),
        };
        // At least one WORD, never a number alone. "2 bags of compost" and "2
        // gates" share a token and are not the same work, and an amount
        // binding is the case where this matters most: `$300` next to a line
        // whose only tie to the span is the 300 itself proves nothing.
        let supported = shared.iter().any(|w| w.chars().any(|c| c.is_alphabetic()));
        verdicts.push(BindingVerdict {
            item: binding.item.to_string(),
            attribute: binding.attribute.to_string(),
            supported,
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

    /// Isaac's flagstone walk (2026-08-10), the one whose estimate put his
    /// $300 of stone on the grading task.
    const FLAGSTONE: &str = "Grade and weed the path area first to get it flat, then lay \
the gravel down, then the flagstone goes in. Three hundred for the flagstone, two hundred \
for the gravel, three hundred for the labor.";

    /// The money version of the same defect — and the reason amounts go
    /// through `normalize`: the walk says "three hundred", the page says
    /// $300, and nothing matches them as text.
    #[test]
    fn a_price_on_the_wrong_line_is_unsupported() {
        let score = grade_bindings(
            &[
                Binding::new("Flagstone, supply and lay", "300"),
                Binding::new("Grade and weed path area", "300"),
            ],
            FLAGSTONE,
        );
        assert!(score.verdicts[0].supported, "the stone is what $300 was said about");
        assert!(!score.verdicts[1].supported, "the grading was never priced out loud");
    }

    /// A price nobody spoke at all. `price_items` may only use a stated
    /// figure or a remembered one (Isaac, 2026-08-10), so with no memory in
    /// play an unspoken amount is an invention — and it is the class of
    /// defect he said is worth interrupting him for.
    #[test]
    fn a_price_that_was_never_spoken_is_unsupported() {
        let score = grade_bindings(&[Binding::new("Flagstone, supply and lay", "450")], FLAGSTONE);
        assert!(!score.ok);
        assert_eq!(score.verdicts[0].sentence, None, "no span of the walk carries $450");
    }

    /// Cents survive the round trip, because a typed tax or a per-unit price
    /// is not whole dollars and must still be checkable.
    #[test]
    fn amounts_read_the_way_they_are_spoken() {
        assert_eq!(dollars(30000), "300");
        assert_eq!(dollars(4720), "47.20");
        assert_eq!(dollars(5), "0.05");
    }

    /// The harvest covers money too: an amount on a line is a binding, the
    /// same as a name on one.
    #[test]
    fn the_harvest_reads_amounts_off_a_document() {
        let doc = serde_json::json!({
            "lines": [
                {"title": "Grade and weed path area", "detail": "", "assignee": null,
                 "amount_cents": 30000},
            ]
        });
        let score = grade_bindings(&document_bindings(&doc, &[]), FLAGSTONE);
        assert_eq!(score.unsupported, 1, "the grading was never the thing $300 was said about");
    }

    /// The shape the extraction fix produced: the classification lives in the
    /// TITLE now, not the detail. Both directions must still separate, and
    /// the wrong one must not be able to prove itself with its own words.
    #[test]
    fn a_classification_in_the_title_is_still_graded_on_its_own_merits() {
        let doc = serde_json::json!({
            "lines": [
                {"title": "Hallway scuffs — normal wear", "detail": "", "is_gap": false},
                {"title": "Bedroom carpet burn — normal wear", "detail": "", "is_gap": false},
            ]
        });
        let score = grade_bindings(&document_bindings(&doc, &["normal wear"]), PROPERTY);
        assert!(score.verdicts[0].supported, "the scuffs are what was called normal wear");
        assert!(
            !score.verdicts[1].supported,
            "the burn is damage — and 'normal wear' must not prove itself: {:?}",
            score.verdicts[1]
        );
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
