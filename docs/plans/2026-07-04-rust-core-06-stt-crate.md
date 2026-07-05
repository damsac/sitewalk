# Murmur Rust Core — Plan 06: The STT Crate (`crates/stt`)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `crates/stt` — a real, on-device speech-to-text crate wrapping whisper.cpp (via `whisper-rs`) behind a clean, testable seam. It turns a stream of 16 kHz mono f32 PCM buffers (captured by the platform shell — Rust never touches the mic) into an **append-only stream of finalized transcript segments** plus a **volatile preview tail** for UI, biased by the user's ≤100-term vocabulary. It is the STT half of spec Rev 2 §2 (live in-session extraction, offline, on-device) and the direct feeder for Plan 05's `LiveExtractor` (finalized segments → `Store::append_transcript` → the append-only char cursor).

This plan **productionizes the spike** (`spikes/stt-whisper/`, GO verdict in `RESULTS.md`). The spike's measured numbers are this plan's design constants; the spike's `stream.rs` finalize logic is the reference for the quality lever (LocalAgreement word-level finalize). We steal ideas, not code — the spike is quarantined (`workspace.exclude = ["spikes"]`); `crates/stt` is a first-class **workspace member**.

**Spike verdict constants (measured on M4 Max, Metal — locked):**
- Engine: `whisper-rs =0.16.0` (exact pin), `metal` feature → `whisper-rs-sys 0.15.0` → vendored whisper.cpp (all MIT).
- Target models: `base.en` q5_1 (RTF 0.009, WER 5.8% clean / 11.7% noisy) and `small.en` q5_1 (RTF 0.021, WER 4.7% / 11.7%). Both clear RTF<0.5 and WER bars from a single model row.
- Chunking: **5 s chunk / 1 s overlap** default (configurable). Naive segment-level time-horizon finalize is lossy (80% streaming WER); **word-level LocalAgreement dedup finalize → 19% WER at ≤3 s finalize latency**. The finalize rule is the quality lever — this plan productionizes it.
- Append-only invariant: a finalized word is never revised. Proven + unit-tested in the spike; the production crate carries equivalent tests.
- Biasing: `initial_prompt` injection of the vocabulary → **+10 to +19 pp** term recall, **zero** hallucination flagged. v1 biasing = initial_prompt injection; trie/logit-bias deferred.
- Live WER (~19%) is ~4× batch WER (~5%). **Implication carried into the design:** the finalized live stream is a *provisional preview*; end-of-session `process()` on the full transcript (Plan 04) remains authoritative. `crates/stt` is not the truth; it feeds the live board.

**The four hard design decisions (justified up front — reviewers read these first):**

1. **The `Decoder` trait is the one seam that touches whisper.** Everything above it — PCM accumulation, chunk cutting, overlap, LocalAgreement finalize, bias-prompt assembly — is **pure Rust with no whisper dependency**, unit-tested against a `ScriptedDecoder` fake that returns canned segments. `whisper-rs` is an **optional dependency behind a `whisper` cargo feature**, off by default. This is what makes requirement 4 (hermetic CI) achievable: `cargo test --workspace` with default features compiles and passes on Linux CI with **no model files, no cmake/clang, no Metal** — because the only thing gated out is the ~40 lines of `WhisperDecoder`. The alternative (a macOS-only workspace) would break the existing Linux CI for `harness`/`murmur-core`/`evals`; rejected.

2. **Caller-driven pump, not an internal worker thread.** `SttStream` spawns **no threads, owns no channels, invokes no callbacks into the caller.** The shell captures audio and, off the real-time thread, calls `push_pcm()` (cheap: buffers samples) then `poll()` (runs the long Metal decode and returns newly-finalized segments). The shell owns cadence — exactly mirroring Plan 05's "the cadence is app-shell policy" (Plan 05 Deferred 3). This is the design that **cannot deadlock**: no worker thread to join, no channel to block on, no Rust→Swift callback re-entrancy. An internal worker would need a shutdown protocol, an mpsc for buffers, and a UniFFI callback interface to emit — three new deadlock/ordering surfaces for zero benefit, since the shell already runs a tick loop for `LiveExtractor`. Interior mutability (`Mutex`) with a strict two-lock order (engine → input, never the reverse) makes the object a `&self` `Arc` that UniFFI exposes directly (Plan 07).

3. **The crate never downloads, never touches the mic, never persists.** Construction takes a **model file path** (the shell's Application Support dir — model management, download, and on-demand-resources plumbing are shell concerns; the crate only opens a file that exists) and a `SttConfig`. Input is pushed PCM; output is pulled segments. No I/O beyond reading the model file at construction.

4. **`crates/stt` and `murmur-core` stay decoupled — the live-extraction wiring is deferred to Plan 07.** `stt` does not depend on `murmur-core` and vice versa. The integration contract (finalized segment → `append_transcript` → `LiveExtractor` cursor) is **documented precisely here and shown as an example**, but the actual tick loop that couples STT.poll to LiveExtractor.maybe_extract is app-shell orchestration living across the FFI boundary. Plan 05's self-review already established this (constraint 4: "STT and live extraction compose without coupling"). Building the loop here would force a crate dependency both plans deliberately avoid. Justified in Task 6 and Deferred.

**Tech stack:** new crate `stt`. Sole non-workspace dep is `whisper-rs =0.16.0` (optional, `metal` feature) + `thiserror` (workspace). Pure-logic tests use only std + a hand-rolled fake. Real-model tests are `#[ignore]`d and env-gated (`MURMUR_WHISPER_MODEL`), mirroring the existing `anthropic_smoke` gate — they need the `whisper` feature **and** a model file, so CI never runs them.

**Spec:** Rev 2 §2 (live, on-device, offline-degradable), §vocabulary point 3 (vocabulary feeds STT contextual biasing — the ≤100-term list from memory), §6 (transcript persists continuously; <8 s budget context). Research: `docs/research/2026-07-04-on-device-stt-frontier.md` (Option B chosen: Rust-side whisper.cpp for biasing control + Android hedge). Spike: `spikes/stt-whisper/RESULTS.md` (GO) and `src/stream.rs` (finalize reference).

---

## File Structure

```
crates/stt/
  Cargo.toml            # NEW: workspace member; optional whisper-rs behind `whisper` feature
  src/
    lib.rs              # NEW: public API — SttStream, SttConfig, FinalizedSegment, SttError, re-exports
    decoder.rs          # NEW: Decoder trait + RawSegment (the whisper seam); ScriptedDecoder (test fake)
    chunk.rs            # NEW: Chunker — PCM accumulation + 5s/1s window cutting (pure)
    finalize.rs         # NEW: LocalAgreement word-level append-only finalizer (the quality lever, pure)
    bias.rs             # NEW: build_bias_prompt (≤100 terms → initial_prompt) — the biasing seam (pure)
    whisper.rs          # NEW (cfg feature="whisper"): WhisperDecoder — the only file that imports whisper-rs
  tests/
    stream_append_only.rs   # NEW: end-to-end append-only contract via ScriptedDecoder (no model, no feature)
Cargo.toml              # MODIFY (root): add "crates/stt" to workspace members
flake.nix               # MODIFY: dev shell gains cmake + clang + LIBCLANG_PATH (for `--features whisper` builds)
README.md               # MODIFY: plan-series line
```

Run cargo via the dev shell or `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo <cmd>` from the repo root. Default builds/tests need **no** native toolchain; `--features whisper` needs the updated dev shell (Task 1).

---

## API sketch (the surface every later plan consumes)

```rust
// Construction (whisper-backed, behind the feature):
let stream = SttStream::with_model(Path::new(&model_path), SttConfig::default(), &vocab_terms)?;
// Construction (any backend — the test/FFI seam):
let stream = SttStream::with_decoder(Box::new(decoder), SttConfig::default(), &vocab_terms);

// Per audio buffer, OFF the real-time thread:
stream.push_pcm(&pcm_f32);              // cheap: buffers 16kHz mono f32 samples
let finalized = stream.poll()?;         // runs decode(s) when a chunk is ready; append-only segments out
let preview   = stream.preview_tail();  // volatile, un-finalized hypothesis for greyed UI (never persisted)

// DONE (supersedes cancel-for-speed canon): flush, don't drop.
let tail = stream.end()?;               // decodes remaining buffered audio, finalizes everything pending
```

`SttStream` is `Send + Sync` (interior `Mutex`), so Plan 07 wraps it in `Arc` and UniFFI exposes the `&self` methods directly — no actor, no async, no callback interface.

---

### Task 1: Workspace member, feature flags, types, and the `Decoder` seam

**Files:** create `crates/stt/Cargo.toml`, `crates/stt/src/lib.rs`, `crates/stt/src/decoder.rs`; modify root `Cargo.toml`, `flake.nix`.

- [ ] **Step 1: Root workspace + crate manifest + dev shell**

Root `Cargo.toml` — add the member; **keep the existing `exclude` line verbatim** (the spike must stay quarantined):
```toml
members = ["crates/harness", "crates/murmur-core", "crates/evals", "crates/stt"]
exclude = ["spikes"]   # prefix match — NOT "spikes/*" (workspace.exclude is not glob; cargo #11405)
```

`crates/stt/Cargo.toml`:
```toml
[package]
name = "stt"
version = "0.1.0"
edition = "2021"

[features]
default = []
# Enabling `whisper` pulls the native stack (whisper-rs + vendored whisper.cpp, Metal).
# OFF by default so `cargo test --workspace` stays hermetic and cross-platform.
whisper = ["dep:whisper-rs"]

[dependencies]
thiserror = { workspace = true }
whisper-rs = { version = "=0.16.0", features = ["metal"], optional = true }
```

`flake.nix` — the dev shell must gain what the spike's `shell.nix` needed for `whisper-rs-sys` to compile whisper.cpp (`cmake`, `clang`, `LIBCLANG_PATH` for bindgen). These are only exercised on `--features whisper` builds, but the shell must provide them:
```nix
devShells.default = pkgs.mkShell {
  packages = with pkgs; [ cargo rustc clippy rustfmt rust-analyzer cmake clang ];
  # bindgen (whisper-rs-sys) needs libclang on its path for `--features whisper`:
  LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
};
```
> Note (from `RESULTS.md`): this machine is a channel-less flake system; bare `nix-shell` fails on `<nixpkgs>`. The project already uses `flake.nix` (not `shell.nix`), so `direnv`/`nix develop` resolves nixpkgs correctly — no `-I nixpkgs=...` workaround needed here.

- [ ] **Step 2: Write the failing tests** (`crates/stt/src/decoder.rs`, bottom)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_decoder_returns_scripts_in_order_and_captures_prompts() {
        let mut d = ScriptedDecoder::new(vec![
            vec![RawSegment { start_cs: 0, end_cs: 200, text: "hello world".into() }],
            vec![RawSegment { start_cs: 0, end_cs: 150, text: "again now".into() }],
        ]);
        let a = d.decode(&[0.0; 16], Some("french drain, ledger")).unwrap();
        assert_eq!(a[0].text, "hello world");
        let b = d.decode(&[0.0; 16], None).unwrap();
        assert_eq!(b[0].text, "again now");
        assert_eq!(d.captured_prompts(), &[Some("french drain, ledger".to_string()), None]);
    }

    #[test]
    fn scripted_decoder_errors_when_exhausted() {
        let mut d = ScriptedDecoder::new(vec![]);
        assert!(matches!(d.decode(&[0.0; 8], None), Err(SttError::Decode(_))));
    }
}
```

- [ ] **Step 3: Implement** (`crates/stt/src/decoder.rs`)

```rust
use crate::SttError;

/// One decoded segment as whisper.cpp emits it: timestamps are CHUNK-RELATIVE
/// centiseconds (offset to absolute audio time by the engine, not here).
#[derive(Clone, Debug, PartialEq)]
pub struct RawSegment {
    pub start_cs: i64,
    pub end_cs: i64,
    pub text: String,
}

/// The one seam that touches whisper. Everything above it (chunk cutting,
/// overlap, LocalAgreement finalize, bias prompt) is pure and testable against
/// a fake. `decode` runs ONE window of samples with an optional `initial_prompt`
/// (the biasing surface). Implementations may be slow (Metal); the caller runs
/// them off the real-time thread (see `SttStream::poll`).
pub trait Decoder: Send {
    fn decode(&mut self, samples: &[f32], initial_prompt: Option<&str>)
        -> Result<Vec<RawSegment>, SttError>;
}

/// Test/example fake: replays scripted segment lists and records the prompts it
/// was handed, so the pure engine can be exercised with zero whisper dependency.
pub struct ScriptedDecoder {
    scripts: std::collections::VecDeque<Vec<RawSegment>>,
    captured_prompts: Vec<Option<String>>,
}

impl ScriptedDecoder {
    pub fn new(scripts: Vec<Vec<RawSegment>>) -> Self {
        Self { scripts: scripts.into(), captured_prompts: Vec::new() }
    }
    pub fn captured_prompts(&self) -> &[Option<String>] {
        &self.captured_prompts
    }
}

impl Decoder for ScriptedDecoder {
    fn decode(&mut self, _samples: &[f32], initial_prompt: Option<&str>)
        -> Result<Vec<RawSegment>, SttError> {
        self.captured_prompts.push(initial_prompt.map(str::to_string));
        self.scripts
            .pop_front()
            .ok_or_else(|| SttError::Decode("scripted decoder exhausted".into()))
    }
}
```

`crates/stt/src/lib.rs` — the public shell + error + config (types only for now):
```rust
//! On-device streaming STT over whisper.cpp (spec Rev 2 §2). PCM in → append-only
//! finalized transcript segments out, biased by the user's ≤100-term vocabulary.
//! The whisper backend is behind the `whisper` feature; the pure chunk/finalize/
//! bias logic compiles and tests everywhere with no native toolchain or model file.

mod bias;
mod chunk;
mod decoder;
mod finalize;
#[cfg(feature = "whisper")]
mod whisper;

pub use decoder::{Decoder, RawSegment, ScriptedDecoder};
#[cfg(feature = "whisper")]
pub use whisper::WhisperDecoder;

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
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            chunk_secs: 5.0,
            overlap_secs: 1.0,
            sample_rate: 16_000,
            language: "en".into(),
            max_bias_terms: 100,
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
```

- [ ] **Step 4: Verify** `nix develop -c cargo test -p stt` (pure tests pass; whisper not compiled) and `cargo build -p stt --features whisper` (native stack compiles in the updated shell).

- [ ] **Step 5: Commit**
```bash
git add -A && git commit -m "feat(stt): scaffold crate — Decoder seam, types, feature-gated whisper, dev shell"
```

---

### Task 2: The Chunker — PCM accumulation and window cutting (pure)

**Files:** create `crates/stt/src/chunk.rs`.

The chunker owns the sample buffer and a `next_window_start` cursor in absolute samples. It cuts `chunk_secs` windows stepping by `chunk_secs - overlap_secs`, and only yields a window once enough samples have arrived to fill it (or on flush). It never decodes — it hands sample slices + the window's absolute start offset to the caller.

- [ ] **Step 1: Write the failing tests** (`crates/stt/src/chunk.rs`, bottom)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // 16 kHz: 5 s = 80_000 samples, 1 s overlap → step = 4 s = 64_000 samples.
    fn chunker() -> Chunker { Chunker::new(16_000, 5.0, 1.0) }

    #[test]
    fn yields_nothing_until_a_full_window_arrives() {
        let mut c = chunker();
        c.push(&vec![0.0; 79_999]);
        assert!(c.take_ready_window().is_none(), "one sample short of a window");
        c.push(&[0.0]);
        let w = c.take_ready_window().expect("full window now ready");
        assert_eq!(w.start_sample, 0);
        assert_eq!(w.samples.len(), 80_000);
    }

    #[test]
    fn steps_by_chunk_minus_overlap() {
        let mut c = chunker();
        c.push(&vec![0.0; 144_000]); // 9 s → windows [0,5s) and [4s,9s)
        let w0 = c.take_ready_window().unwrap();
        assert_eq!(w0.start_sample, 0);
        let w1 = c.take_ready_window().unwrap();
        assert_eq!(w1.start_sample, 64_000, "advanced by 4 s, re-decoding the 1 s overlap");
        assert!(c.take_ready_window().is_none());
    }

    #[test]
    fn flush_emits_the_short_final_window() {
        let mut c = chunker();
        c.push(&vec![0.0; 32_000]); // 2 s only — never fills a 5 s window
        assert!(c.take_ready_window().is_none());
        let w = c.flush().expect("flush yields the remaining tail");
        assert_eq!(w.start_sample, 0);
        assert_eq!(w.samples.len(), 32_000);
        assert!(w.is_final);
        assert!(c.flush().is_none(), "nothing left after flush");
    }

    #[test]
    fn drops_consumed_prefix_to_bound_memory() {
        let mut c = chunker();
        c.push(&vec![0.0; 144_000]);
        c.take_ready_window().unwrap(); // consumes through step=64_000
        c.take_ready_window().unwrap();
        // Buffer retains only from the last window start onward, not all 9 s.
        assert!(c.buffered_samples() <= 80_000, "old audio behind the cursor is freed");
    }
}
```

- [ ] **Step 2: Implement** (`crates/stt/src/chunk.rs`)

```rust
/// A window ready to decode. `start_sample` is absolute (from stream start) for
/// converting chunk-relative segment timestamps to absolute ms. `is_final` marks
/// the flush tail (finalizer uses ∞ horizon on it — nothing comes after).
pub struct Window {
    pub start_sample: u64,
    pub samples: Vec<f32>,
    pub is_final: bool,
}

/// Accumulates PCM and cuts fixed windows with overlap. Pure; no decode, no I/O.
/// Frees audio behind the window cursor to bound memory over an hour-long session.
pub struct Chunker {
    chunk_len: usize,   // samples per window
    step: usize,        // samples between window starts (chunk_len - overlap)
    buf: Vec<f32>,      // samples from `buf_start` onward
    buf_start: u64,     // absolute sample index of buf[0]
    next_start: u64,    // absolute sample index of the next window to emit
    done: bool,
}

impl Chunker {
    pub fn new(sample_rate: u32, chunk_secs: f64, overlap_secs: f64) -> Self {
        let sr = sample_rate as f64;
        let chunk_len = (chunk_secs * sr) as usize;
        let step = (((chunk_secs - overlap_secs).max(0.1)) * sr) as usize;
        Self { chunk_len, step, buf: Vec::new(), buf_start: 0, next_start: 0, done: false }
    }

    pub fn push(&mut self, pcm: &[f32]) {
        self.buf.extend_from_slice(pcm);
    }

    pub fn buffered_samples(&self) -> usize {
        self.buf.len()
    }

    /// Yields the next full window if enough audio has arrived, advancing the
    /// cursor by `step` and freeing audio behind the new cursor.
    pub fn take_ready_window(&mut self) -> Option<Window> {
        let rel_start = (self.next_start - self.buf_start) as usize;
        let rel_end = rel_start + self.chunk_len;
        if rel_end > self.buf.len() {
            return None;
        }
        let samples = self.buf[rel_start..rel_end].to_vec();
        let window = Window { start_sample: self.next_start, samples, is_final: false };
        self.next_start += self.step as u64;
        self.free_consumed();
        Some(window)
    }

    /// The short final window: everything from the cursor to the buffer end,
    /// marked `is_final`. Call once at end()/flush(); returns None if empty.
    pub fn flush(&mut self) -> Option<Window> {
        if self.done {
            return None;
        }
        self.done = true;
        let rel_start = (self.next_start - self.buf_start) as usize;
        if rel_start >= self.buf.len() {
            return None;
        }
        let samples = self.buf[rel_start..].to_vec();
        Some(Window { start_sample: self.next_start, samples, is_final: true })
    }

    fn free_consumed(&mut self) {
        // Retain from the next window's start (which sits `overlap` before
        // `next_start`)... simplest correct bound: keep from `next_start`.
        let keep_from = self.next_start.min(self.buf_start + self.buf.len() as u64);
        let drop_n = (keep_from - self.buf_start) as usize;
        if drop_n > 0 {
            self.buf.drain(..drop_n);
            self.buf_start = keep_from;
        }
    }
}
```
> `free_consumed` keeps memory O(one window), not O(whole session) — important for the hour-long locked-phone session (research Q5/Q8). It drops to `next_start`; the overlap re-decode reads samples that are still ahead of `next_start` in the next window, so nothing needed is freed.

- [ ] **Step 3: Verify** `cargo test -p stt chunk` — green.

- [ ] **Step 4: Commit**
```bash
git add -A && git commit -m "feat(stt): Chunker — bounded-memory PCM windowing at 5s/1s"
```

---

### Task 3: Time-anchored overlap-merge finalizer — the quality lever (pure)

**Files:** create `crates/stt/src/finalize.rs`.

This is the plan's single most important algorithm, and it is the **measured-good pathway** from `RESULTS.md` Table 2 — the *dedup-reassembly* result (5 s/1 s → **19% WER at ≤3 s latency**), **not** the naive segment-level time-horizon rule (80% at the same chunk size). We productionize the spike's `reassemble_dedup` (its longest suffix/prefix token merge, `spikes/stt-whisper/src/stream.rs`) into an *incremental* finalizer, and gate finalization on the spike's time horizon (`chunk_end − overlap`, `stream.rs::finalize`).

**Why the first design was wrong (the bug this task fixes):** the `Chunker` (Task 2) emits fixed-size, time-shifted windows decoded *standalone* and drops consumed audio for bounded memory. So window *k*'s hypothesis and window *k+1*'s hypothesis start at **different instants** (4 s apart, overlapping by 1 s) — `k+1` is **not** a superstring of `k`. A prefix-agreement rule (`common_prefix_len` at position 0) therefore matches nothing mid-session, finalizes nothing, and dumps the whole transcript at `end()`. The fix uses **both signals the pipeline already has**: absolute time (from `Window.start_sample`, plus each `RawSegment`'s `start_cs`/`end_cs`) to place words on a real timeline and decide the finalize horizon, and the spike's **text-overlap suffix/prefix merge** to stitch the ~1 s overlap region without duplicating it.

**Algorithm.** State is a single bounded buffer `pending: Vec<Word>` where `Word { text, start_ms, end_ms }` carries each word's absolute time (all words in one whisper segment share that segment's coarse span — v1 accepts segment-granular time; word-precise is Deferred 4). Per window (`window_start_ms`, its `RawSegment`s, and a `horizon_ms`):
1. **Words + time:** expand segments into `Word`s, `start_ms = window_start_ms + start_cs·10`, `end_ms = window_start_ms + end_cs·10` (centiseconds→ms).
2. **Merge (spike `reassemble_dedup` + time fallback):** find the largest *k* (≤ ~40) where `pending`'s last *k* word *texts* equal the new window's first *k* word texts; append only `new_words[k..]`. This dedups the re-decoded overlap when it re-transcribed identically (the **precise text seam**). When the overlap re-decoded *differently* the text match fails (`best == 0`); an all-or-nothing text merge would then append the *entire* new window and duplicate the overlap phrase. So a **time fallback** (coarse seam, first-decode-wins) kicks in: drop the prefix of `new_words` whose `end_ms` ≤ the max `end_ms` already in `pending` — those cover audio the finalizer already holds — keeping the first decode of the disputed overlap and appending only the genuinely-new suffix. The merge **only ever appends** to `pending` and never rewrites an existing entry.
3. **Finalize before the horizon (spike `finalize`):** drain from the front of `pending` every word whose `end_ms ≤ horizon_ms` and emit it. For a normal window `horizon_ms = window_start_ms + chunk_len − overlap` — which is exactly where the **next** window begins, so those words will never be re-decoded (safe to finalize). The final (flush) window uses `horizon_ms = u64::MAX`, committing everything.

**Bounded memory / bounded tail:** after each window, `pending` holds only words at/after the horizon — roughly the overlap region plus the current window's straddling tail, **O(one chunk)**, never the session. **Append-only:** finalized words leave `pending` and are never touched again; a later window's re-decode of already-finalized audio can't even occur (that audio is behind the window and was dropped by the Chunker), so a committed word is never revised. When the overlap re-decodes *differently* (e.g. `needs work`→`needs word`, or `drain`→`drane`), the merge's **time fallback keeps the first decode and drops the divergent re-decode**, so the disagreement costs at most a single first-decode-wins word (priced into the measured 19% WER) — it does **not** duplicate or revise emitted output. *(Without that fallback, the all-or-nothing text merge would append the whole re-decoded window on any partial disagreement, duplicating the overlap phrase into the committed stream — the failure mode Task 3's disagreement test now guards against.)*

**Prose walk on "the quick brown fox…" (5 s chunk / 1 s overlap → 4 s step):** windows are [0,5 s], [4,9 s], [8,13 s]. Say the speaker's words fall at: `the`@0.2 `quick`@1 `brown`@2 `fox`@3 `jumps`@4.2 `over`@5 `the`@6 `lazy`@7 `dog`@8.2 `near`@9 `the`@10 `old`@11 `red`@12.2 `barn`@13 (segment end-times, seconds).
- **W0** [0,5 s], horizon 4 s: pending = all of W0's words. `the quick brown fox` (ends ≤4 s) → **finalized**; `jumps`@4.2 held. `pending=[jumps]`.
- **W1** [4,9 s], horizon 8 s: new head re-decodes `jumps`@4.2 (the overlap) then `over the lazy dog`. Merge: pending tail `jumps` == new head `jumps` (k=1) → append `over the lazy dog`. Finalize ≤8 s → **`jumps over the lazy`** emitted; `dog`@8.2 held. `pending=[dog]`.
- **W2** [8,13 s], horizon 12 s: merge dedups `dog`, appends `near the old red barn`. Finalize ≤12 s → **`dog near the old`**; `red`@12.2 `barn`@13 held.
- **end()** flush (horizon ∞): merge the short final window (re-says `red barn`, deduped), finalize all → **`red barn`**.
Output, in order, append-only, tail never exceeding ~2 words: `the quick brown fox` · `jumps over the lazy` · `dog near the old` · `red barn`.
> **On the one-word-per-second framing:** the walk assigns each word its own time for readability, but word times are actually **segment-coarse** — every word expanded from one `RawSegment` shares that segment's absolute span (Deferred 4). So a multi-word segment finalizes **atomically**: all its words leave `pending` together the moment the segment's shared `end_ms` clears the horizon (never split mid-segment). The one-word-per-segment cadence above is illustrative only.

- [ ] **Step 1: Write the failing tests** (`crates/stt/src/finalize.rs`, bottom) — the REALISTIC time-shifted composition model: each window's segments start at chunk-relative `cs=0`, only the overlap words repeat, and one test injects an overlap disagreement.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::RawSegment;

    fn seg(cs0: i64, cs1: i64, t: &str) -> RawSegment {
        RawSegment { start_cs: cs0, end_cs: cs1, text: t.into() }
    }
    fn words(ws: &[Word]) -> Vec<&str> {
        ws.iter().map(|w| w.text.as_str()).collect()
    }

    #[test]
    fn finalizes_incrementally_across_time_shifted_windows() {
        let mut f = Finalizer::new();
        // window 0 [0,5s], horizon 4000: last segment straddles 4s → held.
        let e0 = f.ingest(0, &[seg(0, 180, "order twelve"), seg(180, 360, "two by tens"),
                               seg(360, 480, "for the")], 4_000);
        assert_eq!(words(&e0), vec!["order", "twelve", "two", "by", "tens"]);
        assert_eq!(f.preview(), "for the", "the straddling tail is held, not emitted");
        // window 1 [4s,9s], horizon 8000: head re-says the "for the" overlap.
        let e1 = f.ingest(4_000, &[seg(0, 80, "for the"), seg(80, 300, "deck framing"),
                                   seg(300, 480, "today now")], 8_000);
        assert_eq!(words(&e1), vec!["for", "the", "deck", "framing"]);
        // starvation guard: incremental progress, not one end-of-session dump.
        assert!(e0.len() + e1.len() >= 9, "words finalize as windows arrive");
    }

    #[test]
    fn overlap_word_is_finalized_exactly_once() {
        let mut f = Finalizer::new();
        let e0 = f.ingest(0, &[seg(0, 180, "hello there"), seg(360, 480, "friend")], 4_000);
        // "friend" ends 4800 > horizon 4000 → held for the overlap.
        let e1 = f.ingest(4_000, &[seg(0, 80, "friend"), seg(80, 300, "good day")], 8_000);
        let all: Vec<&str> = words(&e0).into_iter().chain(words(&e1)).collect();
        assert_eq!(all.iter().filter(|w| **w == "friend").count(), 1, "overlap emitted once");
    }

    #[test]
    fn append_only_holds_under_overlap_disagreement() {
        let mut f = Finalizer::new();
        let e0 = f.ingest(0, &[seg(0, 180, "the french drain"), seg(180, 480, "needs work")], 4_000);
        assert_eq!(words(&e0), vec!["the", "french", "drain"]); // ends ≤4000; "needs work" held
        // Window 1 re-decodes the overlap "needs work" DIFFERENTLY as "needs word":
        // the all-or-nothing text merge finds no match (best=0), so the TIME-ANCHORED
        // fallback drops the re-decoded overlap (end_ms ≤ pending's max end 4800) and
        // keeps W0's first decode, appending only the genuinely-new suffix.
        let e1 = f.ingest(4_000, &[seg(0, 80, "needs word"), seg(80, 400, "before the pour")], 8_000);

        let all: Vec<&str> = words(&e0).into_iter().chain(words(&e1)).collect();
        // Committed stream is exactly the first-decode reading with the overlap
        // present ONCE — no "needs work needs word" duplication (the bug this fixes).
        assert_eq!(all, vec!["the", "french", "drain", "needs", "work", "before", "the", "pour"]);
        // First decode of the disputed word wins; the divergent re-decode is gone.
        assert!(!all.contains(&"word"), "divergent second decode never reaches committed output");
        assert_eq!(all.iter().filter(|w| **w == "work").count(), 1, "disputed overlap not duplicated");
        // Genuinely-new content still finalizes.
        assert!(all.contains(&"before") && all.contains(&"pour"));
    }

    #[test]
    fn flush_emits_only_the_bounded_tail() {
        let mut f = Finalizer::new();
        let e0 = f.ingest(0, &[seg(0, 180, "alpha beta"), seg(360, 480, "gamma delta")], 4_000);
        assert_eq!(words(&e0), vec!["alpha", "beta"]);
        assert_eq!(f.preview(), "gamma delta", "tail bounded to the straddling segment");
        let tail = f.flush();
        assert_eq!(words(&tail), vec!["gamma", "delta"], "flush finalizes only the held tail");
        assert!(f.flush().is_empty(), "flush is idempotent");
    }
}
```

- [ ] **Step 2: Implement** (`crates/stt/src/finalize.rs`)

```rust
use crate::decoder::RawSegment;

/// A finalized-or-pending word with absolute time. All words expanded from one
/// whisper segment share that segment's coarse span (v1; word-precise deferred).
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Incremental, time-anchored overlap-merge finalizer — the productionized
/// `reassemble_dedup` + `finalize` from `spikes/stt-whisper/src/stream.rs`
/// (`RESULTS.md` Table 2: 19% WER at ≤3 s latency, vs 80% for naive segment
/// finalize). `pending` is bounded to ~one chunk; the emitted stream is
/// append-only (a finalized word is never revised).
#[derive(Default)]
pub struct Finalizer {
    pending: Vec<Word>,
    flushed: bool,
}

impl Finalizer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Merge one decoded window (`window_start_ms` + its segments) into `pending`
    /// via the spike's suffix/prefix text overlap, then finalize every word whose
    /// segment ends at/before `horizon_ms` (= next window's start for a normal
    /// window; `u64::MAX` for the flush window). Returns newly finalized words.
    pub fn ingest(&mut self, window_start_ms: u64, segs: &[RawSegment], horizon_ms: u64) -> Vec<Word> {
        let new_words = words_from_segments(window_start_ms, segs);
        self.merge(new_words);
        self.finalize_before(horizon_ms)
    }

    /// Final window with no successor: commit the entire remaining tail.
    pub fn flush(&mut self) -> Vec<Word> {
        if self.flushed {
            return Vec::new();
        }
        self.flushed = true;
        self.finalize_before(u64::MAX)
    }

    /// Volatile preview (un-finalized tail) for greyed UI. Never persisted.
    pub fn preview(&self) -> String {
        self.pending.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
    }

    /// Merge one decoded window's words into `pending`, deduping the re-decoded
    /// overlap. Only ever appends — existing `pending` words stand (append-only).
    /// TWO seams:
    ///   • **Precise (text) seam** — spike `reassemble_dedup`: the largest *k*
    ///     where `pending`'s last *k* word texts equal the new window's first *k*.
    ///     When the overlap re-decoded identically, this stitches it exactly.
    ///   • **Coarse (time) seam** — first-decode-wins fallback when the text match
    ///     fails (`best == 0`). An all-or-nothing text merge that finds no match
    ///     would append the ENTIRE new window, so a *partially disagreeing* overlap
    ///     (e.g. "needs work" re-decoded as "needs word") would duplicate the
    ///     overlap phrase into the finalized (committed) stream. Instead, use the
    ///     absolute timestamps we deliberately kept: drop the prefix of `new_words`
    ///     whose `end_ms` ≤ the max `end_ms` already in `pending` — those are
    ///     re-transcriptions of audio the finalizer already holds — keeping the
    ///     FIRST decode of the disputed overlap and appending only the genuinely-new
    ///     suffix. Cost of a disagreement is thus "first-decode-wins on the disputed
    ///     word" (a possible single-word error, priced into WER), NOT duplication.
    ///     Stays O(overlap).
    fn merge(&mut self, new_words: Vec<Word>) {
        if self.pending.is_empty() {
            self.pending = new_words;
            return;
        }
        let maxk = self.pending.len().min(new_words.len()).min(40);
        let mut best = 0;
        for k in (1..=maxk).rev() {
            let tail = &self.pending[self.pending.len() - k..];
            if tail.iter().map(|w| &w.text).eq(new_words[..k].iter().map(|w| &w.text)) {
                best = k;
                break;
            }
        }
        if best > 0 {
            self.pending.extend(new_words.into_iter().skip(best)); // precise seam
            return;
        }
        // Coarse seam: no text match → drop the time-covered prefix, keep first decode.
        let pending_max_end = self.pending.iter().map(|w| w.end_ms).max().unwrap_or(0);
        self.pending.extend(new_words.into_iter().skip_while(|w| w.end_ms <= pending_max_end));
    }

    /// Drain and return the front run of words whose segment ends ≤ horizon
    /// (spike `finalize`: `seg.end <= chunk_end − overlap`).
    fn finalize_before(&mut self, horizon_ms: u64) -> Vec<Word> {
        let cut = self.pending.iter().position(|w| w.end_ms > horizon_ms).unwrap_or(self.pending.len());
        self.pending.drain(..cut).collect()
    }
}

fn words_from_segments(window_start_ms: u64, segs: &[RawSegment]) -> Vec<Word> {
    let mut out = Vec::new();
    for s in segs {
        let start_ms = window_start_ms + (s.start_cs.max(0) as u64) * 10;
        let end_ms = window_start_ms + (s.end_cs.max(0) as u64) * 10;
        for tok in s.text.split_whitespace() {
            out.push(Word { text: tok.to_string(), start_ms, end_ms });
        }
    }
    out
}
```
> **Design note for reviewers:** this is the spike's *measured-good* dedup-reassembly, incrementalized — not the naive time-horizon rule (80% WER) and not a prefix-agreement rule (which assumes a shared start anchor the time-shifted Chunker never provides). Time places words and sets the finalize horizon; the text suffix/prefix merge stitches the overlap. `pending` stays O(one chunk). **Expected field-tuning surface:** the merge window (40), punctuation/casing normalization before the text compare, and horizon slack — all measured against the spike corpus, none change this shape.
>
> **WER-transfer caveat:** the spike's 19% figure (`RESULTS.md` Table 2, 5 s/1 s) was measured on `reassemble_dedup` running against the *full untruncated* chunk history and scored end-of-session. This bounded-pending incremental form searches only ~one chunk of `pending`, so it can find shorter matches near the finalized boundary. The number is *expected to transfer* at the 5 s/1 s defaults but was **not re-measured** for the incremental finalizer — re-run the spike's WER harness against this shape before treating 19% as a committed production number.

- [ ] **Step 3: Verify** `cargo test -p stt finalize` — green.

- [ ] **Step 4: Commit**
```bash
git add -A && git commit -m "feat(stt): time-anchored overlap-merge finalizer (productionized spike dedup)"
```

---

### Task 4: The bias seam + `SttStream` orchestration (pure end-to-end via the fake)

**Files:** create `crates/stt/src/bias.rs`; extend `crates/stt/src/lib.rs` with `SttStream`.

- [ ] **Step 1: Bias tests** (`crates/stt/src/bias.rs`, bottom)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_terms_yield_no_prompt() {
        assert_eq!(build_bias_prompt(&[], 100), None);
    }

    #[test]
    fn terms_are_joined_and_capped() {
        let terms: Vec<String> = (0..150).map(|i| format!("term{i}")).collect();
        let p = build_bias_prompt(&terms, 100).unwrap();
        assert!(p.contains("term0") && p.contains("term99"));
        assert!(!p.contains("term100"), "capped at max_bias_terms (spec ≤100)");
    }
}
```

- [ ] **Step 2: Implement bias** (`crates/stt/src/bias.rs`)

```rust
/// Assemble the whisper `initial_prompt` from the user's vocabulary terms
/// (memory `vocabulary` section, spec §vocabulary point 3). Spike `RESULTS.md`:
/// initial_prompt injection gave +10–19 pp term recall with zero hallucination.
/// This is the v1 biasing SEAM — a later plan swaps in trie/logit-bias by
/// replacing what `SttStream` does with these terms, not this signature.
pub fn build_bias_prompt(terms: &[String], max_terms: usize) -> Option<String> {
    let kept: Vec<&str> = terms.iter().take(max_terms).map(String::as_str).collect();
    if kept.is_empty() {
        return None;
    }
    // A glossary-style list; whisper reads the prompt as prior context, so a
    // natural comma list biases toward these spellings without a rigid schema.
    Some(format!("Terms used in this session: {}.", kept.join(", ")))
}
```

- [ ] **Step 3: SttStream tests** (`crates/stt/src/lib.rs`, bottom — the end-to-end pure test)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn seg(cs0: i64, cs1: i64, t: &str) -> RawSegment {
        RawSegment { start_cs: cs0, end_cs: cs1, text: t.into() }
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
    fn config_rejects_overlap_ge_chunk() {
        assert!(SttConfig::default().validate().is_ok());
        let bad = SttConfig { chunk_secs: 5.0, overlap_secs: 5.0, ..SttConfig::default() };
        assert!(matches!(bad.validate(), Err(SttError::Config(_))), "overlap == chunk rejected");
        let worse = SttConfig { chunk_secs: 5.0, overlap_secs: 6.0, ..SttConfig::default() };
        assert!(matches!(worse.validate(), Err(SttError::Config(_))), "overlap > chunk rejected");
    }
}
```

- [ ] **Step 4: Implement `SttStream`** (`crates/stt/src/lib.rs`)

Threading: two mutexes, strict order **engine → input** (never reverse). `push_pcm` takes `input` only (short); `poll`/`preview_tail`/`end` take `engine`, and `poll` briefly takes `input` inside to drain. No thread, no channel, no callback → no deadlock. Emits one `FinalizedSegment` per newly-finalized `Word`, each carrying its whisper segment's absolute `start_ms`/`end_ms` (all words from one segment share that coarse span — good enough for the live board; word-precise timestamps are Deferred 4). `decode_window` computes the finalize horizon from the window's absolute start and hands the raw segments (not a flattened word list) to the finalizer.

```rust
use std::sync::Mutex;

use chunk::Chunker;
use decoder::Decoder;
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
                finalizer: Finalizer::new(),
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
        let decoder = whisper::WhisperDecoder::open(model, &cfg.language)?;
        Ok(Self::with_decoder(Box::new(decoder), cfg, vocab))
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
```
> **`Send + Sync`:** the `Box<dyn Decoder>` is `Send` (trait bound); wrapped in `Mutex`, `SttStream` is `Send + Sync`. Plan 07 wraps it `Arc<SttStream>` and UniFFI exposes `push_pcm`/`poll`/`preview_tail`/`end` as `&self` methods — no async, no callback interface, which is precisely why this can't deadlock across the FFI boundary.

- [ ] **Step 5: Verify** `cargo test -p stt` — all pure tests green (no feature, no model).

- [ ] **Step 6: Commit**
```bash
git add -A && git commit -m "feat(stt): SttStream — caller-driven pump, bias injection, flush-on-end (DONE)"
```

---

### Task 5: The whisper backend (`whisper` feature) + real-model gate

**Files:** create `crates/stt/src/whisper.rs`.

The only file that imports `whisper-rs`. Mirrors the spike's `make_ctx`/`make_params`/`decode` (`spikes/stt-whisper/src/main.rs`), now behind the trait.

- [ ] **Step 1: Implement** (`crates/stt/src/whisper.rs`)

```rust
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
```

- [ ] **Step 2: Real-model smoke test** (`crates/stt/src/whisper.rs`, bottom — env+feature gated, like `anthropic_smoke`)

```rust
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
```

- [ ] **Step 3: Document the model files** — add a `## Models` block to `crates/stt/Cargo.toml`'s neighbouring note or a short `crates/stt/README.md`:
> The crate opens a ggml whisper model the **shell** provisions (download/on-demand-resources is not the crate's job). v1 target files (MIT, from `huggingface.co/ggerganov/whisper.cpp`; `ggml-org` returns 401 today — spike note):
> - `ggml-base.en-q5_1.bin` (~57 MB) — default; RTF 0.009, WER 5.8% clean.
> - `ggml-small.en-q5_1.bin` (~182 MB) — higher accuracy; RTF 0.021, WER 4.7% clean.
> Selection (base vs small, quality vs size/battery) is a shell/config decision, informed by the pending on-device iPhone tier (`RESULTS.md` Table 4).

- [ ] **Step 4: Verify**
- `cargo test -p stt` (no feature) — green, whisper.rs not compiled.
- `cargo test -p stt --features whisper` — compiles the native stack; the smoke test is `#[ignore]`d so it's skipped.
- Manual (dam, on device/mac with a model): `MURMUR_WHISPER_MODEL=… cargo test -p stt --features whisper -- --ignored`.

- [ ] **Step 5: Commit**
```bash
git add -A && git commit -m "feat(stt): whisper.cpp backend behind `whisper` feature; env-gated real-model smoke"
```

---

### Task 6: Integration contract, workspace-green verification, docs

**Files:** create `crates/stt/tests/stream_append_only.rs`; modify `README.md`.

- [ ] **Step 1: End-to-end append-only integration test** (public API only, no feature, no model)

```rust
//! Append-only streaming contract (spec Rev 2 §2) via the public API and a
//! scripted decoder using the REALISTIC time-shifted composition model (window
//! k+1's segments start at chunk-relative cs=0, four seconds later in absolute
//! time; only the 1 s overlap word repeats). Proves the finalized stream Plan 05's
//! LiveExtractor consumes finalizes incrementally, dedups the overlap, never
//! revises a committed word, and end() flushes only the bounded tail.

use stt::{RawSegment, ScriptedDecoder, SttConfig, SttStream};

fn seg(cs0: i64, cs1: i64, t: &str) -> RawSegment {
    RawSegment { start_cs: cs0, end_cs: cs1, text: t.into() }
}

#[test]
fn finalized_stream_is_append_only_across_a_session() {
    // Sentence "the french drain needs regrading before the pour today" spoken
    // over ~13 s. Each window re-decodes only its 1 s overlap head; W1's overlap
    // re-transcribes the held word "needs" imperfectly as "kneads" — the merge's
    // TIME FALLBACK drops that divergent re-decode, keeps W0's first decode, and
    // never duplicates or revises a committed word.
    let decoder = ScriptedDecoder::new(vec![
        // W0 [0,5s] horizon 4s: "the french drain" ≤4s finalizes; "needs" held
        vec![seg(0, 180, "the french"), seg(180, 360, "drain"), seg(360, 480, "needs")],
        // W1 [4,9s] horizon 8s: overlap re-decodes "needs" as "kneads" (dropped by
        // the time fallback → first decode "needs" wins); extends; "before" held
        vec![seg(0, 80, "kneads"), seg(80, 300, "regrading"), seg(300, 480, "before")],
        // W2 [8,13s] horizon 12s: overlap re-says "before", extends; "today" held
        vec![seg(0, 80, "before"), seg(80, 300, "the pour"), seg(300, 480, "today")],
        // flush [12,~13s] horizon ∞: re-says "today"
        vec![seg(0, 80, "today")],
    ]);
    let stream = SttStream::with_decoder(Box::new(decoder), SttConfig::default(), &[]);
    stream.push_pcm(&vec![0.0; 208_000]); // ~13 s → three windows (5s/1s → step 4s)

    // One poll drains every ready window; loop until it stops finalizing new words.
    let mut finalized = Vec::new();
    loop {
        let batch = stream.poll().unwrap();
        if batch.is_empty() {
            break;
        }
        finalized.extend(batch);
    }
    finalized.extend(stream.end().unwrap()); // DONE flushes the held tail

    let text: Vec<&str> = finalized.iter().map(|s| s.text.as_str()).collect();
    // Incremental, in order, append-only. "the french drain" was committed in W0
    // and is never revisited (that audio is behind the window, dropped by the
    // Chunker) — the stream only ever appended.
    assert!(text.starts_with(&["the", "french", "drain"]));
    assert!(text.contains(&"regrading") && text.contains(&"pour") && text.contains(&"today"));
    // Overlap words are finalized exactly once (dedup), not doubled.
    assert_eq!(text.iter().filter(|w| **w == "before").count(), 1, "overlap deduped");
    assert_eq!(text.iter().filter(|w| **w == "today").count(), 1);
    // W1 re-decoded the held word "needs" as "kneads"; the time fallback keeps the
    // first decode and drops the divergent one — no duplication, "kneads" nowhere.
    assert!(text.contains(&"needs"), "first decode of the disputed overlap survives");
    assert!(!text.contains(&"kneads"), "divergent re-decode never reaches committed output");
    assert_eq!(text.iter().filter(|w| **w == "needs").count(), 1, "disputed overlap not duplicated");

    // Absolute-ms timestamps are monotonic — append-only in time.
    let mut prev = 0;
    for s in &finalized {
        assert!(s.start_ms >= prev);
        prev = s.start_ms;
    }
}
```
> The `loop`/`break` form (not the earlier `while { … }` statement-expression) is the clean equivalent — a single `poll()` already drains all currently-ready windows, so the loop just retries until a poll finalizes nothing new.

- [ ] **Step 2: Document the murmur-core wiring contract** (in `crates/stt/README.md` — the seam Plan 07 implements)

> **Integration with `murmur-core` (deferred to Plan 07 — the FFI/shell tick loop):**
> `crates/stt` and `murmur-core` do **not** depend on each other. The shell owns both pumps and wires them:
> ```
> // shell background thread, on cadence:
> stt.push_pcm(pcm);                                  // audio thread hands off buffers
> for seg in stt.poll()? {                            // append-only finalized segments
>     store.append_transcript(&session_id, &format!("{} ", seg.text))?;
> }
> live_extractor.maybe_extract().await?;              // Plan 05: cursor advances over new transcript
> // on DONE:
> for seg in stt.end()? { store.append_transcript(&session_id, &format!("{} ", seg.text))?; }
> // then queue end-of-session process() — the AUTHORITATIVE pass (Plan 04).
> ```
> Why deferred, not built here: (1) cadence is shell policy (Plan 05 Deferred 3 already put the LiveExtractor tick in the shell); (2) both `stt.poll` and `LiveExtractor.maybe_extract` are shell-driven pumps with no core-side coupling (Plan 05 self-review constraint 4); (3) building it here forces an `stt ↔ murmur-core` dependency both plans avoid. The contract above is the whole seam — Plan 07 implements it across UniFFI.

- [ ] **Step 3: README plan-series line** (`README.md`)
```markdown
Done: 01 foundation, 02 memory + reflection + context assembler, 03 domain + storage, 04 processing pipeline + reflection coordinator, 05 live extraction, 06 STT crate.
Next: 07 (FFI: UniFFI boundary — wire STT + LiveExtractor + processing into the platform shell).
```

- [ ] **Step 4: Full verification**
- `nix develop -c cargo test --workspace` → all green **on default features** (this is the CI invocation: no model, no cmake/clang needed at compile time for the default `stt` build).
- `nix develop -c cargo build -p stt --features whisper` → native stack compiles.
- `nix develop -c cargo clippy --workspace --all-targets` → zero warnings (fix mechanically; no `#[allow]`; STOP and report if a fix changes behavior). Also run `cargo clippy -p stt --features whisper --all-targets`.

- [ ] **Step 5: Commit**
```bash
git add -A && git commit -m "test(stt): append-only e2e; docs: murmur-core wiring contract; plan 06 done"
```

---

## Deferred (named, for later plans)

1. **The full STT → live-extraction tick loop (Plan 07, FFI + shells).** The contract is documented (Task 6); the loop that couples `stt.poll` → `append_transcript` → `LiveExtractor.maybe_extract` is shell orchestration across UniFFI. Deliberately not built here to keep `stt` and `murmur-core` decoupled.
2. **On-device iPhone tier verification (`RESULTS.md` Table 4, PENDING).** The GO is provisional pending a device check: `base.en`/`small.en` RTF<1.0 and no thermal kill over 10 min locked. Mac margins (RTF 0.009–0.02) make this expected-pass, but it is the one unretired GO condition — run before shipping (needs dam's device; `spikes/stt-whisper/ios/README.md`).
3. **Trie / logit-bias hotword decoder (research §4; the biasing ceiling).** v1 uses `initial_prompt` (+10–19 pp, proven). The deeper decoder-internal biasing (19–22% B-WER lit. gains) is an optimization, not a prerequisite — swaps in behind `build_bias_prompt`'s seam without touching the pipeline. Also untested: a full 100-term list against real noisy jobsite audio (the case most likely to hallucinate).
4. **Word-precise timestamps.** v1 finalized segments carry **segment-coarse** time: every word expanded from one whisper segment shares that segment's absolute `start_ms`/`end_ms` (a batch of words finalized from one `ingest` therefore shares the source segment's span, not a per-word time). Word-level alignment (whisper cross-attention) for audio-scrubbing UI is a later concern — `FinalizedSegment` already has the fields.
5. **Model download / on-demand resources / model selection UI.** The crate opens a provisioned file. Fetching, storage, and base-vs-small selection are shell/config concerns (Task 5 doc).
6. **Rolling prior-transcript context in the prompt.** The spike (and v1) use the prompt slot purely for bias terms. Carrying recent transcript for cross-chunk coherence could help but risks diluting the ≤100 bias terms (whisper's 224-token prompt window) — revisit only with evidence.
7. **Battery/thermal instrumentation, chunk-size auto-tuning.** Adaptive chunk/model selection under thermal pressure (research Q8) is a shell-driven policy once on-device numbers exist. Config is already exposed (`chunk_secs`, model choice).
8. **Android backend.** Option B's payoff (research §6): the same pure engine + `Decoder` trait; only a JNI/NDK `WhisperDecoder` and audio handoff differ. Out of v1 scope; the trait keeps the door open cheaply.
9. **Diarization (FluidAudio/pyannote).** Nice-to-have per the brief; no first-party on-device path. Not v1.
10. **VAD-gated decoding.** Skipping silent windows to cut battery/compute is a real optimization but adds a component; the finalizer already tolerates empty hypotheses. Deferred until battery numbers justify it.

## Self-Review Notes

- **Spec coverage:** Rev 2 §2 on-device streaming STT ✓ (Tasks 2–5); append-only finalized stream ✓ (Task 3 finalizer + Task 6 e2e — a committed word is never revised, carrying the spike's append-only + no-double-emit-of-overlap invariants as behavior, and adding the starvation/incremental-progress guard the old design lacked); volatile preview tail ✓ (`preview_tail`); DONE = flush-not-drop ✓ (`end()`, supersedes cancel-for-speed canon); §vocabulary point 3 biasing ✓ (Task 4 `build_bias_prompt`, initial_prompt injection, ≤100 cap); offline/on-device ✓ (no network, model is a local file). Live-is-provisional / process() authoritative ✓ (stated design constant + integration contract queues process() as truth).
- **The four hard requirements, discharged:** (1) Decode trait — `Decoder` isolates whisper; the entire pipeline is tested against `ScriptedDecoder` with zero whisper dependency (Tasks 2–4, 6). (2) Hermetic CI — `whisper-rs` is `optional`, `default = []`; `cargo test --workspace` compiles on Linux with no model/cmake/clang; real-model test is `#[ignore]` + env-gated + feature-gated (three locks). (3) Threading — caller-driven pump, no thread/channel/callback, strict two-lock order (engine→input) = can't deadlock; `Send + Sync` for a direct UniFFI `&self` object. (4) Build hygiene — `flake.nix` gains cmake/clang/LIBCLANG_PATH (Task 1) for `--features whisper`; CI stays on default features. Feature-gate chosen over macOS-only workspace **because** existing Linux CI for the other three crates must stay green.
- **Design constants traced to measurement:** 5 s/1 s default and the **word-level text-overlap dedup + time-horizon finalize** (the measured-good *dedup-reassembly* pathway, NOT the naive segment-level rule) come straight from `RESULTS.md` Table 2 (19% vs 80% WER); the finalizer productionizes the spike's `reassemble_dedup` merge + `finalize` horizon (`stream.rs`). base.en/small.en q5_1 from Table 1/3; initial_prompt biasing from Table 3 (+10–19 pp, 0 hallucination); flush-on-end from the ∞-horizon final chunk in `stream.rs::finalize`.
- **API surface for Plan 07 (FFI):** `SttStream::{with_model, with_decoder, push_pcm, poll, preview_tail, end}` — all `&self`, sync, `Send + Sync`. No async, no callback interface. UniFFI wraps `Arc<SttStream>`. `FinalizedSegment { start_ms, end_ms, text }` and `SttConfig` are plain structs (add `serde`/UniFFI derives in Plan 07 if the boundary needs them — not added now to keep deps minimal).
- **Judgment calls for reviewers:** (a) **caller-driven pump over internal worker** — the shell already ticks LiveExtractor; a worker adds a shutdown protocol + mpsc + callback interface (3 deadlock surfaces) for nothing. (b) **feature-gated optional whisper** over macOS-only workspace — keeps `cargo test --workspace` cross-platform/hermetic; the cost is a `#[cfg(feature)]` on one file and one constructor. (c) **time-anchored incremental overlap-merge** — the productionized form of the spike's *measured-good* `reassemble_dedup` (text suffix/prefix merge) gated by the `finalize` time horizon. This replaces an earlier prefix-agreement design that was **wrong**: the bounded-memory Chunker emits time-shifted standalone windows, so window k+1 is not a superstring of k and position-0 prefix agreement never matches — nothing would finalize mid-session. The fix uses absolute time (to place words + set the horizon) plus the spike's text merge (to dedup the overlap); `pending` stays O(one chunk). The merge has **two seams**: the text suffix/prefix match (precise, when the overlap re-decoded identically) and a **time-anchored first-decode-wins fallback** (coarse, when it re-decoded differently). The fallback is load-bearing: without it, the spike's all-or-nothing text merge appends the whole re-decoded window on any *partial* overlap disagreement, duplicating the overlap phrase into committed output — a correctness bug caught in review. With it, a disagreement costs at most one first-decode-wins word (priced into WER), never duplication. (d) **integration loop deferred to Plan 07** — building it here couples `stt`↔`murmur-core`, which both plans avoid; the contract is fully documented instead. (e) **segment-coarse timestamps in v1** — the live board doesn't need word-precise times; each finalized word carries its whisper segment's absolute span, and the struct carries the fields for word-precise later. (f) **`end()` idempotent, flushes** — DONE means finalize everything pending; the old cancel-for-speed canon is explicitly superseded (this is a capture whose transcript feeds the authoritative process()).
- **Test-count checkpoint:** T1 +2 (decoder), T2 +4 (chunk), T3 +4 (finalize — incl. the rewritten overlap-disagreement test), T4 +2 (bias) +4 (stream — incl. `config_rejects_overlap_ge_chunk`), T5 +1 (#[ignore] smoke), T6 +1 (e2e) ≈ **18 new**, of which 17 run in default CI. Counts are expectations, not gates.
- **Constraints surfaced for Plan 07:** (1) `SttStream` is `&self`/sync — wrap `Arc`, call `poll` from a background thread (long Metal decode); do NOT call from the audio render callback (research Q6). (2) The shell must provision the model file and pass its path — the crate never downloads. (3) The shell owns the tick cadence for BOTH `stt.poll` and `LiveExtractor.maybe_extract`, and appends finalized segments to `Store::append_transcript` between them. (4) Build the app/FFI target with `--features whisper`; CI without it. (5) `SttConfig`/`FinalizedSegment` may need UniFFI/serde derives at the boundary.
```
