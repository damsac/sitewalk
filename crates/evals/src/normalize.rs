//! Deterministic text normalization + set similarity for the grader. No
//! network, no randomness: same input → same output, every run. This is what
//! makes eval scores comparable across prompt variants.
//!
//! Numeric normalization (locked-metric rule, unit-tested below): trade
//! transcripts and their faithful paraphrases mix spelled-out numbers,
//! digits, currency, and dimension shorthand ("twelve hundred" / "$1,200",
//! "sixteen-foot two-by-tens" / "16' 2x10s"). Without folding these to a
//! shared canonical form, a faithful paraphrase scores well under the Dice
//! match threshold on token overlap alone. The canonical form is: bare
//! decimal digits, no separators — "1200", "16", "10", never "1,200" or
//! "sixteen". `x`/`by` (dimension multiplication) and a bare `hundred`
//! that failed to merge are treated as noise and dropped, same as a
//! stopword — they carry no signal once the numbers on either side are
//! canonical.

use std::collections::BTreeSet;

/// A tiny, closed stopword list — words that carry no extraction signal and
/// only add noise to overlap. Kept small and fixed on purpose: a big list would
/// swallow real content ("no", "not"). Do NOT tune this per-corpus.
///
/// `by` and `x` are the two spellings of a dimension's multiplication sign
/// ("two-by-ten" / "2x10") — noise once the numbers on either side are
/// canonicalized. `hundred` is consumed by `combine_number_words` when it
/// follows a numeral; a `hundred` that reaches this filter unmerged (no
/// leading numeral) is also noise, not a countable quantity on its own.
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "to", "of", "for", "and", "or", "is", "are", "was", "were",
    "on", "in", "at", "we", "i", "it", "that", "this", "with", "need", "needs",
    "by", "x", "hundred",
];

/// English number words this grader understands: ones 0–19 and tens 20–90.
/// Deliberately small — a Dice-matching aid for trade-jargon prices and
/// dimensions, not a general number parser. Compound forms beyond
/// "<ones> hundred [<tens-or-ones>]" (e.g. "twelve hundred fifty") are not
/// needed by the corpus and are out of scope.
fn word_to_num(w: &str) -> Option<u64> {
    Some(match w {
        "zero" => 0,
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        "eleven" => 11,
        "twelve" => 12,
        "thirteen" => 13,
        "fourteen" => 14,
        "fifteen" => 15,
        "sixteen" => 16,
        "seventeen" => 17,
        "eighteen" => 18,
        "nineteen" => 19,
        "twenty" => 20,
        "thirty" => 30,
        "forty" => 40,
        "fifty" => 50,
        "sixty" => 60,
        "seventy" => 70,
        "eighty" => 80,
        "ninety" => 90,
        _ => return None,
    })
}

/// Removes a thousands-separator comma ("1,200" → "1200") and splits a
/// digit-`x`-digit dimension multiplier ("2x10" → "2 x 10") so the generic
/// alphanumeric splitter below treats each number as its own token. Both
/// rewrites only fire when flanked by digits on both sides, so ordinary
/// words and punctuation are untouched. Currency symbols ("$") need no
/// special handling — they're already non-alphanumeric and fall out in the
/// generic split.
fn rewrite_numeric_punctuation(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        let prev_digit = i > 0 && chars[i - 1].is_ascii_digit();
        let next_digit = i + 1 < chars.len() && chars[i + 1].is_ascii_digit();
        if c == ',' && prev_digit && next_digit {
            continue; // drop thousands separator
        }
        if c == 'x' && prev_digit && next_digit {
            out.push(' ');
            out.push('x');
            out.push(' ');
            continue;
        }
        out.push(c);
    }
    out
}

/// Converts individual number words to digits ("sixteen" → "16"), then merges
/// a trailing literal "hundred" into its preceding numeral ("twelve hundred"
/// → "1200"), optionally absorbing one more <100 numeral right after it
/// ("twelve hundred fifty" → "1250"). Word→digit mapping runs first so the
/// merge only has to look for a digit token followed by the word "hundred".
fn combine_number_words(tokens: Vec<String>) -> Vec<String> {
    let mapped: Vec<String> = tokens
        .into_iter()
        .map(|t| match word_to_num(&t) {
            Some(n) => n.to_string(),
            None => t,
        })
        .collect();

    fn is_number(s: &str) -> bool {
        !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
    }

    let mut out = Vec::with_capacity(mapped.len());
    let mut i = 0;
    while i < mapped.len() {
        if is_number(&mapped[i]) && mapped.get(i + 1).map(String::as_str) == Some("hundred") {
            let mut val: u64 = mapped[i].parse().unwrap_or(0) * 100;
            let mut consumed = 2;
            if let Some(next) = mapped.get(i + 2) {
                if is_number(next) {
                    if let Ok(n2) = next.parse::<u64>() {
                        if n2 < 100 {
                            val += n2;
                            consumed = 3;
                        }
                    }
                }
            }
            out.push(val.to_string());
            i += consumed;
        } else {
            out.push(mapped[i].clone());
            i += 1;
        }
    }
    out
}

/// Lowercase, canonicalize numbers (spelled words → digits, thousands
/// separators and dimension `x` split out), strip a trailing plural `s`
/// (word or bare numeral), drop stopwords, and collect into a set. Returns a
/// `BTreeSet` for deterministic iteration order (matters only for debug
/// output; scores are set ops).
pub fn token_set(s: &str) -> BTreeSet<String> {
    let rewritten = rewrite_numeric_punctuation(&s.to_lowercase());
    let raw: Vec<String> = rewritten
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(strip_plural)
        .collect();
    combine_number_words(raw)
        .into_iter()
        .filter(|w| !STOPWORDS.contains(&w.as_str()))
        .collect()
}

/// Strip a single trailing plural `s`: for word tokens, `joists`→`joist`, but
/// not for 2-char words (`as`→`a` would be wrong) or double-`s` (`loss`→`los`
/// would be wrong). For a bare numeral, any length is safe — `10s`→`10`,
/// `2s`→`2` — there's no "as"-style ambiguity once every character is a digit.
fn strip_plural(w: &str) -> String {
    if w.len() > 3 && w.ends_with('s') && !w.ends_with("ss") {
        return w[..w.len() - 1].to_string();
    }
    if let Some(base) = w.strip_suffix('s') {
        if !base.is_empty() && base.chars().all(|c| c.is_ascii_digit()) {
            return base.to_string();
        }
    }
    w.to_string()
}

/// Dice coefficient: `2·|A∩B| / (|A|+|B|)`. Symmetric, order-independent, in
/// `[0,1]`. Empty-vs-anything is 0.0 (never NaN).
pub fn dice(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    let total = a.len() + b.len();
    if total == 0 {
        return 0.0;
    }
    let inter = a.intersection(b).count();
    (2.0 * inter as f64) / total as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_lowercases_strips_punct_and_stopwords() {
        // "Order the lumber!" -> {order, lumber}  (the/for/a dropped, "!" gone)
        let t = token_set("Order the lumber!");
        assert!(t.contains("order"));
        assert!(t.contains("lumber"));
        assert!(!t.contains("the"));
    }

    #[test]
    fn normalize_strips_trailing_plural_s() {
        assert_eq!(token_set("joists"), token_set("joist"));
    }

    #[test]
    fn dice_is_one_for_identical_sets() {
        assert_eq!(dice(&token_set("order lumber"), &token_set("order lumber")), 1.0);
    }

    #[test]
    fn dice_is_order_independent() {
        let a = dice(&token_set("order the lumber"), &token_set("lumber order"));
        assert_eq!(a, 1.0, "stopword-stripped token SETS are equal regardless of order");
    }

    #[test]
    fn dice_is_zero_for_disjoint_sets() {
        assert_eq!(dice(&token_set("order lumber"), &token_set("call framer")), 0.0);
    }

    #[test]
    fn dice_partial_overlap_is_between() {
        // {order,lumber,deck} vs {order,lumber} -> 2*2/(3+2) = 0.8
        let d = dice(&token_set("order lumber deck"), &token_set("order lumber"));
        assert!((d - 0.8).abs() < 1e-9, "got {d}");
    }

    #[test]
    fn empty_sets_score_zero_not_nan() {
        assert_eq!(dice(&token_set(""), &token_set("")), 0.0);
        assert_eq!(dice(&token_set("order"), &token_set("")), 0.0);
    }

    #[test]
    fn spelled_number_words_normalize_to_digits() {
        assert_eq!(token_set("sixteen"), token_set("16"));
        assert_eq!(token_set("ninety dollars"), token_set("90 dollars"));
    }

    #[test]
    fn hundred_merges_with_its_leading_numeral() {
        assert_eq!(token_set("twelve hundred dollars"), token_set("1200 dollars"));
    }

    #[test]
    fn currency_and_thousands_separator_are_stripped() {
        assert_eq!(token_set("$1,200"), token_set("1200"));
    }

    #[test]
    fn dimension_shorthand_matches_spelled_out_form() {
        // "2x10" and "two-by-ten" both canonicalize to the shared token set
        // {2, 10} once `x`/`by` are treated as separator noise.
        assert_eq!(token_set("2x10"), token_set("two-by-ten"));
        assert_eq!(token_set("2x10s"), token_set("two-by-tens"));
    }

    #[test]
    fn reviewer_case_price_paraphrase_matches_above_threshold() {
        // Ground truth (rambling_long_walk fixture) vs a faithful paraphrase
        // that uses digits/currency/abbreviation instead of spelled-out
        // numbers. Before the numeric-normalization fix this scored ~0.47
        // (below the 0.5 match threshold); the fix must clear it.
        let truth = token_set(
            "roughly twelve hundred dollars for the water heater swap including venting changes",
        );
        let candidate = token_set("$1,200 for water heater swap incl. venting");
        let d = dice(&truth, &candidate);
        assert!(d >= 0.5, "expected >= 0.5, got {d}");
    }

    #[test]
    fn reviewer_case_dimension_paraphrase_matches_above_threshold() {
        // Ground truth (deck_walk_contacts fixture) vs a faithful paraphrase
        // using dimension shorthand instead of spelled-out numbers. Before
        // the fix this scored ~0.31.
        let truth = token_set("two sixteen-foot pressure-treated two-by-tens");
        let candidate = token_set("16' 2x10s");
        let d = dice(&truth, &candidate);
        assert!(d >= 0.5, "expected >= 0.5, got {d}");
    }
}
