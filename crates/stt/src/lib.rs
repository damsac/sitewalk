//! On-device streaming STT over whisper.cpp (spec Rev 2 §2). PCM in → append-only
//! finalized transcript segments out, biased by the user's ≤100-term vocabulary.
//! The whisper backend is behind the `whisper` feature; the pure chunk/finalize/
//! bias logic compiles and tests everywhere with no native toolchain or model file.

mod bias;
mod chunk;
mod decoder;
mod finalize;
mod vad;
#[cfg(feature = "whisper")]
mod whisper;

pub use bias::build_bias_prompt;
pub use decoder::{Decoder, RawSegment, ScriptedDecoder, WordTiming};
#[cfg(feature = "whisper")]
pub use whisper::WhisperDecoder;
#[cfg(feature = "whisper")]
pub use whisper::{load_warm_model, WarmModel};

/// A finalized, never-to-be-revised transcript segment (append-only contract).
/// Timestamps are ABSOLUTE audio milliseconds from stream start. The shell
/// appends `text` to `Store::append_transcript` (Plan 05 cursor feeder).
#[derive(Clone, Debug, PartialEq)]
pub struct FinalizedSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct SttConfig {
    /// Decode window length (spike default 5 s).
    pub chunk_secs: f64,
    /// Overlap re-decoded each window for LocalAgreement (spike default 1 s).
    pub overlap_secs: f64,
    /// Sample rate the shell must feed (whisper wants 16 kHz mono f32).
    pub sample_rate: u32,
    /// Whisper language hint ("en" for the *.en models).
    pub language: String,
    /// Hard cap on vocabulary terms injected via initial_prompt (spec: ≤100).
    pub max_bias_terms: usize,
    /// Whether the whisper backend may use the GPU (Metal). Default `true`
    /// (device builds). MUST be `false` on the iOS SIMULATOR: Metal there
    /// hard-crashes (SIGTRAP in ggml_metal_buffer_set_tensor via MTLSimDevice)
    /// instead of degrading — Plan 08 D7's "degrades to CPU" assumption was
    /// FALSIFIED by sim verification; CPU/BLAS decode on sim is proven working.
    /// Ignored by non-whisper decoders (`ScriptedDecoder`).
    pub use_gpu: bool,
    /// Per-window RMS-energy pre-gate (Plan 08 Task 11a, R3). A ready window
    /// whose RMS amplitude is below this is NOT decoded (skip the Metal decode,
    /// emit nothing) — but its samples still entered the Chunker and advanced
    /// the window cursor, so the "absolute ms from stream start" timestamp
    /// contract does NOT drift (pre-Chunker sample dropping is the rejected
    /// option). Default `0.0` = decode everything (conservative: never elides
    /// speech); the Task 12 SNR eval sets a measured value. `f32` RMS of
    /// [-1,1] PCM.
    pub vad_rms_threshold: f32,
    /// no_speech_prob post-check (Plan 08 Task 11b, R3). Finalized segments
    /// whose `RawSegment.no_speech_prob` exceeds this are dropped before they
    /// reach the committed transcript — the guard against whisper fluently
    /// hallucinating text over machinery drone. Default `0.6` (whisper's own
    /// customary no-speech threshold); the Task 12 eval tunes it. Segments with
    /// the default `no_speech_prob = 0.0` (ScriptedDecoder, Plan-06 tests) are
    /// always kept.
    pub no_speech_prob_threshold: f32,
    /// Enable whisper `token_timestamps` → per-word timing on `RawSegment.words`
    /// (Plan 09 D5). Default `true`. Crate-internal: backend-agnostic (works on
    /// CPU/BLAS and Metal alike) with no sim hazard, so it is NOT surfaced to
    /// `EngineConfig`/Swift (unlike `use_gpu`). Reversible without a code change —
    /// if the SNR sweep (Task 7) shows an unacceptable WER/RTF delta, flip to
    /// `false`: the `Finalizer` already degrades to segment-coarse, so nothing
    /// else changes. Ignored by non-whisper decoders (`ScriptedDecoder`).
    pub word_timestamps: bool,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            chunk_secs: 5.0,
            overlap_secs: 1.0,
            sample_rate: 16_000,
            language: "en".into(),
            max_bias_terms: 100,
            use_gpu: true,
            vad_rms_threshold: 0.0,
            no_speech_prob_threshold: 0.6,
            word_timestamps: true,
        }
    }
}

impl SttConfig {
    /// Reject configs the pipeline math can't honor. `overlap_secs >= chunk_secs`
    /// makes the finalize horizon (`chunk_len_ms − overlap_ms`, u64) underflow and
    /// leaves no forward progress per window, so it is a `Config` error. Called by
    /// `SttStream::with_model` (the production constructor); `with_decoder` also
    /// guards the horizon with `saturating_sub` for the test/FFI seam.
    pub fn validate(&self) -> Result<(), SttError> {
        if self.overlap_secs >= self.chunk_secs {
            return Err(SttError::Config(format!(
                "overlap_secs ({}) must be < chunk_secs ({})",
                self.overlap_secs, self.chunk_secs
            )));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SttError {
    #[error("model load failed: {0}")]
    ModelLoad(String),
    #[error("decode failed: {0}")]
    Decode(String),
    #[error("invalid config: {0}")]
    Config(String),
}

use std::sync::Mutex;

use chunk::Chunker;
use finalize::Finalizer;

struct Engine {
    decoder: Box<dyn Decoder>,
    chunker: Chunker,
    finalizer: Finalizer,
    #[cfg(test)]
    captured_prompts: Vec<Option<String>>,
}

pub struct SttStream {
    cfg: SttConfig,
    bias_prompt: Option<String>,
    input: Mutex<Vec<f32>>,      // pending PCM handed off from the audio thread
    engine: Mutex<Engine>,
}

impl SttStream {
    pub fn with_decoder(decoder: Box<dyn Decoder>, cfg: SttConfig, vocab: &[String]) -> Self {
        let bias_prompt = bias::build_bias_prompt(vocab, cfg.max_bias_terms);
        let chunker = Chunker::new(cfg.sample_rate, cfg.chunk_secs, cfg.overlap_secs);
        SttStream {
            input: Mutex::new(Vec::new()),
            engine: Mutex::new(Engine {
                decoder,
                chunker,
                finalizer: Finalizer::with_no_speech_threshold(cfg.no_speech_prob_threshold),
                #[cfg(test)]
                captured_prompts: Vec::new(),
            }),
            bias_prompt,
            cfg,
        }
    }

    #[cfg(feature = "whisper")]
    pub fn with_model(model: &std::path::Path, cfg: SttConfig, vocab: &[String])
        -> Result<Self, SttError> {
        cfg.validate()?; // reject overlap ≥ chunk before opening the model
        let decoder = whisper::WhisperDecoder::open(model, &cfg.language, cfg.use_gpu, cfg.word_timestamps)?;
        Ok(Self::with_decoder(Box::new(decoder), cfg, vocab))
    }

    /// Plan 20 D6: build a stream over an already-warmed model — NO model
    /// load. Internally `Arc::clone`s the `WarmModel`'s context into a
    /// `WhisperDecoder`; the handle stays reusable for the next walk.
    /// (`use_gpu` was fixed when the context was loaded; `cfg.use_gpu` is
    /// ignored here.) `cfg` window math is guarded the same way
    /// `with_decoder` guards it (saturating horizon).
    #[cfg(feature = "whisper")]
    pub fn with_warm(model: &WarmModel, cfg: SttConfig, vocab: &[String]) -> Self {
        let decoder = whisper::WhisperDecoder::from_context(
            model.ctx.clone(),
            &cfg.language,
            cfg.word_timestamps,
        );
        Self::with_decoder(Box::new(decoder), cfg, vocab)
    }

    /// Buffer PCM. Cheap: a short lock, no decode. Call OFF the real-time audio
    /// thread (hand buffers over from the AVAudioEngine tap — research Q6).
    pub fn push_pcm(&self, pcm: &[f32]) {
        self.input.lock().unwrap().extend_from_slice(pcm);
    }

    /// Drain buffered PCM into the chunker and decode every window now ready,
    /// returning all segments finalized this call (append-only). Runs the long
    /// Metal decode on the CALLER's thread — the shell calls this from a
    /// background thread on its own cadence (Plan 05 Deferred 3).
    pub fn poll(&self) -> Result<Vec<FinalizedSegment>, SttError> {
        let mut eng = self.engine.lock().unwrap();      // engine first...
        {
            let mut input = self.input.lock().unwrap(); // ...then input, briefly
            eng.chunker.push(&input);
            input.clear();
        }                                                // input released before decode
        let mut out = Vec::new();
        while let Some(w) = eng.chunker.take_ready_window() {
            self.decode_window(&mut eng, w, &mut out)?;
        }
        Ok(out)
    }

    /// Volatile preview tail for greyed UI. Never persisted, never append-only.
    pub fn preview_tail(&self) -> String {
        self.engine.lock().unwrap().finalizer.preview()
    }

    /// DONE (supersedes cancel-for-speed canon): flush the remaining buffered
    /// audio as a final window and finalize everything pending. Idempotent.
    pub fn end(&self) -> Result<Vec<FinalizedSegment>, SttError> {
        let mut eng = self.engine.lock().unwrap();
        {
            let mut input = self.input.lock().unwrap();
            eng.chunker.push(&input);
            input.clear();
        }
        let mut out = Vec::new();
        while let Some(w) = eng.chunker.take_ready_window() {
            self.decode_window(&mut eng, w, &mut out)?;
        }
        if let Some(w) = eng.chunker.flush() {
            // is_final window → decode_window uses an ∞ horizon → finalizes all.
            self.decode_window(&mut eng, w, &mut out)?;
        } else {
            // Nothing left to decode, but the last normal window may have held a
            // tail behind its horizon — flush it.
            emit(&mut out, eng.finalizer.flush());
        }
        Ok(out)
    }

    fn decode_window(&self, eng: &mut Engine, w: chunk::Window, out: &mut Vec<FinalizedSegment>)
        -> Result<(), SttError> {
        // Per-window energy pre-gate (Task 11a, R3): skip the DECODE for a
        // below-threshold (silence/quiet) NON-FINAL window — emit nothing, don't
        // call the decoder. The window's samples already entered the Chunker and
        // advanced `next_start` (in take_ready_window), so the absolute-ms
        // timestamp clock does NOT drift: the next window's start_sample is
        // unaffected. The final flush window is never gated (it may carry the
        // operator's last words; a truly-silent flush decodes to nothing anyway).
        if !w.is_final && !vad::is_speech(&w.samples, self.cfg.vad_rms_threshold) {
            return Ok(());
        }
        let window_start_ms = self.sample_to_ms(w.start_sample);
        let horizon_ms = if w.is_final {
            u64::MAX
        } else {
            // saturating_sub guards the test/FFI seam (with_decoder skips validate);
            // with_model rejects overlap ≥ chunk up front so this can't underflow there.
            window_start_ms + self.chunk_len_ms().saturating_sub(self.overlap_ms())
        };
        let raw = eng.decode_with_prompt(&w.samples, self.bias_prompt.as_deref())?;
        emit(out, eng.finalizer.ingest(window_start_ms, &raw, horizon_ms));
        Ok(())
    }

    fn sample_to_ms(&self, sample: u64) -> u64 {
        sample * 1000 / self.cfg.sample_rate as u64
    }
    fn chunk_len_ms(&self) -> u64 {
        (self.cfg.chunk_secs * 1000.0) as u64
    }
    fn overlap_ms(&self) -> u64 {
        (self.cfg.overlap_secs * 1000.0) as u64
    }

    #[cfg(test)]
    fn debug_captured_prompts(&self) -> Vec<Option<String>> {
        self.engine.lock().unwrap().captured_prompts.clone()
    }
}

impl Engine {
    fn decode_with_prompt(&mut self, samples: &[f32], prompt: Option<&str>)
        -> Result<Vec<RawSegment>, SttError> {
        #[cfg(test)]
        self.captured_prompts.push(prompt.map(str::to_string));
        self.decoder.decode(samples, prompt)
    }
}

/// Map finalized `Word`s to `FinalizedSegment`s, preserving each word's
/// (segment-coarse) absolute span.
fn emit(out: &mut Vec<FinalizedSegment>, words: Vec<finalize::Word>) {
    out.extend(words.into_iter().map(|w| FinalizedSegment {
        start_ms: w.start_ms,
        end_ms: w.end_ms,
        text: w.text,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(cs0: i64, cs1: i64, t: &str) -> RawSegment {
        RawSegment { start_cs: cs0, end_cs: cs1, text: t.into(), no_speech_prob: 0.0, words: vec![] }
    }
    fn text(v: &[FinalizedSegment]) -> Vec<&str> {
        v.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn bias_prompt_is_passed_to_every_decode() {
        // 9 s of PCM → two 5 s/1 s windows, both drained in one poll() call.
        let decoder = ScriptedDecoder::new(vec![
            vec![seg(0, 300, "the french drain")],
            vec![seg(0, 80, "drain"), seg(80, 300, "is backing")],
        ]);
        let stream = SttStream::with_decoder(
            Box::new(decoder),
            SttConfig::default(),
            &["french drain".to_string()],
        );
        stream.push_pcm(&vec![0.0; 144_000]);
        stream.poll().unwrap();
        // The scripted decoder recorded the prompt each decode saw.
        let prompts = stream.debug_captured_prompts();
        assert_eq!(prompts.len(), 2, "both ready windows decoded");
        assert!(prompts.iter().all(|p| p.as_deref() == Some("Terms used in this session: french drain.")));
    }

    #[test]
    fn poll_finalizes_incrementally_and_end_flushes_bounded_tail() {
        // REALISTIC time-shifted composition (NOT superstrings): window k+1's
        // segments start at chunk-relative cs=0, four seconds later in absolute
        // time; only the 1 s overlap words repeat.
        let decoder = ScriptedDecoder::new(vec![
            // window 0 [0,5s]: "for the" straddles the 4 s horizon → held
            vec![seg(0, 180, "order twelve"), seg(180, 360, "two by tens"), seg(360, 480, "for the")],
            // window 1 [4,9s]: head re-says the "for the" overlap, "today" straddles 8 s
            vec![seg(0, 80, "for the"), seg(80, 300, "deck framing"), seg(300, 480, "today")],
            // flush window [8,~9s]: re-says the "today" overlap
            vec![seg(0, 80, "today")],
        ]);
        let stream = SttStream::with_decoder(Box::new(decoder), SttConfig::default(), &[]);
        stream.push_pcm(&vec![0.0; 144_000]); // 9 s → W0 + W1 both ready
        let live = stream.poll().unwrap();     // one poll drains BOTH ready windows
        assert_eq!(text(&live),
            vec!["order", "twelve", "two", "by", "tens", "for", "the", "deck", "framing"]);
        assert_eq!(stream.preview_tail(), "today", "the straddling tail is held, bounded");
        let tail = stream.end().unwrap();      // flush finalizes only the held tail
        assert_eq!(text(&tail), vec!["today"]);
        // append-only in time: start_ms non-decreasing across the whole stream.
        let mut prev = 0;
        for s in live.iter().chain(tail.iter()) {
            assert!(s.start_ms >= prev);
            prev = s.start_ms;
        }
    }

    #[test]
    fn poll_is_a_noop_until_a_window_is_ready() {
        let stream = SttStream::with_decoder(
            Box::new(ScriptedDecoder::new(vec![])), SttConfig::default(), &[]);
        stream.push_pcm(&vec![0.0; 1000]); // far short of a window
        assert!(stream.poll().unwrap().is_empty(), "no decode, no scripted panic");
    }

    #[test]
    fn low_energy_window_is_not_decoded_but_cursor_still_advances() {
        // ScriptedDecoder has ONE script; it must be consumed by the SPEECH
        // window, NOT the silence window (energy-gated before the decoder runs).
        let decoder = ScriptedDecoder::new(vec![vec![seg(0, 100, "hello")]]);
        let cfg = SttConfig { vad_rms_threshold: 0.05, ..SttConfig::default() };
        let stream = SttStream::with_decoder(Box::new(decoder), cfg, &[]);

        // Window 0: one full window of silence → RMS 0 < 0.05 → NOT decoded.
        stream.push_pcm(&vec![0.0; 80_000]);
        let out0 = stream.poll().unwrap();
        assert!(out0.is_empty(), "silence window emits nothing (decode skipped)");

        // Window 1: speech-level energy → decoded. Its start_sample is 64_000
        // (advanced one step) — proving the skipped window still advanced the
        // Chunker cursor, so the absolute-ms timestamp contract holds.
        stream.push_pcm(&vec![0.3; 64_000]);
        let out1 = stream.poll().unwrap();
        assert_eq!(text(&out1), vec!["hello"], "speech window decodes the one script");
        assert!(out1[0].start_ms >= 4_000, "start_ms reflects the advanced cursor (64k samples = 4s)");
    }

    #[test]
    fn high_no_speech_prob_segment_is_not_finalized() {
        // One window, two segments: a machinery-drone hallucination (0.95) and
        // real speech (0.05). Only the speech reaches the committed stream (R3).
        let decoder = ScriptedDecoder::new(vec![vec![
            RawSegment { start_cs: 0, end_cs: 100, text: "machinery drone".into(), no_speech_prob: 0.95, words: vec![] },
            RawSegment { start_cs: 100, end_cs: 200, text: "order lumber".into(), no_speech_prob: 0.05, words: vec![] },
        ]]);
        let cfg = SttConfig { no_speech_prob_threshold: 0.6, ..SttConfig::default() };
        let stream = SttStream::with_decoder(Box::new(decoder), cfg, &[]);
        stream.push_pcm(&vec![0.3; 80_000]); // one full window (RMS gate is a no-op at default 0.0)
        let live = stream.poll().unwrap();
        let words = text(&live);
        assert!(
            !words.contains(&"machinery") && !words.contains(&"drone"),
            "high no_speech_prob segment dropped before commit (R3)"
        );
        assert_eq!(words, vec!["order", "lumber"], "the real-speech segment is kept");
    }

    #[test]
    fn gpu_defaults_on_and_is_overridable() {
        // Device default: GPU on. The sim path overrides it via struct-update
        // (exactly what the FFI layer does from EngineConfig.stt_use_gpu).
        assert!(SttConfig::default().use_gpu);
        let sim = SttConfig { use_gpu: false, ..SttConfig::default() };
        assert!(!sim.use_gpu);
        assert!(sim.validate().is_ok(), "the knob is orthogonal to config validity");
    }

    #[test]
    fn word_timestamps_defaults_on_and_is_overridable() {
        assert!(SttConfig::default().word_timestamps);
        let off = SttConfig { word_timestamps: false, ..SttConfig::default() };
        assert!(!off.word_timestamps);
        assert!(off.validate().is_ok(), "orthogonal to config validity");
    }

    #[test]
    fn config_rejects_overlap_ge_chunk() {
        assert!(SttConfig::default().validate().is_ok());
        let bad = SttConfig { chunk_secs: 5.0, overlap_secs: 5.0, ..SttConfig::default() };
        assert!(matches!(bad.validate(), Err(SttError::Config(_))), "overlap == chunk rejected");
        let worse = SttConfig { chunk_secs: 5.0, overlap_secs: 6.0, ..SttConfig::default() };
        assert!(matches!(worse.validate(), Err(SttError::Config(_))), "overlap > chunk rejected");
    }
}
