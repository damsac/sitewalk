//! Folding spoken prices onto the work they describe.
//!
//! An operator walks a job and talks the way people talk: the scope first,
//! the money after. *"Five yards of mulch, two bags of compost, redo all the
//! garden beds — five hundred for labor, two hundred mulch, a hundred
//! compost."* The extractor faithfully records six items, because six things
//! were said. The document render is one line per item, so the estimate came
//! out with six lines: three unpriced scope lines marked NOT HEARD, and three
//! bare price lines under them.
//!
//! That is wrong on the paper even though it is right in the store. A client
//! reading it sees mulch twice and an estimate that appears to have missed
//! three prices it was actually told. Isaac's field report, 2026-08-09:
//! *"it made separate lines for labor, mulch, compost. It should have put
//! those prices in the first three lines."*
//!
//! ## What this does
//!
//! One deterministic pass over the session's items, before the render:
//!
//! - An item that is **nothing but a price for something else** (`"$200
//!   mulch"`) folds onto the earlier item it names (`"5 yards mulch"`) and
//!   stops being a line of its own.
//! - An item that **carries its own price** (`"5 yards mulch $200"`) keeps its
//!   line and takes the amount into the amount column.
//! - A price that **names nothing already on the board** (`"$500 labor"`)
//!   keeps its own line — a labor line on an estimate is a real line, and
//!   inventing a home for it would be a guess.
//!
//! ## Why it is deterministic, and why it is stingy
//!
//! Prices are the one thing R4/R6 will not let us guess: a wrong number in a
//! sent estimate is the product's worst failure mode, and it is worse when it
//! is wrong *quietly*. So this pass never invents an amount, never moves one
//! onto an item it cannot name a shared word with, and folds at most one
//! price onto any line. Everything it declines to fold stays visible as its
//! own line, which is the same information the operator has today — the
//! failure mode of this code is "no better than before", never "silently
//! attached $500 to the wrong item".
//!
//! It also runs BEFORE the LLM pricing pass, and what it produces is
//! authoritative over it: a price the operator actually said outranks a price
//! the model inferred, and items already priced here are never sent to the
//! model at all (cheaper, and it cannot contradict the operator).
//!
//! ## Why it is not in the extraction prompt
//!
//! Asking the extractor to attach prices to earlier items means asking a
//! partial-transcript pass to hold and revise state it cannot see — the live
//! pass sees only the newest slice, and the price usually arrives minutes
//! after the scope. Text arithmetic on the finished item list is the pass
//! that has all the information, and it is testable without a provider.

use std::collections::{HashMap, HashSet};

use crate::domain::CapturedItem;

/// Words that carry no identity when matching a price to its work. Kept
/// deliberately short: every word removed here is a word that can no longer
/// disagree, and a fold on a weak match is worse than no fold at all.
const STOPWORDS: [&str; 24] = [
    "the", "a", "an", "of", "for", "and", "to", "on", "in", "at", "all", "some", "any", "with",
    "plus", "per", "each", "is", "it", "that", "this", "there", "was", "were",
];

/// The pass's output: the items to render, in order, and the amounts the
/// operator actually spoke, keyed by the id of the item that should carry
/// them.
pub(crate) struct FoldedPrices {
    /// Items to render. Same order as the input, minus the price statements
    /// that found a home, with a rewritten `text` on any item whose amount
    /// moved into the amount column (leaving "$200" in the title next to a
    /// "$200" column reads as a mistake on the paper).
    pub items: Vec<CapturedItem>,
    /// `item_id` → cents, for prices that were SPOKEN. Authoritative over
    /// anything the pricing pass infers.
    pub amounts: HashMap<String, i64>,
}

impl FoldedPrices {
    /// The identity result: every item kept, nothing priced. Used for
    /// documents that have no amount column, where lifting a price out of a
    /// title would delete it from the only place it appears.
    pub fn unfolded(items: &[CapturedItem]) -> Self {
        FoldedPrices { items: items.to_vec(), amounts: HashMap::new() }
    }
}

/// How an item relates to money, once its text has been read.
enum Reading {
    /// No amount, or an amount we refuse to read (a rate, an ambiguous pair).
    Plain,
    /// Carries its own price: substantial text of its own, plus an amount.
    SelfPriced { cents: i64, title: String },
    /// A price for something else: an amount and a bare label, nothing more.
    Statement { cents: i64, title: String, label_tokens: Vec<String> },
}

/// The pass. See the module docs for the rules; the short version is that a
/// price only moves when the item it moves onto shares a real word with it.
pub(crate) fn fold_spoken_prices(items: &[CapturedItem]) -> FoldedPrices {
    let readings: Vec<Reading> = items.iter().map(|i| read(&i.text)).collect();

    // Candidates are the items a price can land ON: work lines that don't
    // already carry a price of their own. A price statement is never a target
    // — folding "$200 mulch" onto "$100 compost" would be nonsense.
    let mut amounts: HashMap<String, i64> = HashMap::new();
    let mut titles: HashMap<String, String> = HashMap::new();
    let mut folded_away: HashSet<String> = HashSet::new();
    let mut claimed: HashSet<usize> = HashSet::new();

    for (index, reading) in readings.iter().enumerate() {
        match reading {
            Reading::Plain => {}
            Reading::SelfPriced { cents, title } => {
                amounts.insert(items[index].id.clone(), *cents);
                titles.insert(items[index].id.clone(), title.clone());
                // A self-priced line that fully COVERS a bare line already on
                // the board absorbs it.
                //
                // Isaac's flagstone estimate (2026-08-10) had the pair:
                // "Poison oak removal" (no price) sitting orphaned directly
                // above "Regular poison oak removal — $200". Extraction is
                // asked for both the work and the price, and the fold merges
                // them — except this price's label ran to four words, so it
                // read as a line item in its own right rather than a price
                // statement, and no merge happened. The client sees the same
                // job twice, once with a gap.
                //
                // Only when the bare line adds NOTHING the priced one lacks
                // (every one of its words is already there), and the RICHER
                // title survives — so "5 yards of mulch installed, $450"
                // absorbing a bare "Mulch" keeps the yards and the installed.
                if let Some(covered) =
                    fully_covered_by(&tokenize(title), items, &readings, &claimed)
                {
                    claimed.insert(covered);
                    folded_away.insert(items[covered].id.clone());
                }
            }
            Reading::Statement { cents, title, label_tokens } => {
                match target_for(label_tokens, items, &readings, &claimed) {
                    Some(target) => {
                        claimed.insert(target);
                        amounts.insert(items[target].id.clone(), *cents);
                        folded_away.insert(items[index].id.clone());
                    }
                    // Nothing on the board answers to this price. It stays a
                    // line of its own — the operator said it, and dropping it
                    // to keep the paper tidy would delete money from an
                    // estimate.
                    None => {
                        amounts.insert(items[index].id.clone(), *cents);
                        titles.insert(items[index].id.clone(), title.clone());
                    }
                }
            }
        }
    }

    let kept = items
        .iter()
        .filter(|i| !folded_away.contains(&i.id))
        .map(|i| match titles.get(&i.id) {
            Some(title) => CapturedItem { text: title.clone(), ..i.clone() },
            None => i.clone(),
        })
        .collect();

    FoldedPrices { items: kept, amounts }
}

/// The item a price statement belongs to: the FIRST unclaimed work line that
/// shares a content word with the price's label — and, among those, the line
/// the price is most ABOUT.
///
/// This was "the first line that shares a word", earliest-wins, on the theory
/// that items are in spoken order and the earlier mention is the intended one.
/// Isaac's estimate (2026-08-10) is the counter-example: he said "three
/// hundred for the flagstone", and the earliest line mentioning flagstone was
/// **"Grade and weed path area for flagstones"** — a prep task that merely
/// referred to the material. His $300 for stone landed on grading, and the
/// stone itself had no price at all.
///
/// So: score each candidate by how much of it the label accounts for
/// (shared tokens ÷ the line's own tokens). "Flagstone" is the whole of a line
/// called "Flagstone" (1.0) and one word in six of "Grade and weed path area
/// for flagstones" (0.17). The most-about line wins; spoken order breaks ties,
/// which preserves the old behaviour exactly whenever candidates are equally
/// about the thing (two lines both simply called "mulch").
fn target_for(
    label_tokens: &[String],
    items: &[CapturedItem],
    readings: &[Reading],
    claimed: &HashSet<usize>,
) -> Option<usize> {
    if label_tokens.is_empty() {
        return None;
    }
    let wanted: HashSet<&str> = label_tokens.iter().map(String::as_str).collect();
    let mut best: Option<(usize, f64)> = None;
    for (index, item) in items.iter().enumerate() {
        if claimed.contains(&index) || !matches!(readings[index], Reading::Plain) {
            continue;
        }
        let tokens = tokenize(&item.text);
        if tokens.is_empty() {
            continue;
        }
        let shared = tokens.iter().filter(|t| wanted.contains(t.as_str())).count();
        if shared == 0 {
            continue;
        }
        let aboutness = shared as f64 / tokens.len() as f64;
        // Strictly greater: the earliest of equally-about lines keeps winning.
        if best.is_none_or(|(_, top)| aboutness > top) {
            best = Some((index, aboutness));
        }
    }
    best.map(|(index, _)| index)
}

/// The first unclaimed bare line whose every word already appears in
/// `tokens` — i.e. one that would add nothing to the document that the
/// caller's own line does not already say.
///
/// Deliberately total containment, not the `target_for` "most about" score: a
/// partial overlap means the bare line carries something of its own, and
/// absorbing it would delete that. "Poison oak removal" ⊆ "Regular poison oak
/// removal" absorbs; "Front bed mulch" ⊄ "5 yards of mulch" does not.
fn fully_covered_by(
    tokens: &[String],
    items: &[CapturedItem],
    readings: &[Reading],
    claimed: &HashSet<usize>,
) -> Option<usize> {
    if tokens.is_empty() {
        return None;
    }
    let have: HashSet<&str> = tokens.iter().map(String::as_str).collect();
    items.iter().enumerate().position(|(index, item)| {
        if claimed.contains(&index) || !matches!(readings[index], Reading::Plain) {
            return false;
        }
        let theirs = tokenize(&item.text);
        !theirs.is_empty() && theirs.iter().all(|t| have.contains(t.as_str()))
    })
}

/// Reads one item's text for money.
fn read(text: &str) -> Reading {
    let Some(found) = find_amount(text) else { return Reading::Plain };
    let label = strip_amount(text, &found);
    let title = if label.is_empty() { text.trim().to_string() } else { capitalized(&label) };
    let tokens = tokenize(&label);

    // The line between "a price for something else" and "a line item that
    // happens to state its price" is whether the text describes work. Two
    // signals, both cheap and both conservative: a quantity ("5 yards mulch
    // $200") or enough words to be a description rather than a name.
    let describes_work = tokens.len() > MAX_LABEL_WORDS || has_quantity(&label);
    if describes_work {
        Reading::SelfPriced { cents: found.cents, title }
    } else {
        Reading::Statement { cents: found.cents, title, label_tokens: tokens }
    }
}

/// A label longer than this is a description of work, not the name of it.
/// Three covers what people actually say when they price something they
/// already described — "labor", "the mulch", "all garden beds" — without
/// swallowing a real scope line.
const MAX_LABEL_WORDS: usize = 3;

struct Amount {
    cents: i64,
    /// Byte range of the whole money token in the source text.
    span: (usize, usize),
}

/// The single money amount in `text`, if there is exactly one and we are
/// willing to read it as a line total.
///
/// Refuses, in every case returning `None`:
/// - **No amount.** Nothing to do.
/// - **Two or more amounts** (`"$200 mulch and $100 compost"`). Which one is
///   this line's? Guessing picks a number for a client's invoice by coin
///   flip; the operator can see and fix one merged line, and cannot see a
///   wrong split.
/// - **A rate** (`"$95/yd"`, `"$40 per hour"`). A rate times an unknown
///   quantity is not a total, and putting it in the total column would
///   understate the bill — the one direction of error that costs the operator
///   money rather than embarrassment.
fn find_amount(text: &str) -> Option<Amount> {
    let mut found: Option<Amount> = None;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let dollar_first = bytes[i] == b'$';
        let digits_at = if dollar_first { i + 1 } else { i };
        // A number starts here only at a token boundary — "3x4" must not read
        // as two amounts, and "EST-0047" must not read as one.
        let boundary = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let Some((value, after)) = read_number(text, digits_at) else {
            i += 1;
            continue;
        };
        if !boundary || (!dollar_first && !has_currency_word(text, after)) {
            i = after.max(i + 1);
            continue;
        }
        let end = if dollar_first { after } else { end_of_currency_word(text, after) };
        if is_rate(text, end) {
            return None;
        }
        if found.is_some() {
            return None; // two amounts — ambiguous, hands off
        }
        found = Some(Amount { cents: value, span: (start, end) });
        i = end;
    }
    found
}

/// Parses `1,250`, `1250.50`, `250` at `at`, returning cents and the byte
/// index just past the number.
fn read_number(text: &str, at: usize) -> Option<(i64, usize)> {
    let bytes = text.as_bytes();
    let mut i = at;
    let mut whole = String::new();
    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b',') {
        if bytes[i].is_ascii_digit() {
            whole.push(bytes[i] as char);
        }
        i += 1;
    }
    if whole.is_empty() {
        return None;
    }
    let mut cents = whole.parse::<i64>().ok()?.checked_mul(100)?;
    // Cents, only as exactly two digits — "$200.5" is far more likely a
    // stray transcription than fifty cents, and reading it either way is a
    // guess about money.
    if i + 2 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
        let two = i + 3 <= bytes.len() && bytes[i + 2].is_ascii_digit();
        let ends = i + 3 >= bytes.len() || !bytes[i + 3].is_ascii_digit();
        if two && ends {
            let frac: i64 = text[i + 1..i + 3].parse().ok()?;
            cents = cents.checked_add(frac)?;
            i += 3;
        }
    }
    Some((cents, i))
}

/// "500 dollars" / "500 bucks" — the bare-number forms speech-to-text
/// produces when the speaker doesn't say a symbol.
fn has_currency_word(text: &str, after: usize) -> bool {
    let rest = text[after..].trim_start().to_ascii_lowercase();
    rest.starts_with("dollar") || rest.starts_with("buck")
}

fn end_of_currency_word(text: &str, after: usize) -> usize {
    let trimmed = text[after..].trim_start();
    let offset = after + (text.len() - after - text[after..].trim_start().len());
    let word_len = trimmed
        .char_indices()
        .find(|(_, c)| !c.is_alphabetic())
        .map_or(trimmed.len(), |(i, _)| i);
    offset + word_len
}

/// Whether the amount ending at `end` is a rate rather than a total.
fn is_rate(text: &str, end: usize) -> bool {
    let rest = text[end..].trim_start().to_ascii_lowercase();
    rest.starts_with('/') || rest.starts_with("per ") || rest.starts_with("an hour")
        || rest.starts_with("a yard") || rest.starts_with("each")
}

/// The text with the money token cut out and the joining words that are left
/// dangling ("for", "at", "—") cleaned off both ends.
fn strip_amount(text: &str, found: &Amount) -> String {
    let mut label = String::with_capacity(text.len());
    label.push_str(&text[..found.span.0]);
    label.push(' ');
    label.push_str(&text[found.span.1..]);
    let trimmed: &[_] = &[' ', '\t', '-', '–', '—', ':', ',', '.', ';', '(', ')', '@'];
    let mut label = label.trim_matches(trimmed).to_string();
    for lead in ["for ", "at ", "on ", "of ", "is ", "was "] {
        if label.to_ascii_lowercase().starts_with(lead) {
            label = label[lead.len()..].to_string();
        }
    }
    for tail in [" for", " at", " on", " of", " is", " was"] {
        if label.to_ascii_lowercase().ends_with(tail) {
            label.truncate(label.len() - tail.len());
        }
    }
    label.trim_matches(trimmed).to_string()
}

/// Content words, lowercased, stopwords dropped, naive-singularized so
/// "beds" matches "bed". Words shorter than three characters are dropped
/// too — matching on "ft" or "yd" would fold a price onto whatever unit
/// happened to appear first.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_lowercase())
        .filter(|w| w.len() >= 3 && !STOPWORDS.contains(&w.as_str()) && !w.chars().all(|c| c.is_ascii_digit()))
        .map(|w| singular(&w))
        .collect()
}

fn singular(word: &str) -> String {
    if word.len() > 3 && word.ends_with('s') && !word.ends_with("ss") {
        word[..word.len() - 1].to_string()
    } else {
        word.to_string()
    }
}

/// Whether a label states a quantity — the tell that it describes work
/// rather than naming it.
fn has_quantity(label: &str) -> bool {
    label.split(|c: char| !c.is_alphanumeric()).any(|w| {
        !w.is_empty() && (w.chars().all(|c| c.is_ascii_digit()) || w.chars().next().is_some_and(|c| c.is_ascii_digit()))
    })
}

fn capitalized(label: &str) -> String {
    let mut chars = label.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ItemSource;

    fn item(id: &str, kind: &str, text: &str) -> CapturedItem {
        CapturedItem {
            id: id.into(),
            session_id: "s1".into(),
            kind: kind.into(),
            text: text.into(),
            right: String::new(),
            source: ItemSource::Authoritative,
            done: false,
            created_at: 0,
            updated_at: 0,
            device_id: "d".into(),
        }
    }

    /// Isaac's field report, 2026-08-09, verbatim as an estimate.
    #[test]
    fn isaacs_walk_folds_the_prices_onto_the_work() {
        let items = vec![
            item("i1", "part", "5 yards mulch"),
            item("i2", "part", "2 bags compost"),
            item("i3", "todo", "redo all garden beds (5 beds)"),
            item("i4", "price", "$500 labor"),
            item("i5", "price", "$200 mulch"),
            item("i6", "price", "$100 compost"),
        ];
        let folded = fold_spoken_prices(&items);

        let titles: Vec<&str> = folded.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(
            titles,
            vec!["5 yards mulch", "2 bags compost", "redo all garden beds (5 beds)", "Labor"],
            "mulch and compost folded away; labor stayed as its own line, without its $ in the title"
        );
        assert_eq!(folded.amounts.get("i1"), Some(&20000), "$200 landed on the mulch line");
        assert_eq!(folded.amounts.get("i2"), Some(&10000), "$100 landed on the compost line");
        assert_eq!(folded.amounts.get("i4"), Some(&50000), "labor prices itself");
        assert_eq!(folded.amounts.get("i3"), None, "the beds were never priced — still a gap");
    }

    /// Isaac's flagstone estimate, 2026-08-10: "Poison oak removal" sat
    /// orphaned and unpriced directly above "Regular poison oak removal —
    /// $200". Same job, twice, one of them looking like a gap.
    #[test]
    fn a_self_priced_line_absorbs_the_bare_line_it_already_covers() {
        let items = vec![
            item("i1", "todo", "Poison oak removal"),
            item("i2", "price", "$200 regular poison oak removal"),
            item("i3", "price", "$100 hazard pay poison oak"),
        ];
        let folded = fold_spoken_prices(&items);
        let titles: Vec<&str> = folded.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(
            titles,
            vec!["Regular poison oak removal", "Hazard pay poison oak"],
            "the bare duplicate is gone; hazard pay stays its own line"
        );
        assert_eq!(folded.amounts.get("i2"), Some(&20000));
        assert_eq!(folded.amounts.get("i3"), Some(&10000));
    }

    /// Absorption must never delete detail: a bare line is only absorbed when
    /// every word of it is already present in the priced line.
    #[test]
    fn absorption_keeps_the_richer_title_and_spares_a_line_with_its_own_words() {
        let items = vec![
            item("i1", "part", "Mulch"),
            item("i2", "part", "Front bed edging"),
            item("i3", "price", "$450 five yards of mulch installed"),
        ];
        let folded = fold_spoken_prices(&items);
        let titles: Vec<&str> = folded.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(
            titles,
            vec!["Front bed edging", "Five yards of mulch installed"],
            "bare Mulch absorbed into the richer line; the edging is untouched"
        );
        assert_eq!(folded.amounts.get("i3"), Some(&45000));
    }

    #[test]
    fn an_item_that_carries_its_own_price_keeps_its_line() {
        let items = vec![item("i1", "part", "5 yards of mulch installed $450")];
        let folded = fold_spoken_prices(&items);
        assert_eq!(folded.items.len(), 1);
        assert_eq!(folded.items[0].text, "5 yards of mulch installed");
        assert_eq!(folded.amounts.get("i1"), Some(&45000));
    }

    #[test]
    fn a_price_naming_nothing_on_the_board_stays_its_own_line() {
        let items = vec![item("i1", "todo", "fix the gate"), item("i2", "price", "$500 labor")];
        let folded = fold_spoken_prices(&items);
        assert_eq!(folded.items.len(), 2, "nothing folded — no shared word");
        assert_eq!(folded.amounts.get("i2"), Some(&50000));
        assert_eq!(folded.amounts.get("i1"), None);
    }

    #[test]
    fn one_price_per_line_never_two() {
        let items = vec![
            item("i1", "part", "5 yards mulch"),
            item("i2", "price", "$200 mulch"),
            item("i3", "price", "$300 mulch"),
        ];
        let folded = fold_spoken_prices(&items);
        assert_eq!(folded.amounts.get("i1"), Some(&20000), "first price wins the line");
        assert_eq!(folded.amounts.get("i3"), Some(&30000), "the second stays visible, not lost");
        assert_eq!(folded.items.len(), 2);
    }

    #[test]
    fn earliest_matching_line_wins_when_they_are_equally_about_it() {
        let items = vec![
            item("i1", "part", "front bed mulch"),
            item("i2", "part", "back bed mulch"),
            item("i3", "price", "$200 mulch"),
        ];
        let folded = fold_spoken_prices(&items);
        assert_eq!(folded.amounts.get("i1"), Some(&20000));
        assert_eq!(folded.amounts.get("i2"), None);
    }

    /// Isaac's estimate, 2026-08-10: "three hundred for the flagstone" landed
    /// on "Grade and weed path area for flagstones" — the earliest line that
    /// merely MENTIONED the material — and the stone itself went unpriced.
    #[test]
    fn a_price_lands_on_the_line_it_is_most_about_not_the_first_mention() {
        let items = vec![
            item("i1", "todo", "Grade and weed path area for flagstones"),
            item("i2", "part", "Flagstone"),
            item("i3", "todo", "Flagstone path labor"),
            item("i4", "price", "$300 flagstone"),
        ];
        let folded = fold_spoken_prices(&items);
        assert_eq!(folded.amounts.get("i2"), Some(&30000), "the stone is priced");
        assert_eq!(folded.amounts.get("i1"), None, "not the prep task that mentions it");
        assert_eq!(folded.amounts.get("i3"), None, "and not the labor line either");
    }

    /// The tie-break still runs in spoken order, so a walk with no
    /// distinguishing line behaves exactly as it did before.
    #[test]
    fn aboutness_never_beats_an_exact_single_word_line() {
        let items = vec![
            item("i1", "part", "Gravel"),
            item("i2", "todo", "Spread gravel over the graded path area"),
            item("i3", "price", "$200 gravel"),
        ];
        let folded = fold_spoken_prices(&items);
        assert_eq!(folded.amounts.get("i1"), Some(&20000));
        assert_eq!(folded.amounts.get("i2"), None);
    }

    #[test]
    fn plural_and_singular_match() {
        let items =
            vec![item("i1", "todo", "rebuild 3 garden beds"), item("i2", "price", "$900 the bed")];
        let folded = fold_spoken_prices(&items);
        assert_eq!(folded.amounts.get("i1"), Some(&90000));
        assert_eq!(folded.items.len(), 1);
    }

    #[test]
    fn spoken_dollars_without_a_symbol_are_read() {
        let items =
            vec![item("i1", "part", "cedar chips"), item("i2", "price", "500 dollars for chips")];
        let folded = fold_spoken_prices(&items);
        assert_eq!(folded.amounts.get("i1"), Some(&50000));
        assert_eq!(folded.items.len(), 1);
    }

    #[test]
    fn commas_and_cents_parse() {
        assert_eq!(find_amount("$1,250 stone").map(|a| a.cents), Some(125_000));
        assert_eq!(find_amount("$1250.75 stone").map(|a| a.cents), Some(125_075));
        assert_eq!(find_amount("$1250.5 stone").map(|a| a.cents), Some(125_000), "not two digits — read the dollars, never invent the cents");
    }

    #[test]
    fn a_rate_is_never_read_as_a_total() {
        for text in ["mulch $95/yd", "$40 per hour labor", "$40 an hour", "$95 each"] {
            assert!(find_amount(text).is_none(), "{text} is a rate, not a line total");
        }
        let items = vec![item("i1", "part", "mulch"), item("i2", "price", "mulch $95/yd")];
        let folded = fold_spoken_prices(&items);
        assert!(folded.amounts.is_empty(), "a rate prices nothing");
        assert_eq!(folded.items.len(), 2, "and the rate stays visible, in the operator's words");
        assert_eq!(folded.items[1].text, "mulch $95/yd");
    }

    #[test]
    fn two_amounts_in_one_item_are_left_alone() {
        assert!(find_amount("$200 mulch and $100 compost").is_none());
        let items = vec![item("i1", "price", "$200 mulch and $100 compost")];
        let folded = fold_spoken_prices(&items);
        assert!(folded.amounts.is_empty());
        assert_eq!(folded.items[0].text, "$200 mulch and $100 compost", "untouched");
    }

    #[test]
    fn bare_numbers_and_document_numbers_are_not_money() {
        assert!(find_amount("5 yards mulch").is_none(), "a quantity is not a price");
        assert!(find_amount("EST-0047 reissue").is_none());
        assert!(find_amount("replace 3x4 post").is_none());
    }

    #[test]
    fn items_without_money_pass_through_untouched() {
        let items = vec![item("i1", "todo", "haul the old bark away"), item("i2", "safety", "loose railing")];
        let folded = fold_spoken_prices(&items);
        assert!(folded.amounts.is_empty());
        assert_eq!(folded.items.len(), 2);
        assert_eq!(folded.items[0].text, "haul the old bark away");
    }

    #[test]
    fn a_price_statement_is_never_a_target_for_another_price() {
        // "$100 compost" must not collect "$200 compost" — a price does not
        // price a price.
        let items = vec![item("i1", "price", "$100 compost"), item("i2", "price", "$200 compost")];
        let folded = fold_spoken_prices(&items);
        assert_eq!(folded.items.len(), 2);
        assert_eq!(folded.amounts.get("i1"), Some(&10000));
        assert_eq!(folded.amounts.get("i2"), Some(&20000));
    }

    #[test]
    fn a_bare_amount_with_no_label_keeps_the_operators_text() {
        let items = vec![item("i1", "todo", "mulch the beds"), item("i2", "price", "$500")];
        let folded = fold_spoken_prices(&items);
        assert_eq!(folded.items.len(), 2, "no label, nothing to match on");
        assert_eq!(folded.items[1].text, "$500", "the title is all it said");
        assert_eq!(folded.amounts.get("i2"), Some(&50000));
    }

    #[test]
    fn unfolded_keeps_everything() {
        let items = vec![item("i1", "price", "$200 mulch"), item("i2", "part", "mulch")];
        let folded = FoldedPrices::unfolded(&items);
        assert_eq!(folded.items.len(), 2);
        assert!(folded.amounts.is_empty());
    }
}
