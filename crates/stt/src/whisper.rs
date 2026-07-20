use std::path::Path;
use std::sync::Arc;

use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
};

use whisper_rs::WhisperSegment;

use crate::decoder::{Decoder, RawSegment, WordTiming};
use crate::SttError;

/// Plan 20 D6 (F1): the warmed whisper model as an OPAQUE, stt-owned handle.
/// Wraps the loaded `Arc<WhisperContext>` and deliberately exposes nothing —
/// callers (the ffi crate) can only hold it and hand it back to
/// `SttStream::with_warm`; no `whisper_rs` type ever crosses the stt
/// boundary. Load once via `load_warm_model` (the expensive model read +
/// Metal init), then every `with_warm` is an `Arc::clone` — no reload.
pub struct WarmModel {
    pub(crate) ctx: Arc<WhisperContext>,
}

/// Compile-assert `WarmModel: Send + Sync` (Plan 20 Stage 3 / R2): the warm
/// handle is held in the engine and consumed from walk/pump contexts.
/// Verified true in vendored whisper-rs 0.16.0 (`WhisperInnerContext` is
/// `unsafe impl Send`/`Sync`; the model is read-only after load, decode
/// states are per-call).
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<WarmModel>();
};

/// Load the whisper model once, off the tap path (Plan 20 D6/D7): the same
/// `WhisperContext::new_with_params` open `SttStream::with_model` performs,
/// captured into the reusable opaque handle.
pub fn load_warm_model(model: &Path, use_gpu: bool) -> Result<WarmModel, SttError> {
    let mut params = WhisperContextParameters::default();
    params.use_gpu(use_gpu);
    let ctx = WhisperContext::new_with_params(
        model.to_str().ok_or_else(|| SttError::ModelLoad("non-utf8 model path".into()))?,
        params,
    )
    .map_err(|e| SttError::ModelLoad(e.to_string()))?;
    Ok(WarmModel { ctx: Arc::new(ctx) })
}

/// whisper.cpp backend (Metal). Holds a loaded model context (shared via
/// `Arc` so a warmed context is reusable across sessions, Plan 20 D6); each
/// `decode` creates a fresh state (whisper-rs pattern). The crate NEVER
/// downloads the model — `open` reads a file the shell has already
/// provisioned.
pub struct WhisperDecoder {
    ctx: Arc<WhisperContext>,
    language: String,
    /// When on, `decode` sets whisper `token_timestamps` and reconstructs
    /// per-word timing onto `RawSegment.words` (Plan 09 D5). Flows from
    /// `SttConfig.word_timestamps` (default true, crate-internal).
    word_timestamps: bool,
}

impl WhisperDecoder {
    /// Wrap an already-loaded context (Plan 20 D6): the warm path. NO model
    /// load happens here — the context is `Arc::clone`d from a `WarmModel`.
    pub(crate) fn from_context(
        ctx: Arc<WhisperContext>,
        language: &str,
        word_timestamps: bool,
    ) -> Self {
        Self { ctx, language: language.to_string(), word_timestamps }
    }

    /// `use_gpu` is LOAD-BEARING (Plan 08 D7, falsified assumption): with the
    /// `metal` cargo feature the default is GPU-on, which HARD-CRASHES on the
    /// iOS simulator (SIGTRAP in ggml_metal_buffer_set_tensor via MTLSimDevice)
    /// rather than degrading to CPU. Sim builds must pass `false` (CPU/BLAS
    /// decode, proven working); device builds pass `true` (Metal). The value
    /// flows from `SttConfig.use_gpu` ← `EngineConfig.stt_use_gpu` ← Swift's
    /// `#if targetEnvironment(simulator)`.
    ///
    /// `word_timestamps` flows from `SttConfig.word_timestamps` (Plan 09 D5):
    /// when on, per-token `t0`/`t1` are grouped into `RawSegment.words`.
    pub fn open(model: &Path, language: &str, use_gpu: bool, word_timestamps: bool)
        -> Result<Self, SttError> {
        let mut params = WhisperContextParameters::default();
        params.use_gpu(use_gpu);
        let ctx = WhisperContext::new_with_params(
            model.to_str().ok_or_else(|| SttError::ModelLoad("non-utf8 model path".into()))?,
            params,
        )
        .map_err(|e| SttError::ModelLoad(e.to_string()))?;
        Ok(Self::from_context(Arc::new(ctx), language, word_timestamps))
    }
}

/// Group whisper tokens into words. Word boundaries are marked by a leading
/// space in the token's text (BPE); special/timestamp tokens (empty or
/// bracketed text) are skipped. Each word takes the FIRST sub-token's `t0` and
/// the LAST sub-token's `t1` (centiseconds, chunk-relative — same units as
/// `RawSegment.start_cs`/`end_cs`). The finalizer (D4) is the safety net: if
/// this reconstruction's count disagrees with the segment's whitespace split,
/// `words_from_segments` falls back to segment-coarse, so an imperfect grouping
/// degrades gracefully rather than corrupting output.
fn build_word_timings(seg: &WhisperSegment) -> Vec<WordTiming> {
    let mut words: Vec<WordTiming> = Vec::new();
    for i in 0..seg.n_tokens() {
        let Some(tok) = seg.get_token(i) else { continue };
        let Ok(raw) = tok.to_str_lossy() else { continue };
        // Skip special/timestamp markers (empty trimmed, or bracketed).
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('[') || trimmed.starts_with("<|") {
            continue;
        }
        let data = tok.token_data();
        // A leading space starts a new word (BPE); the first content token also
        // starts one. Otherwise the sub-token extends the current word.
        if raw.starts_with(' ') || words.is_empty() {
            words.push(WordTiming { text: trimmed.to_string(), start_cs: data.t0, end_cs: data.t1 });
        } else if let Some(last) = words.last_mut() {
            last.text.push_str(trimmed);
            last.end_cs = data.t1;
        }
    }
    words
}

impl Decoder for WhisperDecoder {
    fn decode(&mut self, samples: &[f32], initial_prompt: Option<&str>)
        -> Result<Vec<RawSegment>, SttError> {
        let mut state = self.ctx.create_state().map_err(|e| SttError::Decode(e.to_string()))?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(&self.language));
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_print_timestamps(false);
        params.set_translate(false);
        // Plan 09 D5: enable per-token timestamps so we can reconstruct per-word
        // timing below. Leaves segmentation/max_len/split_on_word untouched (D2),
        // so the no_speech_prob basis (Plan 08) is byte-for-byte unchanged.
        params.set_token_timestamps(self.word_timestamps);
        if let Some(p) = initial_prompt {
            params.set_initial_prompt(p);
        }
        state.full(params, samples).map_err(|e| SttError::Decode(e.to_string()))?;
        let n = state.full_n_segments();
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            if let Some(seg) = state.get_segment(i) {
                let text = seg.to_str_lossy().map(|c| c.into_owned()).unwrap_or_default();
                // Reconstruct per-word timing from token t0/t1 when enabled; the
                // finalizer self-heals to coarse if the count disagrees (D4).
                let words = if self.word_timestamps {
                    build_word_timings(&seg)
                } else {
                    Vec::new()
                };
                out.push(RawSegment {
                    start_cs: seg.start_timestamp(),
                    end_cs: seg.end_timestamp(),
                    text: text.trim().to_string(),
                    // Task 11b (R3): whisper's per-segment no-speech probability
                    // feeds the Finalizer's drop gate. Only compiled with the
                    // `whisper` feature, so it never affects the hermetic build.
                    no_speech_prob: seg.no_speech_probability(),
                    words,
                });
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs ONLY when the `whisper` feature is on AND MURMUR_WHISPER_MODEL points
    /// at a real ggml model file. #[ignore] keeps it out of `cargo test`; CI never
    /// has the model, so CI never runs it. Manual: reads the model, decodes 1 s of
    /// silence, asserts the pipeline returns without error.
    #[test]
    #[ignore = "needs a real model file via MURMUR_WHISPER_MODEL"]
    fn real_model_decodes_silence() {
        let model = std::env::var("MURMUR_WHISPER_MODEL")
            .expect("set MURMUR_WHISPER_MODEL to a ggml-*.bin path");
        let mut d = WhisperDecoder::open(std::path::Path::new(&model), "en", true, true).unwrap();
        let silence = vec![0.0f32; 16_000];
        let segs = d.decode(&silence, Some("Terms used in this session: french drain.")).unwrap();
        // silence may yield zero or a blank segment — the contract is "no error".
        let _ = segs;
    }

    /// Word-timestamp population on real audio (Plan 09 Task 5). Gated on
    /// MURMUR_WHISPER_MODEL; if MURMUR_WHISPER_SPEECH_WAV points at a 16 kHz mono
    /// f32-decodable WAV of known content it also spot-checks word placement.
    /// `#[ignore]` keeps it out of CI (no model, no fixture committed).
    #[test]
    #[ignore = "needs a real model file via MURMUR_WHISPER_MODEL (+ optional MURMUR_WHISPER_SPEECH_WAV)"]
    fn real_model_populates_word_timings() {
        let model = std::env::var("MURMUR_WHISPER_MODEL")
            .expect("set MURMUR_WHISPER_MODEL to a ggml-*.bin path");
        let mut d = WhisperDecoder::open(std::path::Path::new(&model), "en", true, true).unwrap();

        let Ok(wav_path) = std::env::var("MURMUR_WHISPER_SPEECH_WAV") else {
            // No speech WAV: fall back to the contract check — token_timestamps
            // on must not crash the decode; word population is then validated by
            // the Task 7 sweep on real audio.
            let silence = vec![0.0f32; 16_000];
            let _ = d.decode(&silence, None).unwrap();
            return;
        };

        // Minimal 16-bit PCM WAV reader (no dep): assumes 16 kHz mono s16le.
        let bytes = std::fs::read(&wav_path).expect("read speech WAV");
        let samples: Vec<f32> = bytes[44..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect();
        let segs = d.decode(&samples, None).unwrap();

        // count + monotonicity: at least one segment aligns word-for-word and its
        // per-word spans are non-decreasing.
        let aligned = segs.iter().find(|s| {
            !s.words.is_empty() && s.words.len() == s.text.split_whitespace().count()
        });
        let seg = aligned.expect("at least one segment with word-aligned timing");
        let mut prev = i64::MIN;
        for w in &seg.words {
            assert!(w.start_cs >= prev, "word start_cs non-decreasing");
            assert!(w.end_cs >= w.start_cs, "word end_cs >= start_cs");
            prev = w.start_cs;
        }

        // Per-word ground-truth spot-check (finding 2): if the fixture contains
        // known words, they must land within a tolerance window of their expected
        // centisecond position — catches gross BPE mis-grouping the count guard
        // alone (D3) would pass. Expected positions are optional env pairs like
        // MURMUR_WHISPER_EXPECT="french:120,drain:200" (word:center_cs), ±50 cs.
        if let Ok(expect) = std::env::var("MURMUR_WHISPER_EXPECT") {
            let all_words: Vec<&WordTiming> = segs.iter().flat_map(|s| s.words.iter()).collect();
            for pair in expect.split(',') {
                let (word, center) = pair.split_once(':').expect("word:center_cs");
                let center: i64 = center.parse().expect("center_cs is an int");
                let hit = all_words.iter().find(|w| w.text.eq_ignore_ascii_case(word))
                    .unwrap_or_else(|| panic!("expected word {word:?} not found in timings"));
                let mid = (hit.start_cs + hit.end_cs) / 2;
                assert!((mid - center).abs() <= 50,
                    "word {word:?} landed at {mid} cs, expected within ±50 of {center}");
            }
        }
    }
}
