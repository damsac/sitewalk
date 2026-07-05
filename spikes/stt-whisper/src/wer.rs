// Table 3: WER + target-term recall, and the `initial_prompt` biasing experiment.
//
// One hand-rolled, unit-tested token-level Levenshtein lives here; `stream.rs` reuses
// `token_edit_distance` for boundary re-transcription (no second diff implementation).

use crate::{decode, load_wav_16k_mono, make_ctx, model_label};
use std::collections::HashMap;

/// Lowercase, strip surrounding punctuation, split on whitespace into word tokens.
pub fn tokenize(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric() || *c == '\'')
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Token-level Levenshtein edit distance (substitution/insertion/deletion each cost 1).
pub fn token_edit_distance(a: &[String], b: &[String]) -> usize {
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        cur[0] = i;
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m]
}

/// WER = token edit distance ÷ reference token count.
pub fn wer(reference: &str, hypothesis: &str) -> f64 {
    let r = tokenize(reference);
    let h = tokenize(hypothesis);
    if r.is_empty() {
        return 0.0;
    }
    token_edit_distance(&r, &h) as f64 / r.len() as f64
}

/// Does the contiguous token sequence `needle` appear anywhere in `haystack`?
fn contains_seq(haystack: &[String], needle: &[String]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return needle.is_empty();
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Target-term recall: of the curated terms that appear in the reference, what fraction also
/// appear in the hypothesis? Returns (found_in_hyp, present_in_ref, recall).
pub fn term_recall(reference: &str, hypothesis: &str, terms: &[String]) -> (usize, usize, f64) {
    let r = tokenize(reference);
    let h = tokenize(hypothesis);
    let mut present = 0usize;
    let mut found = 0usize;
    for term in terms {
        let t = tokenize(term);
        if t.is_empty() {
            continue;
        }
        if contains_seq(&r, &t) {
            present += 1;
            if contains_seq(&h, &t) {
                found += 1;
            }
        }
    }
    let recall = if present == 0 { 0.0 } else { found as f64 / present as f64 };
    (found, present, recall)
}

/// Heuristic hallucination flag: runaway repetition or gross over-generation vs. reference.
/// `initial_prompt` used as a keyword list is known to induce repetition loops (survey §4).
pub fn hallucination_flag(reference: &str, hypothesis: &str) -> Option<String> {
    let r = tokenize(reference);
    let h = tokenize(hypothesis);
    // (a) gross length blow-up
    if !r.is_empty() && h.len() as f64 > 1.5 * r.len() as f64 {
        return Some(format!("output {}x reference length", h.len() / r.len().max(1)));
    }
    // (b) long consecutive-token repetition loop
    let mut run = 1usize;
    let mut max_run = 1usize;
    for w in h.windows(2) {
        if w[0] == w[1] {
            run += 1;
            max_run = max_run.max(run);
        } else {
            run = 1;
        }
    }
    if max_run >= 5 {
        return Some(format!("repetition loop (token repeated {max_run}x)"));
    }
    None
}

// ---- deterministic white-noise injection (no rand crate; fixed-seed xorshift) ----
fn add_white_noise(samples: &mut [f32], snr_db: f64) {
    // signal power
    let sig_pow: f64 =
        samples.iter().map(|&s| (s as f64) * (s as f64)).sum::<f64>() / samples.len().max(1) as f64;
    let noise_pow = sig_pow / 10f64.powf(snr_db / 10.0);
    let amp = noise_pow.sqrt();
    let mut state: u64 = 0x9E3779B97F4A7C15; // fixed seed → reproducible noise
    for s in samples.iter_mut() {
        // xorshift64 → uniform in [-1,1], scaled to target noise amplitude
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let u = (state >> 11) as f64 / (1u64 << 53) as f64; // [0,1)
        let noise = (u * 2.0 - 1.0) * amp * 1.732; // *sqrt(3): uniform var → power=amp^2
        *s = (*s as f64 + noise) as f32;
    }
}

fn load_audio(flags: &HashMap<String, String>) -> (Vec<f32>, f64, String) {
    let audio = flags.get("audio").expect("--audio required");
    let (mut samples, dur) = load_wav_16k_mono(audio);
    let cond = if let Some(snr) = flags.get("snr") {
        let db: f64 = snr.parse().expect("--snr must be a number (dB)");
        add_white_noise(&mut samples, db);
        format!("noisy(+{db:.0}dB SNR, synthetic)")
    } else {
        "quiet".to_string()
    };
    (samples, dur, cond)
}

fn read_terms(path: &str) -> Vec<String> {
    std::fs::read_to_string(path)
        .expect("read terms")
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

pub fn run_accuracy(flags: &HashMap<String, String>) {
    let model = flags.get("model").expect("--model required");
    let reference = std::fs::read_to_string(flags.get("reference").expect("--reference required"))
        .expect("read reference");
    let (samples, _dur, cond) = load_audio(flags);
    let clip = flags.get("audio").unwrap().rsplit('/').next().unwrap().to_string();

    let ctx = make_ctx(model);
    let _warm = decode(&ctx, &samples, None);
    let d = decode(&ctx, &samples, None);
    let w = wer(&reference, &d.text) * 100.0;
    let halluc = hallucination_flag(&reference, &d.text)
        .map(|s| format!("YES ({s})"))
        .unwrap_or_else(|| "no".to_string());
    let label = model_label(model);

    eprintln!("[accuracy] {label} {clip} {cond}: WER={w:.1}%  halluc={halluc}");
    eprintln!("[accuracy] hyp: {}", d.text);
    // Table 3 row (recall columns filled by `bias`)
    println!("| {label} | {clip} | {cond} | {w:.1} | — | — | — | {halluc} | WER only (see bias run for recall) |");
}

pub fn run_bias(flags: &HashMap<String, String>) {
    let model = flags.get("model").expect("--model required");
    let reference = std::fs::read_to_string(flags.get("reference").expect("--reference required"))
        .expect("read reference");
    let terms = read_terms(flags.get("terms").expect("--terms required"));
    let (samples, _dur, cond) = load_audio(flags);
    let clip = flags.get("audio").unwrap().rsplit('/').next().unwrap().to_string();

    // Inject the curated vocabulary as an initial_prompt (comma-separated keyword list).
    let prompt = terms.join(", ");

    let ctx = make_ctx(model);
    let _warm = decode(&ctx, &samples, None);
    let base = decode(&ctx, &samples, None);
    let biased = decode(&ctx, &samples, Some(&prompt));

    let (_bf, bp, base_recall) = term_recall(&reference, &base.text, &terms);
    let (_pf, _pp, bias_recall) = term_recall(&reference, &biased.text, &terms);
    let delta_pp = (bias_recall - base_recall) * 100.0;
    let base_wer = wer(&reference, &base.text) * 100.0;
    let halluc = hallucination_flag(&reference, &biased.text)
        .map(|s| format!("YES ({s})"))
        .unwrap_or_else(|| "no".to_string());
    let label = model_label(model);

    eprintln!(
        "[bias] {label} {clip} {cond}: terms-in-ref={bp}  recall no-bias={:.0}%  recall prompt={:.0}%  Δ={delta_pp:+.0}pp  WER={base_wer:.1}%  halluc(biased)={halluc}",
        base_recall * 100.0,
        bias_recall * 100.0
    );
    println!(
        "| {label} | {clip} | {cond} | {base_wer:.1} | {:.0}% ({}/{}) | {:.0}% | {delta_pp:+.0} | {halluc} | initial_prompt = {} curated terms |",
        base_recall * 100.0,
        _bf,
        bp,
        bias_recall * 100.0,
        terms.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<String> {
        tokenize(s)
    }

    #[test]
    fn edit_distance_basics() {
        assert_eq!(token_edit_distance(&toks("a b c"), &toks("a b c")), 0);
        assert_eq!(token_edit_distance(&toks("a b c"), &toks("a x c")), 1); // sub
        assert_eq!(token_edit_distance(&toks("a b c"), &toks("a b")), 1); // del
        assert_eq!(token_edit_distance(&toks("a b"), &toks("a b c")), 1); // ins
        assert_eq!(token_edit_distance(&toks(""), &toks("a b c")), 3);
    }

    #[test]
    fn wer_known() {
        // reference 4 tokens, 1 substitution → 25%
        assert!((wer("the quick brown fox", "the quick red fox") - 0.25).abs() < 1e-9);
        // punctuation + case normalized away → perfect
        assert_eq!(wer("The Quick, Brown Fox.", "the quick brown fox"), 0.0);
    }

    #[test]
    fn term_recall_basics() {
        let terms = vec!["french drain".to_string(), "GFCI".to_string(), "joist".to_string()];
        // ref has all three; hyp drops "french drain" and mangles GFCI
        let (found, present, recall) = term_recall(
            "the french drain and the GFCI and a joist",
            "the french and the gfci and a joist",
            &terms,
        );
        assert_eq!(present, 3);
        assert_eq!(found, 2); // gfci + joist present; "french drain" (2-gram) missing
        assert!((recall - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn hallucination_detects_repetition() {
        assert!(hallucination_flag("a b c", "x x x x x x").is_some());
        assert!(hallucination_flag("a b c", "a b c").is_none());
    }
}
