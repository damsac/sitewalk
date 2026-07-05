//! Cheap energy-based voice-activity pre-gate (Plan 08 Task 11a, product rule
//! R3: under-extract, never invent). Pure, allocation-free, no native deps — so
//! it compiles and tests everywhere with the `whisper` feature off.
//!
//! The gate's job is to keep whisper from being handed near-silent windows on
//! which it fluently hallucinates text. It runs on a window's samples AFTER the
//! Chunker has already accepted them (so the sample-count-derived timestamp
//! clock is untouched); a below-threshold window simply skips the decode.

/// Root-mean-square amplitude of a PCM window. Samples are expected in [-1, 1]
/// (whisper's f32 range); the result is in the same scale. Empty → `0.0`.
///
/// f64 accumulation keeps the sum-of-squares stable across a full 5 s window
/// (80k samples) without the precision loss an f32 running sum would suffer.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    (sum_sq / samples.len() as f64).sqrt() as f32
}

/// True if the window carries enough energy to be worth decoding. A `threshold`
/// of `0.0` accepts everything (the conservative default — never elides speech).
pub fn is_speech(samples: &[f32], threshold: f32) -> bool {
    rms(samples) >= threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_of_silence_is_zero() {
        assert_eq!(rms(&[0.0; 1000]), 0.0);
        assert_eq!(rms(&[]), 0.0);
    }

    #[test]
    fn rms_of_constant_amplitude_equals_that_amplitude() {
        // RMS of a DC signal at ±0.3 is 0.3.
        let r = rms(&[0.3; 4096]);
        assert!((r - 0.3).abs() < 1e-5, "rms={r}");
    }

    #[test]
    fn is_speech_gates_on_the_threshold() {
        assert!(!is_speech(&[0.0; 512], 0.05), "silence gated out");
        assert!(is_speech(&[0.3; 512], 0.05), "speech-level energy passes");
        assert!(is_speech(&[0.0; 512], 0.0), "threshold 0 accepts everything");
    }
}
