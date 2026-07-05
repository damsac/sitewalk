// Table 2: chunked pseudo-streaming + append-only finalize derivation.
//
// Feeds the clip in N-second sliding windows with a small overlap, measures how much text is
// re-transcribed across chunk boundaries (token edit distance, reusing wer.rs — one Levenshtein),
// derives an append-only finalized stream (the Plan 05 cursor contract), and measures the
// audio-time finalize latency that contract costs.

use crate::wer::{token_edit_distance, tokenize, wer};
use crate::{decode, load_wav_16k_mono, make_ctx};
use std::collections::HashMap;

const SR: usize = 16_000;

/// One decoded segment, timestamps in absolute audio seconds.
#[derive(Clone, Debug)]
pub struct Seg {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// What one chunk emitted, plus the audio-time at which the chunk ends (= when its data is
/// available in a real-time stream).
#[derive(Clone, Debug)]
pub struct ChunkEmission {
    pub chunk_end: f64,
    pub segs: Vec<Seg>,
}

pub struct FinalizeResult {
    pub finalized: Vec<Seg>,
    /// audio-time delay per finalized seg: (finalizing chunk end) − (seg end)
    pub latencies: Vec<f64>,
}

/// Append-only finalize rule (pure, unit-tested).
///
/// A segment is finalized once it lies fully behind the *horizon* of some chunk — i.e. its end
/// is older than (chunk_end − overlap), so no later chunk will revisit that audio. We commit each
/// finalized segment exactly once (dedup by monotonically advancing `committed_until`), which
/// guarantees the emitted stream is append-only: no previously-emitted word is ever revised.
/// The final chunk flushes its tail (no successor to confirm it).
pub fn finalize(chunks: &[ChunkEmission], overlap: f64) -> FinalizeResult {
    let mut finalized: Vec<Seg> = Vec::new();
    let mut latencies: Vec<f64> = Vec::new();
    let mut committed_until = 0.0f64;
    let eps = 1e-6;
    let last = chunks.len().saturating_sub(1);
    for (i, ch) in chunks.iter().enumerate() {
        // Last chunk: nothing comes after, so everything is safe to finalize.
        let horizon = if i == last { f64::INFINITY } else { ch.chunk_end - overlap };
        for seg in &ch.segs {
            if seg.end <= horizon + eps && seg.end > committed_until + eps {
                latencies.push((ch.chunk_end - seg.end).max(0.0));
                committed_until = seg.end;
                finalized.push(seg.clone());
            }
        }
    }
    FinalizeResult { finalized, latencies }
}

/// Reassemble a complete transcript by concatenating each chunk's text and deduplicating the
/// overlap via longest suffix/prefix token match (a lightweight LocalAgreement-style merge —
/// the technique whisper_streaming uses). Still append-only (only ever appends), but recovers
/// the content the naive time-horizon rule drops. Used to fairly attribute streaming loss to the
/// engine vs. the finalize rule.
pub fn reassemble_dedup(chunks: &[ChunkEmission]) -> String {
    let mut committed: Vec<String> = Vec::new();
    for ch in chunks {
        let chunk_text: String =
            ch.segs.iter().map(|s| s.text.clone()).collect::<Vec<_>>().join(" ");
        let new_toks = tokenize(&chunk_text);
        if committed.is_empty() {
            committed = new_toks;
            continue;
        }
        // Find longest k where committed tail == new head (k up to ~40 tokens).
        let maxk = committed.len().min(new_toks.len()).min(40);
        let mut best = 0;
        for k in (1..=maxk).rev() {
            if committed[committed.len() - k..] == new_toks[..k] {
                best = k;
                break;
            }
        }
        committed.extend_from_slice(&new_toks[best..]);
    }
    committed.join(" ")
}

/// Boundary re-transcription for one chunk pair: token edit distance over the text each chunk
/// emitted inside the shared overlap window, normalized by the larger token count → a %.
fn boundary_retrans(prev: &ChunkEmission, next: &ChunkEmission, overlap: f64) -> f64 {
    let win_start = prev.chunk_end - overlap;
    let win_end = prev.chunk_end;
    let text_in = |segs: &[Seg]| -> Vec<String> {
        let joined: String = segs
            .iter()
            .filter(|s| s.end > win_start && s.start < win_end)
            .map(|s| s.text.clone())
            .collect::<Vec<_>>()
            .join(" ");
        tokenize(&joined)
    };
    let a = text_in(&prev.segs);
    let b = text_in(&next.segs);
    let denom = a.len().max(b.len());
    if denom == 0 {
        return 0.0;
    }
    token_edit_distance(&a, &b) as f64 / denom as f64 * 100.0
}

/// Decode the clip chunk-by-chunk, returning per-chunk emissions with absolute timestamps.
fn decode_chunks(model: &str, samples: &[f32], chunk: f64, overlap: f64) -> Vec<ChunkEmission> {
    let ctx = make_ctx(model);
    // warm-up (Metal shader JIT) on the first chunk-sized slice
    let warm_len = ((chunk * SR as f64) as usize).min(samples.len());
    let _ = decode(&ctx, &samples[..warm_len], None);

    let step = (chunk - overlap).max(0.1);
    let total = samples.len() as f64 / SR as f64;
    let mut emissions = Vec::new();
    let mut start = 0.0f64;
    while start < total {
        let s_idx = (start * SR as f64) as usize;
        let e_idx = (((start + chunk) * SR as f64) as usize).min(samples.len());
        if s_idx >= e_idx {
            break;
        }
        let d = decode(&ctx, &samples[s_idx..e_idx], None);
        let chunk_end = e_idx as f64 / SR as f64;
        let segs = d
            .segments
            .iter()
            .map(|(t0, t1, txt)| Seg {
                start: start + *t0 as f64 / 100.0, // centiseconds → sec, offset by chunk start
                end: start + *t1 as f64 / 100.0,
                text: txt.trim().to_string(),
            })
            .collect();
        emissions.push(ChunkEmission { chunk_end, segs });
        start += step;
    }
    emissions
}

pub fn run(flags: &HashMap<String, String>) {
    let model = flags.get("model").expect("--model required");
    let audio = flags.get("audio").expect("--audio required");
    let chunk: f64 = flags.get("chunk").map(|s| s.parse().unwrap()).unwrap_or(5.0);
    let overlap: f64 = flags.get("overlap").map(|s| s.parse().unwrap()).unwrap_or(1.0);

    let (samples, _dur) = load_wav_16k_mono(audio);
    let emissions = decode_chunks(model, &samples, chunk, overlap);

    // boundary re-transcription, averaged over chunk pairs
    let mut retrans = Vec::new();
    for pair in emissions.windows(2) {
        retrans.push(boundary_retrans(&pair[0], &pair[1], overlap));
    }
    let avg_retrans = if retrans.is_empty() {
        0.0
    } else {
        retrans.iter().sum::<f64>() / retrans.len() as f64
    };

    let fr = finalize(&emissions, overlap);
    let max_lat = fr.latencies.iter().cloned().fold(0.0, f64::max);
    let avg_lat = if fr.latencies.is_empty() {
        0.0
    } else {
        fr.latencies.iter().sum::<f64>() / fr.latencies.len() as f64
    };

    // Reassembled finalized text (append-only stream)
    let final_text: String = fr
        .finalized
        .iter()
        .map(|s| s.text.clone())
        .collect::<Vec<_>>()
        .join(" ");

    // Completeness cost: WER of the reassembled finalized (append-only) stream vs reference.
    // This exposes the loss a naive time-horizon finalize suffers — the number that matters for
    // the Plan 05 cursor contract, not just latency.
    let dedup_text = reassemble_dedup(&emissions);
    let (naive_note, dedup_note) = match flags.get("reference") {
        Some(path) => {
            let reference = std::fs::read_to_string(path).expect("read reference");
            let naive_w = wer(&reference, &final_text) * 100.0;
            let dedup_w = wer(&reference, &dedup_text) * 100.0;
            (format!("naive-finalize WER {naive_w:.0}%"), format!("dedup-reassembly WER {dedup_w:.0}%"))
        }
        None => ("no --reference".to_string(), "no --reference".to_string()),
    };

    eprintln!(
        "[stream] chunk={chunk}s overlap={overlap}s: {} chunks, {} finalized segs, boundary re-transcription avg {avg_retrans:.0}%, finalize latency avg {avg_lat:.2}s max {max_lat:.2}s | {naive_note} | {dedup_note}",
        emissions.len(),
        fr.finalized.len()
    );
    eprintln!("[stream] naive-finalized stream: {final_text}");
    eprintln!("[stream] dedup-reassembled stream: {dedup_text}");

    // Table 2 row: Chunk | Overlap | Boundary re-transcription % | Finalize latency (s) | Append-only derivable? | Notes
    println!(
        "| {chunk:.1} | {overlap:.1} | {avg_retrans:.0} | {max_lat:.2} (max), {avg_lat:.2} (avg) | invariant yes (unit-tested) | {naive_note}; {dedup_note} |"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f64, end: f64, text: &str) -> Seg {
        Seg { start, end, text: text.to_string() }
    }

    /// The core contract: the finalized stream is append-only — a word committed by an early
    /// chunk is never revised by a later chunk, even when the later chunk re-transcribes the
    /// overlap region differently.
    #[test]
    fn finalized_stream_is_append_only() {
        // chunk0 ends at 5s (horizon 4s w/ overlap 1): "the french drain" spans 0..4
        // chunk1 ends at 9s, re-transcribes 4..5 overlap DIFFERENTLY ("drane") + new "is backing"
        let chunks = vec![
            ChunkEmission {
                chunk_end: 5.0,
                segs: vec![seg(0.0, 3.5, "the french drain"), seg(3.6, 4.8, "along the")],
            },
            ChunkEmission {
                chunk_end: 9.0,
                // note: re-emits 3.6..4.8 as a DIFFERENT string — must NOT overwrite committed
                segs: vec![seg(3.6, 4.8, "a long the WRONG"), seg(5.0, 8.0, "is backing up")],
            },
        ];
        let fr = finalize(&chunks, 1.0);
        let text: Vec<&str> = fr.finalized.iter().map(|s| s.text.as_str()).collect();
        // chunk0 finalizes seg ending 3.5 (< horizon 4.0). seg ending 4.8 is in overlap zone,
        // deferred. chunk1 (last) flushes: seg 3.6..4.8 committed from chunk1's version, then 5..8.
        assert_eq!(text, vec!["the french drain", "a long the WRONG", "is backing up"]);
        // Append-only: end times strictly increase (no revision of already-committed audio).
        let mut prev = -1.0;
        for s in &fr.finalized {
            assert!(s.end > prev, "finalized stream must advance monotonically");
            prev = s.end;
        }
    }

    #[test]
    fn dedup_reassembly_merges_overlap() {
        // Two chunks whose texts share an overlapping tail/head — dedup must not duplicate it.
        let chunks = vec![
            ChunkEmission { chunk_end: 5.0, segs: vec![seg(0.0, 4.0, "the french drain is backing")] },
            ChunkEmission { chunk_end: 9.0, segs: vec![seg(3.0, 8.0, "is backing up badly today")] },
        ];
        assert_eq!(reassemble_dedup(&chunks), "the french drain is backing up badly today");
    }

    #[test]
    fn no_double_emit_of_overlap() {
        // Same segment emitted by two consecutive chunks must be committed once.
        let chunks = vec![
            ChunkEmission { chunk_end: 5.0, segs: vec![seg(0.0, 2.0, "hello world")] },
            ChunkEmission {
                chunk_end: 9.0,
                segs: vec![seg(0.0, 2.0, "hello world"), seg(4.0, 6.0, "again now")],
            },
        ];
        let fr = finalize(&chunks, 1.0);
        let text: Vec<&str> = fr.finalized.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(text, vec!["hello world", "again now"]);
    }
}
