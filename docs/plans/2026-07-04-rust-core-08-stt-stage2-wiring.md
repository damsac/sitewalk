# Murmur Rust Core — Plan 08: STT Stage 2 — mic audio → `crates/stt` → the append path

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Rust tasks are **hermetic** (ScriptedDecoder/MockProvider — no model, no cmake, no Metal, no network). Swift tasks carry sim/device-testable checks. `cargo test --workspace` must NEVER require the `whisper` feature, a model file, or a native toolchain — this is the load-bearing CI invariant of the whole STT effort (Plan 06 requirement 4) and it does not bend here.

**Goal:** Wire **real microphone audio** end-to-end so a spoken walk fills the live board on device: iOS mic capture → 16 kHz mono f32 PCM across FFI → the `crates/stt` `SttStream` (whisper) → finalized transcript text → the **EXISTING `WalkSession` append path** (`Store::append_transcript` → `LiveExtractor::maybe_extract` → `boardUpdated`) → DONE → the existing two-phase `finish()` → document in review. This is Plan 07 **D8 stage 2**, explicitly designed to be **additive**: a new `push_audio` FFI method, the text `append_transcript` path **untouched**.

**What this plan is NOT.** It does not rewrite `append_transcript`, the `LiveExtractor` actor, the tick/finish serialization (D3b), the board-snapshot event, or `finish()`'s two-phase processing. It does not build model download/management UI, word-precise timestamps, trie/logit biasing, diarization, or Android. It wires the mic → STT → the append seam that already exists, plus the minimum event surface for the UI. **Part C** adds construction-site noise robustness (a VAD/no-speech gate, an Apple voice-processing knob, and a noise eval) — sequenced after the core wiring, off the milestone critical path. See **Non-goals** for the full de-scope list.

**Hard dependencies (all DONE, main @ `8381d56`):** Plan 06 (`crates/stt`: `SttStream` caller-driven pump, `Decoder` seam + `ScriptedDecoder`, `WhisperDecoder` behind the `whisper` feature, `build_bias_prompt` ≤100 terms, `end()` flush). Plan 07 (`crates/ffi`: `MurmurEngine`, `WalkSession` with sync fire-and-forget `append_transcript` + tick, `boardUpdated` snapshots, two-phase `finish()`, key hygiene). Plan 06a (`source` column + atomic-at-finish swap). This plan sits on top of all three and reuses their seams verbatim.

**Carry-forward constraints (from Plan 07 / ROADMAP §5 "07 carry notes"):**
- **Fallible constructors are being fixed in a parallel lane.** Assume `MurmurEngine::new` and `begin_walk` **may become `Result`-returning** (a bad `db_path` or, now, a bad `stt_model_path`, should surface as an error, not `expect()`-panic). This plan's model-load failure path is designed to ride that fallibility — see D5. If the parallel lane hasn't landed when Task 5 runs, the model-load error is surfaced through whatever construction shape exists then (Result if landed; a stored `Option<SttStream> = None` + a logged degrade if not) — Task 5 Step 3 branches on this explicitly.
- STT DONE flush-vs-speed is an **OPEN CANON question** (sac ack pending, ROADMAP "Decisions needed"). This plan picks a **default (flush)** behind a config toggle and flags it — see D6.

---

## Architecture — decisions, justified (reviewers read these first)

### D1. Swift captures audio; Rust never touches the mic
Audio session, permissions, and interruption handling stay in Swift (exactly as `SpeechSource` does today). Swift's `AVAudioEngine` taps the input node, an `AVAudioConverter` down-mixes/resamples to **16 kHz mono f32** (what whisper wants, `SttConfig.sample_rate = 16_000`), and Swift pushes those PCM frames across FFI. Rust receives only `Vec<f32>`. This mirrors Plan 06 decision 3 ("the crate never touches the mic") and keeps the platform audio-session complexity where the platform APIs live. Rejected: Rust-side capture (would need a CoreAudio/AVFoundation binding across FFI for zero benefit — the shell already owns the audio session for `SpeechSource`).

**Frame size / cadence / backpressure.** The `AVAudioEngine` tap fires on the audio render thread with hardware-rate buffers (typically 48 kHz, ~1024–4800 frames). The tap callback does the **minimum**: convert to 16 kHz f32 and hand the samples to a lock-free-ish Swift buffer; a **separate** (non-render) task calls `session.pushAudio(samples:)` in batches (~every 100–250 ms, i.e. a few thousand 16 kHz samples). `pushAudio` is a **cheap enqueue** (`SttStream::push_pcm` = one short `Mutex<Vec<f32>>` extend — Plan 06); the long Metal decode happens on the Rust pump thread (D2), never on the render thread (Plan 06 requirement 3 / research Q6). Backpressure: whisper RTF ≪ 0.5 on device (spike: base.en 0.009), so the pump drains far faster than real time; the `SttStream` input buffer and `Chunker` free consumed audio (Plan 06 `free_consumed`, O(one window)), so unbounded growth is not a concern for a normal session. If the pump ever falls behind (thermal throttle), the input `Vec` grows but is still bounded by session length — acceptable for v1; adaptive drop is Deferred.

### D2. The pump lives in Rust (a dedicated blocking thread), not in Swift — `SttStream::poll` is never crossed by Swift
Plan 07 D8 already committed to this: *"the bridge will pump it on a dedicated task/thread and feed finalized segments into `append_transcript` — so `append_audio` is a cheap enqueue, decode happens elsewhere."* We honor it. `WalkSession` owns an optional `Arc<SttStream>` and, when audio is enabled, spawns **one dedicated `std::thread` per session** (NOT a tokio async task) that loops: wait for a "new PCM" notification (or a periodic tick) → call `stt.poll()` (the long, **blocking** Metal decode) → for each finalized `FinalizedSegment`, feed its text into the session's **existing** `append_transcript` path.

Why a dedicated OS thread and not `tokio::spawn`: `SttStream::poll()` is a **synchronous blocking** call (Metal decode); running it on a tokio async worker would block that worker and starve `LiveExtractor` ticks / `finish()`. `spawn_blocking` is the tokio-idiomatic alternative, but a single long-lived pump thread with explicit start/stop is simpler to reason about, needs no per-poll task churn, and matches Plan 06's "the shell owns the tick loop" posture (here the Rust bridge *is* the shell for the STT pump). The thread parks on a `Condvar`/channel between polls, so it's cheap when idle.

**The pump feeds the existing append seam — it does not reimplement it.** For each finalized segment the pump calls `self.append_transcript(text)` — the same sync fire-and-forget method Swift's text path calls (`session.rs:154`). That method already: writes the chunk under a scoped `Store` lock, then spawns the `LiveExtractor` tick on `runtime_handle`, which emits `boardUpdated`. So the audio path reuses **100%** of the live-board machinery, including the tick/finish exclusion (D3b): a pump-triggered tick queues behind `finish()`'s held extractor mutex exactly like a Swift-triggered one. **Lock order is preserved:** `poll()` holds `SttStream`'s internal `engine → input` locks (Plan 06) and **releases them before returning** the segments; `append_transcript` is called *after* `poll()` returns, so the STT engine lock is never nested with the `Store`/extractor locks. No new lock-inversion surface.

### D3. `push_audio` and the STT stream are additive and feature-independent at the FFI surface
The FFI method `WalkSession::push_audio(samples: Vec<f32>)` and the pump exist **regardless** of the `whisper` cargo feature — because the Swift binary always links the whisper-enabled build, and `cargo test --workspace` (feature off) must still compile the same method set. The split:
- `stt` is an **always-on** path dependency of `crates/ffi` (its pure logic — `SttStream`, `ScriptedDecoder`, `Chunker`, `Finalizer`, `build_bias_prompt` — has no native deps).
- `crates/ffi` gains a `whisper` feature that forwards to `stt/whisper`. Only the **construction of a real `WhisperDecoder` from a model path** is `#[cfg(feature = "whisper")]`.
- With the feature **off** (CI + `cargo test --workspace`): `begin_walk` builds a `WalkSession` whose `stt` field is `None` (text stage-1 still works untouched); `push_audio` is a no-op (or, in tests, the session is built via a test constructor that injects a `ScriptedDecoder`-backed `SttStream` so the pump wiring is exercised hermetically).
- With the feature **on** (device/sim build via `build-ffi.sh --features whisper`): `begin_walk` builds `SttStream::with_model(model_path, cfg, &bias_terms)` and starts the pump.

This is exactly Plan 06's feature-gate discipline extended one crate up. `cargo test --workspace` stays hermetic; the whisper path is compiled only for the app targets.

### D4. Partials vs finalized: UI sees a greyed preview + committed text; only **finalized** text feeds append/extraction
Today the UI transcript comes from the Swift-side pump reading `src.chunks` and doing `transcript += chunk`. In stage 2 the transcript text **originates in Rust** (whisper output), so the bridge must surface it. Two new `WalkEvent` cases carry it to the UI, emitted from the pump:
- `WalkEvent.transcriptCommitted(text)` — newly **finalized** segment text. The UI appends it to the visible transcript. This is the **same text** the pump feeds to `append_transcript` (extraction sees finalized only — Plan 06's append-only contract).
- `WalkEvent.transcriptPreview(text)` — the volatile `SttStream::preview_tail()` (greyed, un-finalized hypothesis). Displayed greyed, **never persisted, never extracted** (Plan 06). Wiring the greyed tail into the UI is a **nice-to-have** for the milestone (makes the demo feel live); committed text is the required path. If time-boxed, emit `preview` but leave the greyed rendering behind a small view flag.

Rationale: "what the UI sees live = finalized + optional greyed preview; what feeds extraction = finalized only" is Plan 06's design constant made concrete at the boundary. Extraction/append is unchanged — it still receives plain text via the existing internal call.

### D5. Model file lifecycle: **bundle `base.en` q5_1 for the milestone**; path flows through `EngineConfig`; load failure is a construction error (not a panic)
- **Which model:** `ggml-base.en-q5_1.bin` (**~60 MB** on disk — the `RESULTS.md` table + the actual file; the spike doc's line 216 says "~57 MB" and is **stale** — Task 8/README use 60 MB; spike RTF 0.009 / WER 5.8% clean) — the spike verdict says base.en is sufficient. `small.en` (~182 MB) is a config swap later, and the noise eval (Part C) may promote it (RTF headroom makes it ~free).
- **Bundle vs download:** **bundle** for the milestone. Rationale: spec §1 is offline-first / bulletproof capture — a first-run download that can fail on a jobsite with no signal violates the core promise, and ~60 MB is an acceptable IPA bump for a milestone. On-demand-resources / a download manager is the scalable answer and is **Deferred** (Plan 06 Deferred 5 — "model management is a shell concern").
- **Path flow:** `EngineConfig` gains `stt_model_path: Option<String>` (and `stt_flush_on_finish: bool`, D6). Swift resolves `Bundle.main.path(forResource: "ggml-base.en-q5_1", ofType: "bin")` and passes it. `begin_walk` reads it: under `#[cfg(feature="whisper")]` + `Some(path)` → `SttStream::with_model`; else `None` (text path).
- **Load failure is fallible, never a panic:** a missing/corrupt model must NOT `expect()`-crash the host (Plan 07's whole finish-degrade ethos). This rides the **fallible-constructor carry-forward**: when `begin_walk` becomes `Result`, a `SttStream::with_model` error propagates as `Err`. Until then, Task 5 stores `stt: None` on load failure and `log`s the degrade (the walk falls back to text-only / no audio ingest rather than crashing). Either way: **no unwind across FFI.**
- **CI stays hermetic:** the model file is never referenced by `cargo test`; `with_model` is `#[cfg(feature="whisper")]`; the whisper feature is off in the workspace test build.

### D6. Pause / resume / finish semantics — finish **flushes** by default (behind a toggle), flagged for canon
- **Pause:** Swift pauses `AVAudioEngine` (stops the tap); no PCM flows; the Rust pump parks (no new-PCM signal). `SttStream` retains its buffer. **Resume:** Swift restarts the tap; PCM resumes; the pump wakes. No Rust-side state reset — the `Chunker`/`Finalizer` continue.
- **Finish (the OPEN CANON question):** default **flush** — `finish()` stops the pump, calls `stt.end()` (Plan 06: decodes the remaining buffered audio, finalizes the last utterance), feeds those final segments through `append_transcript`, **then** runs the existing two-phase process. Controlled by `EngineConfig.stt_flush_on_finish` (default `true`). **Rationale for flush-as-default:** the product is a document builder for field work — losing the operator's *last spoken line* (often the price or the closing instruction) to save one short-window decode is the wrong trade; Plan 06 explicitly designed `end()` to supersede the old cancel-for-speed canon, and base.en RTF ≪ 0.5 makes the flush decode cost sub-second. **Flagged for dam/sac:** the toggle exists precisely so canon can flip it to `false` (speed) without a code change. If the team lands "speed," set the default `false`; the flush path stays available.

### D7. Simulator vs device + a scripted-audio test path
- **Metal on simulator is not guaranteed.** whisper.cpp Metal targets real GPUs; on the iOS simulator Metal may be absent or fall back. `WhisperDecoder::open` uses `use_gpu(true)` (Plan 06) — on sim this should degrade to CPU (slower, still functional) rather than fail; Task 8 verifies and, if GPU init hard-fails on sim, exposes a config to force CPU for sim builds. **Real STT performance is a device concern** and ties to the one unretired GO condition (ROADMAP: iPhone T5 device spike, `spikes/stt-whisper/ios/README.md`).
- **Scripted-audio path (testable without a mic).** Two layers: (a) **hermetic Rust** — the pump + append wiring is fully tested with a `ScriptedDecoder`-backed `SttStream` (no model, no audio, no feature); (b) **Swift** — a `WavFileAudioSource` that reads a bundled 16 kHz fixture WAV and pushes its PCM through `push_audio`, so the whisper path can be exercised end-to-end on device (and on sim if CPU/Metal works) without a live mic. The existing **text** `ScriptedSource` demo path stays the default screenshot/CI flow (D8/Plan 07 D10) — it does not depend on whisper at all.

### D8. Biasing terms come from memory `vocabulary` (+ template seed), set once at `begin_walk`
`SttStream`'s bias prompt is fixed at construction (Plan 06 `build_bias_prompt`). `begin_walk` is where the template and `Memory` are both in hand, so it assembles the ≤100-term vocabulary there: `memory.section_texts("vocabulary")` (verified to exist, `harness/src/memory/mod.rs:112`; the section STT biasing was designed to read, `mod.rs:45`) plus an optional small per-template seed list, capped at `SttConfig.max_bias_terms` (100). Passed to `SttStream::with_model`. No new memory plumbing — this reads an existing section.

---

## File Structure

```
crates/
  ffi/
    Cargo.toml                 # MODIFY: stt path dep (always) + `whisper` feature → stt/whisper
    src/
      engine.rs                # MODIFY: EngineConfig + MurmurEngine gain stt_model_path, stt_flush_on_finish
      session.rs               # MODIFY: WalkSession.stt + pump thread; push_audio(); async finish() flush;
                               #         async cancel() (stop pump + delete_session); bias-terms assembly
      events.rs                # MODIFY: WalkEvent.TranscriptCommitted / TranscriptPreview
      convert.rs               # (no change expected — segments don't cross FFI)
    tests/
      audio_pump_e2e.rs        # NEW: push_audio → pump → append → boardUpdated + transcript events
  stt/                         # Part C (Task 11):
    src/lib.rs                 # MODIFY: SttConfig thresholds; per-window VAD gate in poll() (after take_ready_window)
    src/vad.rs                 # NEW: energy/VAD helper (pure, hermetic)
    src/decoder.rs             # MODIFY: RawSegment gains no_speech_prob (default 0.0) — breaks 4 struct-literal sites
    src/finalize.rs            # MODIFY: drop segments above the no_speech_prob threshold
    src/whisper.rs             # MODIFY: WhisperDecoder populates no_speech_prob (cfg)
  # murmur-core: NO change — cancel() uses the existing, tested Store::delete_session (issue #3 fix)
apps/ios/
  Sources/Engine/
    AudioCaptureSource.swift   # NEW: AVAudioEngine tap → 16kHz mono f32 → session.pushAudio; voice-proc knob (Task 10)
    WavFileAudioSource.swift   # NEW: bundled 16kHz WAV → push_audio (scripted-audio test path, D7)
    TranscriptSource.swift     # MODIFY: audio-source protocol note (STT now Rust-side, not SFSpeech)
    MurmurEngine.swift         # MODIFY: pushAudio + cancel passthrough; transcript events → UI; model path
  Sources/App/
    AppModel.swift             # MODIFY: live=1 uses the audio path; transcript from Rust events; discardWalk → cancel()
    GalleryApp.swift           # MODIFY: EngineConfig with stt_model_path + flush toggle + voiceproc arg
  Packages/MurmurCoreFFI/
    Resources/ggml-base.en-q5_1.bin   # NEW (~60 MB, gitignored large binary; provisioned by build-ffi.sh doc)
  project.yml                  # MODIFY: bundle the model resource; whisper build via build-ffi.sh
  build-ffi.sh                 # MODIFY: build crates/ffi with `--features whisper`
spikes/stt-whisper/            # Part C (Task 12): noise SNR sweep harness + RESULTS.md append
README.md                      # MODIFY: plan-series line
```

Run cargo via the dev shell or `nix shell nixpkgs#cargo nixpkgs#rustc -c cargo <cmd>` from the repo root. Default builds/tests need **no** native toolchain; the app/FFI build adds `--features whisper` (needs the dev shell's cmake/clang, Plan 06 Task 1). iOS: `cd apps/ios && xcodegen generate && xcodebuild … -destination 'platform=iOS Simulator,name=iPhone 17 Pro'`.

---

## Part A — The Rust bridge (hermetic — no model, no feature, no network)

### Task 1: `stt` dependency + `EngineConfig` fields + `WalkSession.stt` slot + test constructor

**Files:** modify `crates/ffi/Cargo.toml`, `crates/ffi/src/engine.rs`, `crates/ffi/src/session.rs`.

**Goal:** Make `crates/ffi` depend on `stt` (always), add the `whisper` forwarding feature, extend `EngineConfig`, and give `WalkSession` an optional `SttStream` slot + a test constructor that injects a `ScriptedDecoder`-backed stream — all compiling and testing with the feature **off**.

- [ ] **Step 1 — failing test** (`engine.rs` tests): extend `config_debug_redacts_the_api_key`-style coverage — construct an `EngineConfig` with `stt_model_path: Some("…")` and `stt_flush_on_finish: true`, assert the `Debug` output still redacts `api_key` and does **not** need the new fields to leak anything sensitive (model path is fine to print). Add `stt_defaults_are_sane`: a config with `stt_model_path: None` builds providers normally.
- [ ] **Step 2 — run to see failure** (new fields don't exist).
- [ ] **Step 3 — implement:**
  - `Cargo.toml`: add `stt = { path = "../stt" }` under `[dependencies]`; add to `[features]`: `whisper = ["stt/whisper"]`. (Do **not** make `stt` optional — its pure logic is always needed for the pump + tests.)
  - `engine.rs`: `EngineConfig` gains `pub stt_model_path: Option<String>` and `pub stt_flush_on_finish: bool`. Extend the hand-written `Debug` to print both (neither is secret). Update both `#[cfg(test)]` config literals in the file.
  - `session.rs`: `WalkSession` gains `stt: Option<Arc<stt::SttStream>>` and `flush_on_finish: bool` fields; thread them through `WalkSession::new` and the `test_session` helper (default `stt: None`, `flush_on_finish: true`). No behavior change yet — the field is unused until Task 2.
- [ ] **Step 4 — verify:** `cargo test -p ffi engine` green; `cargo build -p ffi` (feature off) compiles; `cargo build -p ffi --features whisper` compiles (native stack builds in the dev shell).
- [ ] **Step 5 — commit:** `feat(ffi): stt dep + whisper feature forward; EngineConfig stt fields; WalkSession stt slot`

---

### Task 2: `push_audio` + the pump thread → the existing append path (hermetic via `ScriptedDecoder`)

**Files:** modify `crates/ffi/src/session.rs`.

**Goal:** Add the FFI `push_audio` method and the dedicated pump thread that polls the `SttStream` and feeds finalized text into the **existing** `append_transcript` path — proven with a `ScriptedDecoder`-backed `SttStream` and a `MockProvider` live extractor (no model, no feature).

- [ ] **Step 1 — test-only audio session constructor + failing test.** Add a `#[doc(hidden)] pub` test constructor (mirroring `MurmurEngine::with_providers`'s "pub not cfg(test)" reasoning so the integration-test binary in Task 8's e2e can call it) that builds a `WalkSession` with an injected `SttStream::with_decoder(Box::new(ScriptedDecoder::new(scripts)), SttConfig::default(), &[])`. Then the failing test:

```rust
    #[tokio::test]
    async fn push_audio_pumps_stt_and_feeds_the_append_path() {
        // ScriptedDecoder yields enough finalized text that the live MockProvider
        // extractor lands one item → exactly one BoardUpdated snapshot.
        // Live provider: add_item("order lumber") + end_turn, min_new_chars = 1.
        // push_audio(9s of zeros) → pump polls → finalized text → append_transcript
        //   → tick → BoardUpdated{items:[order lumber]} on the listener channel.
    }
```

  The scripted segments reproduce Plan 06's realistic time-shifted composition (window k+1 at chunk-relative cs=0; only the 1 s overlap repeats), enough words to clear `LiveExtractor.min_new_chars`.
- [ ] **Step 2 — run to see failure** (`push_audio` and the pump don't exist).
- [ ] **Step 3 — implement:**
  - `WalkSession` gains pump state: a `Mutex<Option<PumpHandle>>` (or an `AtomicBool` stop flag + a `Condvar`/`std::sync::mpsc` "wake" signal + a `JoinHandle`). Keep it simple: an `Arc<(Mutex<PumpState>, Condvar)>` where `PumpState { stop: bool, wake: bool }`.
  - `start_pump(self: &Arc<Self>)`: if `stt` is `Some`, spawn ONE `std::thread`. Loop:
    1. Park on the `Condvar` until `wake || stop`.
    2. If `stop`, break.
    3. Clear `wake`; call `stt.poll()`. On `Ok(segs)` with non-empty text, for each segment call `self.append_transcript(seg.text + " ")` (reuse the existing method — store write + spawned tick + board snapshot) and (Task 3) emit the transcript event. On `Err`, `log` and continue (a decode error must not kill the pump — capture-never-lost ethos).
  - `#[uniffi::export] pub fn push_audio(self: Arc<Self>, samples: Vec<f32>)`: if `stt` is `Some`, `stt.push_pcm(&samples)` (cheap enqueue) then signal `wake` + notify the Condvar. No-op if `stt` is `None` (text-only session). **This must not block** — it's called from a Swift background task fed by the audio thread (D1).
  - Start the pump from `begin_walk` (Task 5 wires the real stream; here the test constructor starts it) — or lazily on first `push_audio`. Prefer explicit: `begin_walk`/test-ctor calls `start_pump` after construction.
  - **Lock-order note in code:** `poll()` returns before `append_transcript` is called, so the `SttStream` engine lock is never held across the `Store`/extractor locks (D2). Add a comment asserting this.
- [ ] **Step 4 — verify:** `cargo test -p ffi session` green (the new test + all existing session tests, incl. `tick_cannot_interleave_with_finish`, still pass). Confirm no deadlock: a second `push_audio` after the first tick completes doesn't hang.
- [ ] **Step 5 — commit:** `feat(ffi): push_audio + dedicated STT pump thread feeding the existing append path`

---

### Task 3: transcript event surface — `TranscriptCommitted` / `TranscriptPreview`

**Files:** modify `crates/ffi/src/events.rs`, `crates/ffi/src/session.rs`.

**Goal:** Surface finalized transcript text (and the volatile preview tail) to the UI without touching the extraction path.

- [ ] **Step 1 — failing test** (`session.rs` tests): after `push_audio` drives a pump pass, the listener received a `TranscriptCommitted` event whose text contains the finalized words, and (if wired) a `TranscriptPreview` carrying the greyed tail. Assert **the same finalized text** also produced the board item (i.e. committed text and extraction input are the same string).
- [ ] **Step 2 — run to see failure** (event cases don't exist).
- [ ] **Step 3 — implement:**
  - `events.rs`: extend `#[derive(uniffi::Enum)] WalkEvent` with `TranscriptCommitted { text: String }` and `TranscriptPreview { text: String }`. (The existing `BoardUpdated { items }` case is unchanged.)
  - **REQUIRED — fix the three now-refutable bindings in `session.rs` (E0005 or this task won't compile):** adding cases to `WalkEvent` makes every irrefutable single-variant destructure a **refutable** pattern, a hard compile error. Three existing test bindings use `let WalkEvent::BoardUpdated { items } = event;` — at **`session.rs:371`, `:411`, and `:475`** (verified against source). Rewrite each to a `match`/`if let`, e.g.:
    ```rust
    let WalkEvent::BoardUpdated { items } = event else {
        panic!("expected BoardUpdated, got {event:?}");
    };
    ```
    (or `match event { WalkEvent::BoardUpdated { items } => { … }, other => panic!("…{other:?}") }`). The `:475` site is inside a `while let Ok(event) = rx.try_recv()` loop asserting no empty board — there it must **skip** non-`BoardUpdated` events (transcript events may now arrive on the same channel) rather than panic: `if let WalkEvent::BoardUpdated { items } = event { assert!(!items.is_empty(), …) }`. Do this rewrite in the SAME commit as the enum change so the crate never lands in a non-compiling state.
  - `session.rs`: in the pump loop, after collecting finalized segments, emit **one** `TranscriptCommitted { text: joined_finalized }` via the listener (before/independent of the board tick — the tick is async and fires on `runtime_handle`; the transcript event is synchronous from the pump). After each poll, also emit `TranscriptPreview { text: stt.preview_tail() }`.
  - **Swift enum exhaustiveness:** adding cases forces `BoardListener.onEvent`'s `switch` to handle them (Task 7). Note this in the plan so the Swift change is coordinated — the FFI enum and the Swift adapter change together.
- [ ] **Step 4 — verify:** `cargo test -p ffi session` green.
- [ ] **Step 5 — commit:** `feat(ffi): WalkEvent transcript committed/preview events from the STT pump`

---

### Task 4: `finish()` flush + `cancel()` (pump-stop, no worker-block) + bias-terms assembly

**Files:** modify `crates/ffi/src/session.rs`, `crates/ffi/src/engine.rs` (a `cancel_walk` export or `WalkSession::cancel`), and — if the store lacks an "abandon" primitive — `crates/murmur-core/src/store/sessions.rs`.

**Goal:** On DONE, flush the final utterance through the append path (default, toggle-gated) **before** the existing two-phase process. On DISCARD, a `cancel()` path stops the pump **and** tombstones the session (closing both the leaked-thread leak and issue #3's SQLite zombie). Neither path may block a tokio worker on `join()`. Assemble the ≤100-term bias vocabulary at `begin_walk`.

- [ ] **Step 1 — failing tests** (`session.rs` tests):
  - `finish_flushes_the_final_utterance`: a `ScriptedDecoder` `SttStream` holds a final buffered utterance that only `end()` finalizes; `push_audio` then `finish()` — assert the flushed final words reached `append_transcript` (visible in the transcript / a `TranscriptCommitted` event) **before** `process()` ran, and the returned `DocumentPayload` reflects the post-flush transcript. With `flush_on_finish = false`, assert the final held utterance is NOT flushed (speed path).
  - `cancel_stops_the_pump_and_tombstones_the_session`: begin an audio session (test-ctor with a `ScriptedDecoder` `SttStream`), `push_audio`, then `cancel()`. Assert: (a) the pump thread has exited (a subsequent `push_audio` does not resurrect it / no new `TranscriptCommitted` arrives); (b) the session is no longer an open `Recording` row — `store.get_session(sid)` reflects the tombstoned/abandoned status (issue #3). No panic, idempotent (a second `cancel()`, or `cancel()` after `finish()`, is a harmless no-op).
  - `bias_terms_from_memory_vocabulary`: a helper `collect_bias_terms(&Memory, template)` returns `memory.section_texts("vocabulary")` (+ optional seed), capped at 100.
- [ ] **Step 2 — run to see failure.**
- [ ] **Step 3 — implement:**
  - **Shared `stop_pump` helper (fixes finding 3 — never `join()` on a tokio worker):** the pump thread's `poll()` runs a long **blocking** Metal decode; a bare `.join()` inside a `#[uniffi::export(async_runtime="tokio")] async fn` would **block a multi-thread tokio worker** for the duration of an in-flight decode (bounded but real under sim-CPU/thermal). So `stop_pump` sets the `stop` flag + notifies the Condvar, then performs the `join()` via **`tokio::task::spawn_blocking(move || handle.join()).await`** — moving the wait onto tokio's blocking pool, off the async workers. Chosen over signal-and-detach because it gives a **deterministic "pump fully stopped before we touch `SttStream::end()`/`process()`/`delete_session`"** guarantee (no detached thread can call `append_transcript` mid-swap); the cost is one blocking-pool task, which is exactly what that pool is for. **BOTH `finish()` and `cancel()` are async and use `stop_pump().await`** — see the shared-pattern note below.
  - **`finish()` flush:** after acquiring the extractor mutex `_tick_guard`, `stop_pump().await`. Then, if `stt.is_some() && flush_on_finish`, call `stt.end()`; for each finalized segment, write it via a **direct `Store::append_transcript`** (a scoped lock) so the flushed text is part of the transcript `process()` reads. **Do NOT** route the flush through the async tick (we hold the extractor mutex for D3b — a tick would deadlock on it); the flushed text is caught by `process()`'s authoritative extraction anyway, and optionally emit a final `TranscriptCommitted` for the UI. Ordering: stop pump → flush → `end_and_record_session` → `process()`. Existing degrade paths (empty transcript, offline) unchanged.
  - **`cancel()` — one API, both leaks (finding 2 + issue #3), a close cousin of `finish()`:** `#[uniffi::export(async_runtime="tokio")] pub async fn cancel(self: Arc<Self>)`. It is **NOT a sync export** — `discardWalk()` runs on the main actor, uniffi sync exports run on the caller's thread, and the pump `join` can land mid-Metal-decode; a sync `cancel()` would block the UI thread. So `cancel()` mirrors `finish()`'s shape: acquire the extractor mutex `_tick_guard` (excludes ticks), `stop_pump().await` (spawn_blocking join — never blocks a worker OR the UI), then tombstone the store. **Swift calls it from inside a `Task` in `discardWalk()`** (Task 7) so the discard is fire-and-forget off the main thread. It: (1) `stop_pump().await`. (2) **Tombstones the session with the EXISTING `Store::delete_session(id)`, period** — it already cascade-tombstones the session + its items + its artifacts in one transaction (verified by `delete_session_is_a_tombstone`), which is the *complete* fix for issue #3 (a bare "Abandoned" status would tombstone the session row but NOT cascade to items — reintroducing the zombie-items half of the leak). `delete_session` also makes both post-cancel races safe for free: a stale tick that slips through fails cleanly at `get_session`'s `deleted_at IS NULL` filter, and everything serializes on the store mutex. Do **not** add an `abandon_session`/`Abandoned` status. (3) Drop the `SttStream`/listener so the session's `Arc` cycle (the pump held its own `Arc<WalkSession>`) is broken and the object can free. Idempotent: guard on "pump already stopped / session already deleted." **Wire into Swift `AppModel.discardWalk()`** (Task 7) — today it only cancels Swift tasks and never tells Rust, which is exactly why the thread + the SQLite rows leak.
  - **Shared-pattern + Arc-cycle note:** `finish()` and `cancel()` are the SAME shape — async, tick-guard-holding, `stop_pump().await` — differing only in the tail (`process()` + document vs `delete_session`). The pump thread owns an `Arc<WalkSession>` (to call `append_transcript`), so a session is never freed until the pump exits; `finish()` and `cancel()` are therefore the ONLY two exits and both must stop the pump. A walk that is neither finished nor cancelled (app killed mid-walk) leaks only until process death — acceptable, but the `cancel()` wiring in `discardWalk` closes the common case.
  - **Bias terms:** add `fn collect_bias_terms(memory: &Memory, template: Option<&str>) -> Vec<String>` (read `section_texts("vocabulary")`, map to owned, optional per-template seed, truncate to `SttConfig::default().max_bias_terms`). `begin_walk` (Task 5) uses it when building the real stream.
  - `flush_on_finish` is read from the field set in Task 1 (from `EngineConfig`).
- [ ] **Step 4 — verify:** `cargo test -p ffi session` green (flush both directions; `cancel()` stops the pump + `delete_session` cascade-tombstones the session/items/artifacts; existing finish degrade tests unaffected). No `murmur-core` change needed — `delete_session` already exists and is tested.
- [ ] **Step 5 — commit:** `feat(ffi): finish() flush + cancel() (stop pump, delete_session — fixes thread leak + issue #3); bias terms`

---

### Task 5: whisper backend construction in `begin_walk` (cfg-gated) + fallible model load + env-gated real-model test

**Files:** modify `crates/ffi/src/session.rs`, `crates/ffi/src/engine.rs`.

**Goal:** Under the `whisper` feature, `begin_walk` builds a real `SttStream::with_model` from `EngineConfig.stt_model_path` + the bias vocabulary and starts the pump; a bad model path degrades (no panic across FFI, riding the fallible-constructor carry-forward). With the feature off, everything still compiles and the text path is unchanged.

- [ ] **Step 1 — failing / gated tests:**
  - Hermetic (feature off): `begin_walk_without_model_path_is_text_only` — `stt_model_path: None` → `WalkSession.stt` is `None`, `push_audio` is a no-op, the text `append_transcript` path still works (reuse an existing begin_walk test shape).
  - Env+feature gated real-model smoke (`#[ignore]`, like `stt`'s `real_model_decodes_silence` and `anthropic_smoke`): with `--features whisper` + `MURMUR_WHISPER_MODEL` set, `begin_walk` with that path builds a session whose `stt` is `Some`, `push_audio(silence)` + a pump pass returns without error. CI never runs it.
- [ ] **Step 2 — run to see failure.**
- [ ] **Step 3 — implement:**
  - **Thread the two config values onto `MurmurEngine` WITHOUT breaking `with_providers` (finding 4).** `MurmurEngine` gains `stt_model_path: Option<String>` and `stt_flush_on_finish: bool` fields. `new(config)` sets them from `config`. But **`with_providers` (`engine.rs:129`) takes no `EngineConfig`** and is called by every existing mock-provider test — changing its signature would break all of them. So `with_providers` keeps its current parameter list and **initializes the two fields to defaults**: `stt_model_path: None`, `stt_flush_on_finish: true`. Rationale: mock-provider tests exercise the **text path or an injected `ScriptedDecoder` `SttStream`** (via the Task 2 test-only audio-session constructor), never a real model file — so `None` is correct for them and no call site changes. Whisper-path pump tests get their `SttStream` from the test constructor, not from `with_providers`. (If a test ever needs a non-default flush toggle, add a narrow `#[doc(hidden)] with_providers_stt(..)` variant rather than widening the common one — do NOT thread config into `with_providers`.)
  - Build the stream:
    ```rust
    let bias = collect_bias_terms(&self.memory.lock().unwrap(), Some(&template));
    let stt = self.build_stt_stream(&bias);   // Option<Arc<SttStream>>
    ```
    where `build_stt_stream`:
    - `#[cfg(feature = "whisper")]`: if `Some(path)`, `SttStream::with_model(Path::new(path), SttConfig::default(), &bias)` → `Ok` ⇒ `Some(Arc::new(stream))`, `Err(e)` ⇒ **degrade**: `log`/eprintln the load failure (never the key), return `None`. **If the parallel fallible-`begin_walk` lane has landed**, propagate the error as `Err` from `begin_walk` instead of degrading — branch on the constructor's actual signature at implementation time (carry-forward). Either way, **no `expect()`/panic** on a bad model path.
    - `#[cfg(not(feature = "whisper"))]`: always `None` (ignore the path — the text path handles the walk).
  - After constructing `WalkSession`, call `start_pump` (no-op if `stt` is `None`).
  - **Key hygiene:** the model-load log line must never include `api_key` (it doesn't have access to it, but assert in review — Plan 07 R6/redaction posture).
- [ ] **Step 4 — verify:** `cargo test -p ffi` (feature off) green; `cargo test -p ffi --features whisper` compiles and passes (smoke `#[ignore]`d); manual on device/mac: `MURMUR_WHISPER_MODEL=… cargo test -p ffi --features whisper -- --ignored`.
- [ ] **Step 5 — commit:** `feat(ffi): begin_walk builds whisper SttStream from model path (cfg-gated, fallible); real-model smoke gate`

---

## Part B — The Swift shell (sim/device-testable)

### Task 6: `AudioCaptureSource` — AVAudioEngine tap → 16 kHz mono f32 → `push_audio`

**Files:** create `apps/ios/Sources/Engine/AudioCaptureSource.swift`; modify `apps/ios/Sources/Engine/TranscriptSource.swift` (doc note).

**Goal:** Capture mic audio in Swift, convert to 16 kHz mono f32, and push PCM frames across FFI off the render thread (D1). This replaces `SFSpeechRecognizer` as the `live=1` STT source — the engine now does STT in Rust.

- [ ] **Step 1 — implement `AudioCaptureSource`** (an `@MainActor` class; not a `TranscriptSource` — it produces **PCM**, not text):
  - `AVAudioSession`: `.record`, `.measurement`, `.duckOthers`, activate (mirror `SpeechSource`).
  - `AVAudioEngine.inputNode.installTap(onBus:bufferSize:format:)` at the hardware format.
  - An `AVAudioConverter` from the hardware format → `AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16000, channels: 1, interleaved: false)`. In the tap callback (render thread): convert the buffer, copy the 16 kHz f32 samples into a Swift array, and hand off to a **serial background** `DispatchQueue`/`Task` that calls the injected `pushSamples: ([Float]) -> Void` closure (which the adapter wires to `session.pushAudio`). The tap callback itself does the minimum and never blocks (D1 cadence/backpressure).
  - `start()` / `pause()` (`engine.pause()`) / `resume()` (`engine.start()`) / `stop()` (remove tap, stop engine, deactivate session).
  - `requestPermissions()` (mic only — no Speech authorization needed now; STT is on-device whisper).
- [ ] **Step 2 — doc note** in `TranscriptSource.swift`: STT for `live=1` is now Rust-side whisper via `AudioCaptureSource` + `push_audio`; `SpeechSource` (SFSpeechRecognizer) is retained but no longer the live default (or removed if the team prefers — coordinate; keep for the milestone as a fallback, D10-style "delete nothing").
- [ ] **Step 3 — verify (build only here):** `cd apps/ios && xcodegen generate && xcodebuild … build` compiles with the new file (adapter wiring lands in Task 7). No runtime yet.
- [ ] **Step 4 — commit:** `feat(ios): AudioCaptureSource — mic → 16kHz mono f32 PCM, pushed off the render thread`

---

### Task 7: `MurmurEngine` adapter — `pushAudio` passthrough, transcript events → UI, model path

**Files:** modify `apps/ios/Sources/Engine/MurmurEngine.swift`, `apps/ios/Sources/App/AppModel.swift`.

**Goal:** Wire the audio source and Rust transcript events through the existing `WalkEngine` adapter so `live=1` runs a real whisper walk; the UI transcript now comes from Rust `TranscriptCommitted` events.

- [ ] **Step 1 — adapter surface:**
  - Add `func pushAudio(_ samples: [Float])` to the `WalkEngine` protocol (`WalkEngine.swift`) — the parallel to `append(transcript:)` for the audio path. `DemoWalkEngine` implements it as a no-op (scripted text demo needs no audio).
  - Add `func cancel() async` to the `WalkEngine` protocol (the discard path, Task 4 finding 2 — **async**, since the Rust `cancel()` is an async export that `spawn_blocking`-joins the pump). `MurmurEngine.cancel` → `await session?.cancel()` then nils `session`/`continuation` (like `finish()` does). `DemoWalkEngine.cancel` is a no-op.
  - `MurmurEngine.pushAudio` → `session?.pushAudio(samples: samples)`.
  - `BoardListener.onEvent`: handle the new cases. `boardUpdated` unchanged. Add `case .transcriptCommitted(let text)` and `case .transcriptPreview(let text)` → hop to `@MainActor` and yield new `WalkEvent` cases up to `AppModel`.
  - Extend the Swift-side `WalkEvent` enum (`WalkEngine.swift`) with `case transcriptCommitted(String)` and `case transcriptPreview(String)` so the app-facing stream carries them.
- [ ] **Step 2 — `AppModel` wiring:**
  - `startWalk()`: when `!scripted` (i.e. `live=1`), use `AudioCaptureSource` instead of `SpeechSource`; its `pushSamples` closure calls `engine.pushAudio(_:)`. When `scripted`, keep `ScriptedSource` (text) → `engine.append(transcript:)` unchanged (demo path).
  - The event loop gains: `case .transcriptCommitted(let text): self.transcript += text` (the transcript now originates from Rust for the audio path) and `case .transcriptPreview(let text): self.previewTail = text` (a new optional `@Published`-style property; render greyed — **nice-to-have**, D4). Do **not** double-count: in the audio path, the Swift pump no longer appends transcript from `src.chunks` (there are no text chunks — `AudioCaptureSource` yields PCM, not text). Keep the `src.chunks` text-append **only** for the scripted path.
  - **Guard the two paths cleanly:** scripted → text via `append`; live → PCM via `pushAudio` + transcript via events. No path feeds both.
  - **Wire `discardWalk()` to `cancel()` (finding 2):** `AppModel.discardWalk()` today only cancels the Swift `source`/tasks and resets state — it never tells Rust, leaking the pump thread AND zombie `Recording`/item/artifact rows (issue #3). Add `source?.stop()` (for `AudioCaptureSource`) + a fire-and-forget `Task { await engine.cancel() }` to `discardWalk()` so the Rust session is stopped + `delete_session`-tombstoned **off the main thread** (the async `cancel` `spawn_blocking`-joins the pump, so the UI never blocks). Reset the Swift state synchronously as today; the Rust teardown rides the detached `Task`.
- [ ] **Step 3 — model path + config** (`GalleryApp.swift`): in `resolveEngine`, set `stt_model_path` = `Bundle.main.path(forResource: "ggml-base.en-q5_1", ofType: "bin")` and `stt_flush_on_finish: true` on `EngineConfig`. If the model resource is missing, `stt_model_path: nil` → the walk degrades to text-only (no crash). Update the `EngineConfig(...)` literal for the new fields.
- [ ] **Step 4 — verify (build):** `xcodebuild … build` compiles with the new events, adapter methods, and config fields.
- [ ] **Step 5 — commit:** `feat(ios): MurmurEngine pushAudio + transcript events → UI; live=1 uses whisper audio; model path`

---

### Task 8: Packaging — `whisper` feature in `build-ffi.sh`, bundle the model, xcframework

**Files:** modify `apps/ios/build-ffi.sh`, `apps/ios/project.yml`; provision `apps/ios/Packages/MurmurCoreFFI/Resources/ggml-base.en-q5_1.bin`.

**Goal:** Build the FFI static libs **with `--features whisper`** (pulls whisper-rs + vendored whisper.cpp + Metal), regenerate bindings, bundle the model file, and link it — while `cargo test --workspace` stays feature-off/hermetic.

- [ ] **Step 1 — `build-ffi.sh`:** add `--features whisper` to both `cargo build -p ffi --release --target aarch64-apple-ios-sim` and `…-ios` invocations. whisper.cpp needs cmake/clang (already in the dev shell via Plan 06 Task 1) — the script already sets `SDKROOT`/system-Xcode CC for the iOS cross-link; confirm the metal shaders compile for the iOS SDK (whisper-rs-sys vendored build). If the metal build needs an SDK flag, add it here (document). The bindgen step is unaffected (it reads the built library).
- [ ] **Step 2 — bundle the model:** document in `build-ffi.sh` / `MurmurCoreFFI/README.md` how to fetch `ggml-base.en-q5_1.bin` (~60 MB, MIT, `huggingface.co/ggerganov/whisper.cpp`) into `Resources/` (gitignored — large binary, like the xcframework). `project.yml`: add the `Resources/` model file to the app target's `resources:` (or the `MurmurCoreFFI` package resources) so `Bundle.main.path(forResource:)` resolves at runtime. Keep `CODE_SIGNING_ALLOWED: NO` for sim.
- [ ] **Step 3 — verify:** run `apps/ios/build-ffi.sh` (produces whisper-enabled libs + bindings + xcframework); `cd apps/ios && xcodegen generate && xcodebuild … build` links; **crucially** confirm `nix develop -c cargo test --workspace` (repo root, feature OFF) still green — the app build's `--features whisper` must not leak into the workspace test invocation.
- [ ] **Step 4 — commit:** `build(ios): FFI built with whisper feature; bundle base.en model; xcframework wired`

---

### Task 9: `WavFileAudioSource` scripted-audio path + sim/device verification

**Files:** create `apps/ios/Sources/Engine/WavFileAudioSource.swift`; provision a small 16 kHz fixture WAV; verify.

**Goal:** A mic-free way to drive the real whisper path (D7) for testing on sim/device, plus the milestone verification. The text demo path stays intact.

- [ ] **Step 1 — `WavFileAudioSource`:** reads a bundled 16 kHz mono WAV (a short scripted jobsite utterance), and on `start()` pushes its f32 PCM in ~100–250 ms chunks via the same `pushSamples` closure `AudioCaptureSource` uses — so `push_audio` sees realistic frames without a mic. A launch arg (e.g. `wavwalk=1`) selects it over the live mic.
- [ ] **Step 2 — demo path preserved:** confirm `demo=1 autoflow=1` still plays the **text** scripted walk (no whisper, no audio) and the board fills — the milestone screenshot/CI path is unchanged (Plan 07 D10). `make sim-screenshot`-style capture.
- [ ] **Step 3 — sim check (whisper on sim):** launch with `wavwalk=1` + a real key + `ANTHROPIC_BASE_URL`; the bundled WAV drives `push_audio` → whisper decodes (CPU fallback on sim if Metal absent, D7) → finalized text appends → board ticks → DONE builds a document. If Metal init hard-fails on sim, record it and gate on device (Step 4). Capture a `record_sim_video`/`gif_creator` artifact.
- [ ] **Step 4 — device check (the real milestone gate):** on a physical iPhone (ties to ROADMAP's pending T5 device spike), `live=1` real-mic walk: speak a scripted jobsite line → board fills from live whisper passes → DONE builds a real document with at least one honest gap → PDF → send. This is the "real voice walk" milestone; if a device isn't available in the sandbox, record it as an **owed manual verification** with the exact steps.
- [ ] **Step 5 — commit:** `feat(ios): WavFileAudioSource scripted-audio path; sim/device whisper-walk verification`

---

## Part C — Construction-site noise robustness (dam addendum; sequence AFTER core wiring — the milestone gate at Task 9.4 does not move)

> Whisper's failure mode in machinery-only audio is **fluent hallucination** — it invents plausible text from a jackhammer/generator drone. That directly violates product rule **R3 (under-extract, never invent)**. These three tasks harden the noisy-jobsite case. They are separable and land after Tasks 1–9; none is on the critical path to the "real voice walk" milestone.

### Task 10: Swift capture — Apple voice processing / Voice Isolation as an A/B config knob

**Files:** modify `apps/ios/Sources/Engine/AudioCaptureSource.swift`, `GalleryApp.swift`.

**Goal:** Make Apple's on-device noise/echo suppression available on the capture path, as a **toggle**, because aggressive suppression can *hurt* whisper (spectral artifacts) as easily as help — the choice is deferred to the Task 12 noise eval, not decided here.

- [ ] **Step 1 — implement the knob:** on the `AVAudioEngine` input node, gate `try inputNode.setVoiceProcessingEnabled(true)` behind a config flag (e.g. `voiceProcessing: Bool` on the capture source, sourced from a launch arg `voiceproc=1` and/or `EngineConfig`). Document the alternative system route (AVAudioSession input mode / the OS mic-mode "Voice Isolation" the user picks in Control Center) in a comment — that route is user-controlled, not app-set, so the app knob is `setVoiceProcessingEnabled`. Note the format caveat: enabling voice processing can change the node's output format — re-derive the `AVAudioConverter` input format after toggling.
- [ ] **Step 2 — acceptance = BOTH paths runnable, decision deferred:** verify the walk runs with the knob **on** and **off** (no crash, PCM still flows at 16 kHz mono f32). Do **not** pick a default here — Task 12's SNR curves decide it. Record both as runnable in the commit message.
- [ ] **Step 3 — verify (build + a smoke walk each way):** `xcodebuild … build`; a `wavwalk=1` run with `voiceproc=1` and without, both reach the board.
- [ ] **Step 4 — commit:** `feat(ios): Voice Isolation / voice-processing capture knob (A/B; default deferred to noise eval)`

---

### Task 11: Rust VAD / no-speech gating — stop machinery-only hallucination (R3)

**Files:** modify `crates/stt/src/` (a new `vad.rs` energy helper + per-window gate in `lib.rs`; extend `decoder.rs`'s `RawSegment` + `finalize.rs`); hermetic tests throughout.

**Goal:** Gate noise-only audio out of the committed transcript by **two** independent signals, both hermetic-testable with `ScriptedDecoder` + synthetic PCM (no model, no feature):
- **(a) Per-window energy/VAD gate — DEFAULT placement: skip the *decode*, not the *samples*.** After `Chunker::take_ready_window()` yields a window, compute a cheap RMS-energy (optionally zero-crossing) on that window's samples; if it's below the speech threshold, **skip the `decoder.decode()` call** for that window (emit nothing) but leave the `Chunker`/`next_start` cursor advancing exactly as normal — **all samples still enter the Chunker**. This preserves Plan 06's "absolute ms from stream start" timestamp contract, because window start offsets are still derived from the real sample count. Pure Rust, no deps. Testable: a synthetic low-energy window → no decode/no output; a speech-energy window → decodes normally.
  - **Perf fallback (NOT the default — flagged):** dropping samples *before* the Chunker (never buffering silent audio) saves the buffer copy, but it **breaks the timestamp contract** — the Chunker's sample-count-derived clock would drift from wall-clock time whenever samples are elided, so `start_ms/end_ms` no longer map to real audio position. Only consider this if profiling shows the buffer copy matters, and only with an explicit wall-clock re-anchoring scheme. Default is per-window decode-skip above.
- **(b) `no_speech_prob` post-check (in the `Finalizer`):** whisper emits a per-segment no-speech probability; segments above a threshold are discarded before finalize. **This requires a Plan-06 crate change:** `RawSegment` (`decoder.rs`) currently carries only `start_cs/end_cs/text` — add `no_speech_prob: f32` (default `0.0`; `ScriptedDecoder` sets it per script entry; `WhisperDecoder` populates it from `state`). The `Finalizer::ingest` drops words from segments whose `no_speech_prob` exceeds the config threshold. Both thresholds live in `SttConfig` (with measured defaults from Task 12; conservative defaults until then).
  - **Awareness note — the new `RawSegment` field is a compile-catch that breaks 4 existing struct-literal sites** (all mechanical: add `no_speech_prob: 0.0`): `decoder.rs` tests ×2 (`scripted_decoder_returns_scripts_in_order_and_captures_prompts`'s two `RawSegment { … }` literals), `lib.rs`'s test `seg()` helper, and `finalize.rs`'s test `seg()` helper. (The `Default`/builder alternative avoids churn but a field add is the honest signal here.)

- [ ] **Step 1 — failing tests** (hermetic): (i) `low_energy_window_is_not_decoded` — a synthetic low-RMS window yields no decode/no output while all samples still advance the Chunker cursor (next window's `start_ms` is unaffected — timestamp contract holds); a speech-energy window decodes; (ii) `high_no_speech_prob_segment_is_not_finalized` — a `ScriptedDecoder` segment with `no_speech_prob: 0.95` produces **no** committed words, while a `0.05` segment does; (iii) append-only/dedup contracts (Plan 06) still hold with the new field.
- [ ] **Step 2 — run to see failure.**
- [ ] **Step 3 — implement:** add `RawSegment.no_speech_prob` (additive, defaulted — fix the 4 struct-literal sites noted above); `WhisperDecoder` populates it (`#[cfg(feature="whisper")]`); `vad.rs` RMS helper + the **per-window gate in `SttStream::poll` after `take_ready_window()`** (decode-skip, samples still buffered — default placement); `Finalizer` no-speech post-check; `SttConfig` thresholds. **Keep hermetic:** the VAD is pure; the `no_speech_prob` field defaults so all existing Plan-06 tests and `cargo test --workspace` stay green with the feature off.
- [ ] **Step 4 — verify:** `cargo test -p stt` (feature off) green incl. the new gates; `cargo test -p stt --features whisper` compiles; `cargo test --workspace` still hermetic-green.
- [ ] **Step 5 — commit:** `feat(stt): energy/VAD pre-gate + no_speech_prob post-check — no machinery hallucination (R3)`

---

### Task 12: Noise eval — SNR sweep → WER curves → base.en vs small.en decision

**Files:** extend `spikes/stt-whisper/` harness (or a note + fixtures in `crates/evals/`); a `docs/` results append.

**Goal:** Decide, with data, (a) whether base.en survives jobsite noise or small.en should be promoted, and (b) the Task 10 voice-processing default and Task 11 thresholds. **Not hermetic / not CI** — this is a manual measurement harness (needs models + audio), like the original spike.

- [ ] **Step 1 — noise corpus:** mix real construction noise (jackhammer, circular saw, generator, wind — public corpora, e.g. ESC-50/FSD50K/freesound; cite licenses) into the spike's clean test speech at several SNRs (e.g. +20, +10, +5, 0 dB).
- [ ] **Step 2 — run the sweep:** for base.en and small.en, at each SNR, with voice-processing-simulated vs raw (or note it's device-only), produce **WER curves** and a hallucination rate (invented-token count on noise-only clips — the R3 metric).
- [ ] **Step 3 — decide + document:** append results to `spikes/stt-whisper/RESULTS.md` (or a new `docs/research/` note): the base-vs-small call (RTF headroom means small.en is ~free if WER justifies it — D5), the voice-processing default (Task 10), and the VAD/no-speech thresholds (Task 11). Feed the chosen model back to D5/Task 8's bundled file if it changes.
- [ ] **Step 4 — commit:** `docs(stt): construction-noise SNR sweep — WER curves, model + threshold decisions`

---

## Part D — Docs & final review

### Task 13: Docs + independent whole-artifact review

**Files:** modify `README.md`; review across the whole diff.

- [ ] **Step 1 — README:** plan-series line `… 07 FFI bridge, 08 STT stage-2 (mic → whisper → append)`.
- [ ] **Step 2 — full verification:** `nix develop -c cargo test --workspace` (feature OFF, hermetic) + `cargo clippy --workspace --all-targets` (zero warnings) + `cargo build -p ffi --features whisper` + `cargo clippy -p ffi --features whisper --all-targets` + `cargo test -p stt` (incl. the new VAD/no-speech gates) + the iOS build + the Task 9 sim/device flow.
- [ ] **Step 3 — independent whole-artifact review** (CANON: "caught a real cross-module issue in every plan — never skip it"; this is a separate agent from the builder, per MEMORY "independent final review has caught a real issue 9/9 times"). Read the diff `stt → crates/ffi → apps/ios` as one artifact and specifically re-check:
  - **Threading/locks:** `SttStream::poll()` releases its engine/input locks before `append_transcript` runs (no lock inversion, D2); the pump is stopped via `spawn_blocking(join)` in `finish()` (Task 4) so **no tokio worker is blocked** on an in-flight decode; a pump-triggered tick still queues behind `finish()`'s held extractor mutex (D3b intact).
  - **Discard/cancel path (finding 2 + issue #3):** `AppModel.discardWalk()` calls `await engine.cancel()` from a detached `Task` (never blocks the main actor); the async `WalkSession::cancel()` holds the tick-guard, `spawn_blocking`-joins the pump (no leak, no worker/UI block) AND calls `Store::delete_session` (cascade-tombstones session + items + artifacts — no zombie rows of any kind, issue #3 fully closed); `cancel()` is idempotent and safe after `finish()`; the pump's `Arc<WalkSession>` cycle is broken so the object frees. Confirm a discarded walk leaves neither a live thread nor any open/zombie rows, and that a post-cancel stale tick fails cleanly at `get_session`'s `deleted_at IS NULL` filter.
  - **Hermetic CI:** `cargo test --workspace` compiles and passes with `whisper` OFF; no test references a model file; `with_model` and `WhisperDecoder`'s `no_speech_prob` population are fully `#[cfg(feature="whisper")]`; `RawSegment.no_speech_prob` defaults so Plan-06 tests are unbroken.
  - **No panic across FFI:** a bad/missing model path degrades (or returns `Err` via the fallible constructor) — never `expect()`; a decode error in the pump is logged and skipped, not fatal.
  - **Finalized-only extraction / R3:** only finalized text reaches `append_transcript`; `preview_tail` is never persisted/extracted; VAD/no-speech gating drops machinery-only audio before it can hallucinate into the committed stream; the scripted text path and the audio path never double-feed the transcript.
  - **Key hygiene:** the model-load log line and any new `Debug` never print `api_key`.
  - **Additivity:** the text `append_transcript` path is byte-for-byte unchanged; a text-only (`stt: None`) session behaves exactly as Plan 07 shipped.
- [ ] **Step 4 — commit:** `docs: plan 08 done — STT stage-2 mic→whisper→append wired`

---

## Non-goals (explicit)

1. **Model download / on-demand resources / model-selection UI** — bundle base.en for the milestone (D5); fetching/storage/base-vs-small selection is a shell concern (Plan 06 Deferred 5).
2. **Word-precise timestamps** — v1 segment-coarse time (Plan 06 Deferred 4); the coarse-seam fallback's accuracy fix (ROADMAP §2) is a follow-up.
3. **Trie / logit-bias hotword decoder** — v1 is `initial_prompt` injection (Plan 06 Deferred 3).
4. **Battery/thermal auto-tuning, adaptive chunk/model** — Plan 06 Deferred 7; revisit with device numbers (the T5 spike). *(Note: energy/VAD + no-speech gating is now IN scope — Part C Task 11 — because machinery hallucination is an R3 correctness issue, not a battery optimization.)*
5. **Diarization** — Plan 06 Deferred 9.
6. **Android backend** — Plan 06 Deferred 8.
7. **Generative layout-ops protocol** — Plan 07 Deferred 2 (`boardUpdated` snapshots stay coarse).
8. **Voice gap-fill / correction during review, price-book autofill** — Plan 07 Deferred 3/4.
9. **Rolling prior-transcript context in the whisper prompt** — Plan 06 Deferred 6 (risks diluting the ≤100 bias terms).

## Carry-forward constraints surfaced for later plans

- **Fallible `MurmurEngine::new`/`begin_walk`** (parallel lane) — this plan's model-load degrade should become an `Err` return once that lands (D5, Task 5 Step 3).
- **iPhone T5 device spike** (the one unretired STT GO condition) — Task 9 Step 4's device walk is the natural place to retire it; RTF<1.0 + no thermal kill over ~10 min locked (Plan 06 Deferred 2).
- **On-sim Metal availability** — if whisper.cpp Metal can't init on the simulator, a CPU-force config knob may be needed for sim builds (D7); device is the real target.
- **Model-download follow-up** — bundling ~60 MB is a milestone expedient; ODR/download manager is the scalable path (D5). If Task 12's noise eval promotes small.en (~182 MB), the bundle-vs-download trade tightens — a download path may become required.
- **STT DONE flush-vs-speed canon** — default `flush` behind `stt_flush_on_finish` (D6); flip the default if canon lands "speed."

## Self-Review Notes

- **Riskiest assumption (named):** that whisper.cpp's **Metal** build links and runs correctly for the iOS SDK via `build-ffi.sh`'s system-Xcode cross-link path (Task 8), and that Metal degrades to CPU (not a hard failure) on the simulator (D7/Task 9). Mitigation: the milestone gate is the **device** walk (Task 9.4), the sim path has a documented CPU fallback, and the entire Rust pump is proven hermetically with `ScriptedDecoder` (Tasks 2–4) independent of whisper.
- **Additivity discharged:** `push_audio` + the pump are new; `append_transcript`, the `LiveExtractor` actor, `boardUpdated`, and `finish()`'s two-phase process are reused verbatim (D2/D3). A `stt: None` session is exactly Plan 07's text path — the review checks this (Task 13.3).
- **Two lifecycle exits, one shape, both stop the pump (finding 2 + 3):** the pump thread owns an `Arc<WalkSession>`, so `finish()` and `cancel()` are the only paths that can free a session. Both are **async, tick-guard-holding, `stop_pump().await` (spawn_blocking join)** — differing only in the tail: `finish()` runs `process()` + returns the document; `cancel()` calls the existing `Store::delete_session` (cascade-tombstones session + items + artifacts — the *complete* issue #3 fix; a bare "Abandoned" status would have left zombie items). `cancel()` is async precisely so `discardWalk()`'s detached `Task` never blocks the main actor on an in-flight decode. A never-finished-never-cancelled walk (app killed mid-walk) leaks only to process death.
- **Hermetic-CI invariant:** `stt` is an always-on pure dep; only `WhisperDecoder` construction is `#[cfg(feature="whisper")]`; `cargo test --workspace` never sees the feature, a model, or a native STT toolchain. The app build alone adds `--features whisper` (Task 8). This is Plan 06's discipline extended one crate up — the single most important thing this plan must not break.
- **Judgment calls for reviewers:** (a) **Rust-side pump on a dedicated `std::thread`** over Swift-side polling or `tokio::spawn` — Plan 07 D8 committed to a bridge-side pump; a dedicated thread keeps the blocking Metal decode off the tokio workers and off the audio render thread, and reuses the existing `append_transcript` seam. Its two exits share one shape — both async, both join via `spawn_blocking`, so neither blocks a tokio worker nor (for `cancel()` from `discardWalk`'s `Task`) the main actor (Task 4, findings 2/3). (b) **bundle base.en** over first-run download — offline-first (spec §1); download is Deferred. (c) **finish flushes by default** behind a toggle — capture-never-lost over a sub-second speed gain; flagged for canon (D6). (d) **transcript surfaced via two new `WalkEvent` cases** over reusing `boardUpdated` — extraction stays finalized-only; the UI gets committed + optional greyed preview (D4). (e) **feature-independent FFI surface** (`push_audio` always exists; whisper gates only `with_model`) so the Swift binary and the hermetic test build share one method set (D3).
- **Spec coverage:** Rev 2 §2 on-device live STT ✓ (the whole plan); §1 offline/bulletproof capture ✓ (bundled model, flush-on-finish, decode-error-non-fatal pump); §vocabulary point 3 biasing ✓ (D8, memory `vocabulary` → `build_bias_prompt`); §6 <8 s ✓ (finish flush is one sub-second decode before the existing budgeted process). Live-is-provisional / `process()` authoritative ✓ (finalized text feeds the live board; the flushed full transcript feeds authoritative processing, unchanged).
- **R3 / noise robustness (Part C):** whisper's fluent hallucination on machinery-only audio is a correctness bug (invents text = violates under-extract-never-invent), so the energy/VAD pre-gate + `no_speech_prob` post-check (Task 11) are in scope, hermetic-testable, and additive to Plan 06 (`RawSegment.no_speech_prob` defaults to `0.0`). The Apple voice-processing knob (Task 10) is A/B-only — default deferred to the SNR eval (Task 12), because suppression can hurt whisper as easily as help.
- **Test-count checkpoint (expectations, not gates):** T1 +2 (config), T2 +1 (pump e2e), T3 +1 (transcript events), T4 +3 (flush both ways + cancel/tombstone + bias terms), T5 +1 hermetic +1 `#[ignore]` smoke, Task 8 e2e +1 (`audio_pump_e2e.rs`), T11 +3 (VAD gate + no-speech + append-only-holds) ≈ **13 new**, of which 12 run in default (feature-off) CI (the `#[ignore]` smoke and the manual noise eval excluded).
