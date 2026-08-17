//! The speech-accuracy eval: audio → text, the one layer no other axis can
//! see.
//!
//! Gated twice over, because it needs a native stack AND a model file:
//!
//! ```sh
//! scripts/make-asr-fixtures.sh
//! MURMUR_WHISPER_MODEL=apps/ios/Sources/Resources/ggml-small.en-q5_1.bin \
//!   cargo test -p evals --features stt-eval --test stt_accuracy -- --ignored --nocapture
//! ```
//!
//! It drives `SttStream` — the SHIPPING path, chunker and LocalAgreement
//! finalizer included — rather than calling the decoder directly, because a
//! word can also be lost between windows, and an eval that skipped the
//! streaming layer would score a defect it cannot see as a perfect decode.
//!
//! Whisper runs at temperature 0 and this path is deterministic: three runs
//! of this corpus came back byte-identical. So unlike the LLM axes — where a
//! defect and variance take three runs to tell apart — one run here is a
//! valid comparison, and a number that moves means something changed.
//!
//! Every case with vocabulary terms is decoded TWICE, unbiased and biased.
//! That is the actionable number: if adding "weed eating" to the operator's
//! vocabulary recovers the term, the fix for #357 is a vocabulary entry and
//! not a model change.

#![cfg(feature = "stt-eval")]

use std::path::{Path, PathBuf};

use evals::asr::{grade_asr, summarize, AsrCase, AsrScore};
use stt::{SttConfig, SttStream};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/asr")
}

fn cases() -> Vec<AsrCase> {
    let body = std::fs::read_to_string(corpus_dir().join("manifest.json"))
        .expect("the asr manifest is checked in");
    let v: serde_json::Value = serde_json::from_str(&body).expect("manifest parses");
    serde_json::from_value(v["cases"].clone()).expect("cases parse")
}

/// Minimal 16-bit PCM WAV reader — chunk-walking rather than assuming a
/// 44-byte header, because `afconvert` writes a LIST chunk and a fixed offset
/// would read metadata as audio and "measure" a decode of noise.
fn read_wav_f32(path: &Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    assert!(bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE",
            "{} is not a RIFF/WAVE file", path.display());
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let size = u32::from_le_bytes(bytes[i + 4..i + 8].try_into().unwrap()) as usize;
        let start = i + 8;
        if id == b"data" {
            let end = (start + size).min(bytes.len());
            return bytes[start..end]
                .chunks_exact(2)
                .map(|s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0)
                .collect();
        }
        i = start + size + (size & 1); // chunks are word-aligned
    }
    panic!("no data chunk in {}", path.display());
}

/// One decode through the shipping stream, in real-time-sized pushes so the
/// chunker and finalizer behave as they do on a device.
fn decode(model: &Path, pcm: &[f32], vocabulary: &[String]) -> String {
    let cfg = SttConfig { use_gpu: false, ..Default::default() };
    let push = cfg.sample_rate as usize / 10; // 100 ms
    let stream = SttStream::with_model(model, cfg, vocabulary).expect("model loads");

    let mut text = String::new();
    let mut append = |segments: Vec<stt::FinalizedSegment>| {
        for s in segments {
            text.push_str(&s.text);
            text.push(' ');
        }
    };
    for block in pcm.chunks(push) {
        stream.push_pcm(block);
        append(stream.poll().expect("decode"));
    }
    append(stream.end().expect("flush"));
    text
}

#[test]
#[ignore = "needs a whisper model (MURMUR_WHISPER_MODEL) and generated fixtures"]
fn speech_accuracy_over_the_corpus() {
    let model = PathBuf::from(
        std::env::var("MURMUR_WHISPER_MODEL")
            .expect("set MURMUR_WHISPER_MODEL to a ggml-*.bin path"),
    );
    let dir = corpus_dir();
    let cases = cases();
    let mut missing = Vec::new();
    let mut plain: Vec<AsrScore> = Vec::new();
    let mut biased: Vec<AsrScore> = Vec::new();

    for case in &cases {
        let audio = dir.join(&case.audio);
        if !audio.exists() {
            missing.push(case.audio.clone());
            continue;
        }
        let pcm = read_wav_f32(&audio);
        let heard = decode(&model, &pcm, &[]);
        let score = grade_asr(&case.reference, &heard, &case.critical);
        println!("\n===== {} =====", case.id);
        println!("  heard: {}", heard.trim());
        println!(
            "  critical {}/{}   wer {:.3}",
            score.kept.len(),
            score.kept.len() + score.lost.len(),
            score.wer
        );
        for term in &score.lost {
            println!("  LOST   \"{term}\"");
        }

        if !case.vocabulary.is_empty() {
            let heard = decode(&model, &pcm, &case.vocabulary);
            let with_bias = grade_asr(&case.reference, &heard, &case.critical);
            let recovered: Vec<&String> =
                score.lost.iter().filter(|t| with_bias.kept.contains(t)).collect();
            println!(
                "  biased: critical {}/{}   wer {:.3}   recovered {:?}",
                with_bias.kept.len(),
                with_bias.kept.len() + with_bias.lost.len(),
                with_bias.wer,
                recovered
            );
            biased.push(with_bias);
        }
        plain.push(score);
    }

    assert!(
        !plain.is_empty(),
        "no fixture audio found in {} — run scripts/make-asr-fixtures.sh (missing: {missing:?})",
        dir.display()
    );

    let suite = summarize(&plain);
    println!(
        "\n----- unbiased: {}/{} clean   recall {:.3}   wer {:.3} -----",
        suite.clean, suite.cases, suite.mean_critical_recall, suite.mean_wer
    );
    if !suite.lost.is_empty() {
        println!("lost: {:?}", suite.lost);
    }
    if !biased.is_empty() {
        let b = summarize(&biased);
        println!(
            "----- biased:   {}/{} clean   recall {:.3}   wer {:.3} -----",
            b.clean, b.cases, b.mean_critical_recall, b.mean_wer
        );
        if !b.lost.is_empty() {
            println!("still lost: {:?}", b.lost);
        }
    }

    // No threshold. A recall floor picked before the first baseline exists is
    // a number invented to be met; the report is the deliverable, and the
    // floor gets set once there is a real recording corpus to set it from.
    assert!((0.0..=1.0).contains(&suite.mean_critical_recall));
}
