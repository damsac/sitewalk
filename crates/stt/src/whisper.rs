use std::path::Path;

use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
};

use crate::decoder::{Decoder, RawSegment};
use crate::SttError;

/// whisper.cpp backend (Metal). Owns a loaded model context; each `decode`
/// creates a fresh state (whisper-rs pattern). The crate NEVER downloads the
/// model — `open` reads a file the shell has already provisioned.
pub struct WhisperDecoder {
    ctx: WhisperContext,
    language: String,
}

impl WhisperDecoder {
    pub fn open(model: &Path, language: &str) -> Result<Self, SttError> {
        let mut params = WhisperContextParameters::default();
        // Redundant with the `metal` cargo feature (default is already GPU-on when
        // built with metal — the spike used bare defaults and Metal engaged); kept
        // as an explicit, harmless assertion of intent, not a load-bearing call.
        params.use_gpu(true);
        let ctx = WhisperContext::new_with_params(
            model.to_str().ok_or_else(|| SttError::ModelLoad("non-utf8 model path".into()))?,
            params,
        )
        .map_err(|e| SttError::ModelLoad(e.to_string()))?;
        Ok(Self { ctx, language: language.to_string() })
    }
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
        if let Some(p) = initial_prompt {
            params.set_initial_prompt(p);
        }
        state.full(params, samples).map_err(|e| SttError::Decode(e.to_string()))?;
        let n = state.full_n_segments();
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            if let Some(seg) = state.get_segment(i) {
                let text = seg.to_str_lossy().map(|c| c.into_owned()).unwrap_or_default();
                out.push(RawSegment {
                    start_cs: seg.start_timestamp(),
                    end_cs: seg.end_timestamp(),
                    text: text.trim().to_string(),
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
        let mut d = WhisperDecoder::open(std::path::Path::new(&model), "en").unwrap();
        let silence = vec![0.0f32; 16_000];
        let segs = d.decode(&silence, Some("Terms used in this session: french drain.")).unwrap();
        // silence may yield zero or a blank segment — the contract is "no error".
        let _ = segs;
    }
}
