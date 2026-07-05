# Murmur Rust Core — Plan 06 SPIKE: On-Device STT Benchmark (whisper.cpp Rust-side)

> **For agentic workers:** This is a **measurement-first spike**, not a production plan. The code it produces is **disposable and quarantined** (`spikes/stt-whisper/`, NOT a workspace member). The deliverable is a **decision document** (`spikes/stt-whisper/RESULTS.md`) whose rows the tasks fill in. Do NOT wire any of this into `crates/`. When the numbers are in, Plan 06 (real) commits an architecture. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Confirm or kill dam's stated preference — *"go straight to whisper.cpp Rust-side only"* — with measurements, before Plan 06 locks the STT architecture. The [frontier survey](../../research/2026-07-04-on-device-stt-frontier.md) established that essentially **zero iPhone-specific RTF/battery/thermal data and zero construction-site WER data exist in public sources** for any engine; and that the vocabulary→STT biasing loop (≤100 curated terms) — the product's stated differentiator — is *not* deliverable on the platform-side path (Apple `SpeechAnalyzer` has no contextual-biasing surface today). This spike measures what the literature can't tell us, on hardware we control.

**The fork being decided:** Option B (whisper.cpp Rust-side via `whisper-rs`, cross-platform, biasing-capable) vs. the staged-hybrid fallback Option C (Apple `SpeechAnalyzer` for v1, Rust path deferred). See survey §"Recommendation." This spike produces the evidence that picks between them.

**What this spike does NOT do:** it does not implement the trie/logit-bias hotword decoder (deep follow-on, out of scope — see Deferred), does not touch `crates/murmur-core` or the live-extraction cursor, does not ship an app. The live-extraction cursor's **append-only / finalized-only contract** ([Plan 05](2026-07-03-rust-core-05-live-extraction.md), design note 5 and Deferred 2) is a **fixed input constraint** here: kill-question 2 asks whether chunked whisper can *feed* that contract, it does not renegotiate it.

---

## The five kill-questions (the spike is structured around these)

1. **Feasibility.** Does `whisper-rs` (binding to whisper.cpp) build and run in our nix-based Rust workspace on macOS Apple Silicon (dam's dev machine), with the Metal backend? Which model sizes (`tiny` / `base` / `small` / `large-v3-turbo` / `distil-large-v3`) at what RTF and peak memory on the Mac — the Mac being the **optimistic proxy** for the iPhone thermal envelope.
2. **Streaming semantics.** With chunked pseudo-streaming, what do segment boundaries and finality actually look like? Measure re-transcription overlap between consecutive chunks. Can we derive an **append-only finalized stream** (the Plan 05 cursor contract) and at what **finalize-latency** cost?
3. **Biasing surface.** What does `initial_prompt` injection of our ≤100 vocabulary terms actually do to recognition of trade jargon? Measure **term-recall WER with vs. without** the prompt. (Logit-bias / hotword-trie is the deeper, higher-payoff technique — explicitly out of spike scope; but the spike's biasing result decides whether that follow-on is *required*.)
4. **Accuracy proxy.** WER on a small noisy-speech test set. "Good enough for a spike" = enough signal to compare model classes and see biasing lift — **not** production accuracy.
5. **iPhone reality check.** What can run where? The iOS **simulator runs CPU-only whisper** (fine for *functional* checks, useless for battery/thermal — no Metal/ANE, x86-via-Rosetta or arm64 CPU only). Real battery/thermal needs an **actual iPhone**. So: deliver the **Mac benchmark CLI first (runnable today)**; specify the **smallest possible iOS device harness** as an optional second tier dam can run on hardware.

---

## Architecture of the spike

### Quarantine (non-negotiable)

Spike code lives in **`spikes/stt-whisper/`** in the sitewalk repo (`github.com/damsac/sitewalk`, local `~/murmur-rmp`) as its **own standalone Cargo package that is NOT a workspace member**.

- The workspace `Cargo.toml` uses an explicit `members = [...]` list, so a new directory is not auto-joined. **However**, cargo errors on a nested `Cargo.toml` under the workspace root unless it is a member *or* excluded. Therefore the spike adds **one line** to the workspace manifest: `exclude = ["spikes"]`. **Note:** `workspace.exclude` is plain string-prefix matching, **not** glob — a `"spikes/*"` pattern is a no-op (globs only work in `members`; cargo issue #11405). `"spikes"` prefix-excludes the whole subtree. This is the only touch to existing repo files. With it, `cargo test --workspace`, `cargo build --workspace`, and `cargo clippy --all-targets` over the workspace **never see the spike** — the CI gates stay green regardless of whether whisper.cpp builds.
- The spike gets its **own nix shell** (`spikes/stt-whisper/shell.nix`) adding the C/C++ toolchain whisper.cpp needs (`cmake`, `clang`, `libclang` for `bindgen`), so the workspace `flake.nix` (currently just `cargo rustc clippy rustfmt rust-analyzer`) stays clean. The main dev shell must not grow heavy native deps for a throwaway experiment.
- Model files are **large** (18 MB – 1.6 GB). `spikes/stt-whisper/models/` is **gitignored**; a `download-models.sh` script fetches them at run time from the official ggml HuggingFace repo. **Never commit models.**

### Why `whisper-rs` (not direct FFI, not WhisperKit)

- **`whisper-rs`** (tazz4843, MIT, `0.16.0` 2026-03-12 per survey) is the mainstream Rust binding to whisper.cpp. It vendors whisper.cpp as a submodule and builds it via `whisper-rs-sys` (cmake + bindgen), exposing feature flags including **`metal`** (Apple Silicon GPU) and `coreml` (ANE encoder). Using it *is* the thing we're deciding to adopt — the spike should measure the real candidate, not a proxy.
- **Direct whisper.cpp FFI** would be reinventing `whisper-rs-sys` for no spike benefit. Rejected.
- **WhisperKit** is Swift/CoreML, not Rust — it's the *platform-adjacent* comparison point, not the Rust-side candidate. The spike's job is to test the Rust path dam prefers; WhisperKit/Apple numbers are the fallback baseline and can be gathered later in Plan 06 proper if this spike says GO-with-caveats.
- **License:** whisper.cpp MIT, `whisper-rs` MIT, ggml models MIT (OpenAI Whisper weights, MIT). All permissive. `distil-whisper` weights are MIT (HuggingFace/distil-whisper). Record exact license + source URL for every artifact pulled in, in `RESULTS.md`'s attribution section.

### The iOS tier: cheapest path to real-device numbers

Two candidate paths were considered:

- **(A) Minimal UniFFI surface** — wrap the spike Rust crate in a UniFFI object, generate Swift bindings, call from a bare SwiftUI app. This is **real Plan 07 work** (the FFI streaming boundary the survey flags as the hard part, and Plan 05 Deferred 3 names the `&mut self` → `&self` actor wrapper problem). Doing it for a spike front-loads the exact integration cost we're trying to de-risk *later*.
- **(B) whisper.cpp's own bundled iOS example app** — the whisper.cpp repo ships `examples/whisper.swiftui` (and `whisper.objc`), a working SwiftUI app that loads a ggml model and transcribes on-device via the Metal/CoreML backend. It builds with `build-xcframework.sh` (whisper.cpp ships this) → drop-in `whisper.xcframework`. **Near-zero code** to get real-device RTF/thermal/battery.

**Decision: path (B) for the spike.** The iPhone tier's only job is battery/thermal/RTF on real silicon — path (B) delivers that with a fork of an existing app and no FFI design. **UniFFI is explicitly the Plan 07 concern** and stays deferred. Justification: a spike answers "can the *engine* run acceptably on the phone," not "is our *binding architecture* right" — conflating them wastes the spike. If dam wants to prove the Rust-crate-through-UniFFI path specifically, that's a Plan 06/07 task, noted in Deferred.

### Tooling built in the spike (all disposable)

- A tiny **WER + term-recall** module (token-level Levenshtein; target-term recall = fraction of curated vocabulary terms present in reference that appear in hypothesis). No external crate — keep it self-contained and hermetically unit-testable.
- A **chunk-boundary overlap** measure: run the decoder over sliding windows, diff consecutive windows' emitted text, report overlap % and a proposed dedup/finalize rule; measure finalize-latency (audio-time between a word being spoken and it entering the append-only stream).
- A CLI (`cargo run -- <subcommand>`) with subcommands per experiment, each printing a `RESULTS.md`-ready row.

---

## Results table schema (define up front — this is the spike's output)

`spikes/stt-whisper/RESULTS.md` opens with these tables. Every task fills rows.

**Table 1 — Feasibility & performance (Mac, Apple Silicon, Metal backend):**

| Model | Quant | Size (MB) | Load (s) | RTF | Peak RSS (MB) | Backend | Notes |
|-------|-------|-----------|----------|-----|---------------|--------|-------|
| tiny.en | q5_1 | | | | | metal | |
| base.en | q5_1 | | | | | metal | |
| small.en | q5_1 | | | | | metal | |
| large-v3-turbo | q5_0 | | | | | metal | |
| distil-large-v3 | q5_0 | | | | | metal | |

> RTF = wall-clock decode time ÷ audio duration, measured on the **second** decode (first is a discarded Metal-shader-JIT warm-up — state this in Notes). **RTF < 1.0** = faster than real-time. Lower is better. Peak RSS from `getrusage` `ru_maxrss` (bytes on macOS).

**Table 2 — Streaming / append-only (chosen model from Table 1):**

| Chunk (s) | Overlap (s) | Boundary re-transcription % | Finalize latency (s) | Append-only derivable? | Notes |
|-----------|-------------|-----------------------------|----------------------|------------------------|-------|

**Table 3 — Accuracy & biasing (per model × condition):**

| Model | Audio clip | Noise cond. | WER % | Target-term recall (no bias) | Target-term recall (initial_prompt) | Recall Δ (pp) | Hallucination flag | Notes |
|-------|-----------|-------------|-------|------------------------------|-------------------------------------|---------------|--------------------|-------|

**Table 4 — iPhone tier (optional, real device):**

| Device | iOS | Model | RTF | Battery Δ (%/10 min) | Thermal state @ 10 min | Killed in background? | Notes |
|--------|-----|-------|-----|----------------------|------------------------|-----------------------|-------|

---

## File structure

```
spikes/stt-whisper/            # NOT a workspace member (excluded)
  Cargo.toml                   # standalone package; whisper-rs dep with metal feature
  shell.nix                    # cmake + clang + libclang for whisper.cpp build
  .gitignore                   # models/, target/, *.wav recordings
  download-models.sh           # fetch ggml models from HuggingFace at run time
  README.md                    # how to run each experiment
  RESULTS.md                   # THE DELIVERABLE — the four tables + decision
  src/
    main.rs                    # CLI dispatch: bench | stream | accuracy | bias
    wer.rs                     # WER + target-term recall (unit-tested)
    stream.rs                  # chunked decode + overlap/finalize measurement
    bench.rs                   # load/RTF/memory harness
  audio/                       # gitignored; test clips live here
    scripts/                   # committed: the read-aloud jargon scripts (text)
    references/                # committed: hand transcripts (ground truth)
  ios/                         # optional tier: notes + fork pointer, see Task 6
Cargo.toml (workspace)         # MODIFY: add exclude = ["spikes"]  (prefix, not glob)
```

Only `Cargo.toml` (one line) is touched outside `spikes/`.

---

### Task 0: Quarantined scaffold + nix shell + workspace exclude

**Files:** create `spikes/stt-whisper/{Cargo.toml,shell.nix,.gitignore,README.md}`; modify workspace `Cargo.toml`.

- [ ] **Step 1: Exclude the spike from the workspace.** In `~/murmur-rmp/Cargo.toml`, add to the `[workspace]` table:
  ```toml
  exclude = ["spikes"]   # prefix match — NOT "spikes/*" (workspace.exclude is not glob; cargo #11405)
  ```
- [ ] **Step 2: Verify the gates are untouched.** From the repo root:
  ```sh
  nix shell nixpkgs#cargo nixpkgs#rustc -c cargo test --workspace   # must still pass, spike invisible
  nix shell nixpkgs#cargo nixpkgs#rustc -c cargo clippy --all-targets  # zero warnings, spike invisible
  ```
- [ ] **Step 3: Scaffold the spike package.** `spikes/stt-whisper/Cargo.toml`:
  ```toml
  [package]
  name = "stt-whisper-spike"
  version = "0.0.0"
  edition = "2021"
  publish = false

  [dependencies]
  whisper-rs = { version = "=0.16.0", features = ["metal"] }  # exact pin: a silent patch bump would confound Table 1 mid-spike
  hound = "3"        # WAV read (16 kHz mono f32 — whisper's input format)
  # deliberately minimal; WER/stream logic is hand-rolled and disposable
  ```
  `spikes/stt-whisper/shell.nix` — the native toolchain whisper.cpp's build needs, kept OUT of the workspace flake:
  ```nix
  { pkgs ? import <nixpkgs> {} }:
  pkgs.mkShell {
    packages = with pkgs; [ cargo rustc cmake clang ];
    # bindgen needs libclang on its path:
    LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
  }
  ```
  `.gitignore`:
  ```
  /target
  /models
  /audio/*.wav
  ```
- [ ] **Step 4: Prove `whisper-rs` builds with Metal (KILL-QUESTION 1, feasibility gate).**
  ```sh
  cd spikes/stt-whisper && nix-shell --run "cargo build"
  ```
  **Expected:** whisper-rs-sys compiles whisper.cpp (cmake + Metal).
  **If the nix build fails** (missing framework, bindgen/libclang failure, Metal shader/link error), do NOT record KILL evidence yet — first disambiguate "the *engine* can't build here" from "*this nix shell* lacks the Xcode toolchain." **Mandatory fallback:** retry the build **outside** the nix shell against system Xcode command-line tools:
  ```sh
  xcrun --sdk macosx --show-sdk-path          # must resolve to a valid SDK
  # if needed: export DEVELOPER_DIR=$(xcode-select -p); export SDKROOT=$(xcrun --sdk macosx --show-sdk-path)
  cargo build                                 # system toolchain, no nix shell
  ```
  - **nix fails but system-Xcode succeeds** → this is an **environment note only** ("Rust-side native build needs Xcode-managed toolchain, not pure nix"), NOT KILL evidence. Record it under Feasibility and carry on with the system-toolchain path for the remaining tasks.
  - **BOTH fail** after ~2–3 documented attempts → this is the *material feasibility signal*. STOP, record the exact failures in `RESULTS.md` under "Feasibility," and message the team: partial-KILL evidence toward Option C.
  Do not rabbit-hole past ~2–3 documented attempts on either path.
- [ ] **Step 5: Commit.**
  ```sh
  git add -A && git commit -m "spike(stt): quarantined whisper-rs scaffold + nix shell; exclude from workspace"
  ```

---

### Task 1: Model download script + attribution

**Files:** create `spikes/stt-whisper/download-models.sh`.

- [ ] **Step 1:** Script fetches quantized ggml models on demand into gitignored `models/` from the official HuggingFace repo `huggingface.co/ggml-org/whisper.cpp` (the whisper.cpp project now lives under the `ggml-org` org; the old `ggerganov/whisper.cpp` URL redirects) — and `distil-whisper` for the distil model. Accept a model name arg; default to fetching `base.en` + `small.en` (the likely sweet spot). Verify size after download; never commit.
- [ ] **Step 2:** Record in `RESULTS.md` attribution section: each model's source URL, size, quantization, and **license** (Whisper ggml = MIT; distil-whisper = MIT). whisper.cpp = MIT, whisper-rs = MIT.
- [ ] **Step 3:** Commit the script (not the models).
  ```sh
  git add -A && git commit -m "spike(stt): model download script (gitignored models) + license notes"
  ```

---

### Task 2: Feasibility + RTF/memory benchmark CLI → fills Table 1

**Files:** create `spikes/stt-whisper/src/{main.rs,bench.rs}`.

- [ ] **Step 1: Implement `bench` subcommand.** `cargo run -- bench --model models/ggml-base.en-q5_1.bin --audio audio/clip.wav`:
  - Load model (`WhisperContext::new_with_params`), time it (load seconds).
  - **Warm-up decode, then time the SECOND decode.** Metal JIT-compiles shaders on the first decode; timing that skews small models the worst — the exact model-vs-model comparison the decision leans on. So: run one **discarded** warm-up decode after load, then time a fresh decode for RTF. Record the warm-up policy in the Notes column ("RTF = 2nd decode, warm-up discarded").
  - Decode via `WhisperState::full(params, &samples)`; retrieve segments via `state.full_n_segments()` + `state.get_segment(i)` (or `state.as_iter()`). Compute **RTF = decode_secs / audio_secs**.
  - Sample **peak RSS** via `getrusage(RUSAGE_SELF).ru_maxrss`. **On macOS `ru_maxrss` is in BYTES** (Linux: kilobytes) — bake the conversion into the code, not a prose warning:
    ```rust
    // macOS: ru_maxrss is BYTES (Linux would be KiB — this spike targets macOS)
    let peak_mb = rusage.ru_maxrss as f64 / (1024.0 * 1024.0);
    ```
  - Print a `RESULTS.md`-ready Table-1 row.
- [ ] **Step 2: Run across the model matrix** (whichever download; at minimum `tiny.en`, `base.en`, `small.en`; add `large-v3-turbo` and `distil-large-v3` if disk allows). Fill Table 1.
- [ ] **Step 3: Sanity-check Metal is actually engaged** — whisper.cpp logs the backend; confirm "Metal" not "CPU" in stderr. Note it in the Backend column. (A CPU-only number silently inflates RTF and misleads the iPhone proxy.)
- [ ] **Step 4: Commit** with Table 1 filled.
  ```sh
  git add -A && git commit -m "spike(stt): Mac RTF/memory benchmark across model sizes (Table 1)"
  ```

**Exit signal from this task:** a `base`/`small`-class model at **RTF < 0.5** on the Mac is the optimistic proxy that keeps the iPhone (typically 3–5× slower + thermal-limited) plausibly real-time. RTF ≥ 1.0 on the Mac for the smallest usable model is a strong KILL signal.

---

### Task 3: Chunked pseudo-streaming + append-only derivation → fills Table 2

**Files:** create `spikes/stt-whisper/src/stream.rs`; wire `stream` subcommand.

- [ ] **Step 1: Implement sliding-window chunked decode.** `cargo run -- stream --model ... --audio ... --chunk 5 --overlap 1`:
  - Feed the clip in N-second chunks with a small overlap (context to avoid word-splitting at boundaries).
  - For each chunk, decode via `WhisperState::full()` and capture emitted segments with timestamps — either drain after each chunk with `full_n_segments()` / `get_segment(i)` / `as_iter()`, or stream them live via `FullParams::set_segment_callback_safe(closure)`.
  - **Measure boundary re-transcription** as **token-level edit distance, reusing the WER module's function** (Task 4 `wer.rs`): over the overlap window between chunk *k* and chunk *k+1*, report **re-emitted-or-revised words ÷ total words in the overlap window** (a %). Don't hand-roll a second diff — one Levenshtein implementation, unit-tested once.
  - **Propose a finalize rule:** a word is *finalized* (safe to append to the cursor's append-only stream) once it is stable across the overlap region / older than the overlap horizon. Emit the finalized stream and measure **finalize latency** = audio-time from when a word is spoken to when it becomes finalized.
- [ ] **Step 2: Unit-test the dedup/finalize logic** hermetically (feed synthetic overlapping segment lists; assert the finalized stream is append-only — no previously-emitted word is later revised). This is the one place TDD earns its keep: the finalize invariant is the contract Plan 05's cursor depends on.
- [ ] **Step 3: Sweep chunk/overlap params** (e.g. chunk ∈ {3,5,10}s, overlap ∈ {0.5,1,2}s); fill Table 2. Record whether an append-only stream is derivable and at what latency cost.
- [ ] **Step 4: Commit.**
  ```sh
  git add -A && git commit -m "spike(stt): chunked streaming + append-only finalize measurement (Table 2)"
  ```

**Ties to Plan 05:** the finalized stream this produces is exactly what `append_transcript` would receive; the cursor consumes it append-only (Plan 05 design note 5). If append-only requires unbounded re-decode or finalize latency is multi-second-large, that's a KILL signal for the live-extraction UX (though end-of-session `process()` still works).

---

### Task 4: Accuracy + biasing experiment → fills Table 3

**Files:** create `spikes/stt-whisper/src/wer.rs`; wire `accuracy` and `bias` subcommands; add `audio/scripts/` + `audio/references/`.

- [ ] **Step 1: Assemble the test corpus (specify obtainable audio).**
  - **Self-recorded (primary):** write 2–3 short (~60–90s) read-aloud scripts dense with construction/trade jargon and proper nouns (e.g. "french drain," "ledger board," "2x10 joists," "GFCI," "Simpson Strong-Tie," subcontractor names). Commit the **scripts** (`audio/scripts/*.txt`) and **hand transcripts** (`audio/references/*.txt`) — these are the ground truth. dam records himself reading each: once **quiet**, once with **jobsite noise** (play a free jobsite-ambience clip through a speaker — good enough for a spike). WAVs stay gitignored.
  - **Clean baseline (optional sanity):** 1–2 LibriSpeech `test-clean` clips (CC BY 4.0) to anchor "how good is this model on easy audio" vs. our noisy clips. Attribute the license.
  - **"Good enough" bar:** this corpus is for *relative* comparison (model vs. model, biased vs. unbiased, quiet vs. noisy) — not an absolute WER claim. Construction-site realism is aspirational; self-recorded-with-noise is the spike-grade proxy. Say so in RESULTS.
- [ ] **Step 2: Implement WER + target-term recall** in `wer.rs` (token-level Levenshtein WER; term-recall = fraction of the curated vocabulary terms present in the reference that also appear in the hypothesis). Unit-test both on tiny fixed strings.
- [ ] **Step 3: `accuracy` subcommand** — decode each clip (`WhisperState::full()`, retrieve via `full_n_segments()` / `get_segment(i)` / `as_iter()`), compute WER vs. reference, fill Table 3's WER column across models × noise conditions.
- [ ] **Step 4: `bias` subcommand (KILL-QUESTION 3)** — decode each jargon clip twice: (a) no biasing, (b) with the ≤100-term vocabulary injected via `whisper-rs`'s `FullParams::set_initial_prompt`. Compute **target-term recall** for both; report the delta (percentage points). **Watch for hallucination:** the survey warns `initial_prompt` used as a keyword list can induce repetition-loop hallucinations — flag any clip where biased output shows obvious insertions/loops (Hallucination flag column). Fill Table 3.
- [ ] **Step 5: Commit.**
  ```sh
  git add -A && git commit -m "spike(stt): WER + initial_prompt biasing experiment (Table 3)"
  ```

**Interpretation:** `initial_prompt` is a *mechanically mismatched* biasing surface (survey §4) — a measurable positive recall Δ with tolerable hallucination is a GO-with-caveat (the real win needs the trie/logit-bias follow-on). **No lift or net-negative (hallucination) means the vocabulary→STT differentiator requires the deep decoder work** — that raises the cost of Option B and must be surfaced explicitly, not buried.

---

### Task 5: iPhone reality-check tier (OPTIONAL — real hardware) → fills Table 4

**Files:** create `spikes/stt-whisper/ios/README.md` (notes + fork pointer). No app committed to sitewalk.

> This tier is **optional and hardware-gated** — it runs on dam's physical iPhone (Apple Silicon dev machine builds it). The Mac CLI (Tasks 0–4) is the runnable-today deliverable; this is the "if dam has an hour with a device" tier. The **simulator is explicitly insufficient** for the numbers that matter here (no Metal/ANE, no real battery/thermal) — it's good only for a functional "does it transcribe at all on iOS" smoke test.

- [ ] **Step 1: Build whisper.cpp's bundled iOS example.** Clone whisper.cpp, run `./build-xcframework.sh` to produce `whisper.xcframework`, open `examples/whisper.swiftui`, point it at a quantized model (`base.en`/`small.en` q5), build to the device. (Path B from Architecture — no UniFFI, minimal code.)
- [ ] **Step 2: Measure on device** — decode a ~60s clip, record **RTF** (app logs decode time); run a ~10-min sustained loop and record **battery Δ**, **thermal state** (`ProcessInfo.thermalState`), and whether iOS **kills the app in background** (the survey's biggest open question for hour-long locked capture). Fill Table 4.
- [ ] **Step 3: Document, don't productionize.** `ios/README.md` records the fork commit/pointer, the model used, and the numbers. This app is throwaway — the real iOS integration (UniFFI surface over the Rust core, background-audio session, chunked live feed) is **Plan 07**, not this spike.
- [ ] **Step 4: Commit** the notes.
  ```sh
  git add -A && git commit -m "spike(stt): iPhone device-tier notes + Table 4 (whisper.cpp example app)"
  ```

---

### Task 6: Synthesize RESULTS.md → GO/KILL decision

**Files:** finalize `spikes/stt-whisper/RESULTS.md`.

- [ ] **Step 1:** Ensure all four tables are filled (Table 4 marked "not run — no device" if the optional tier was skipped).
- [ ] **Step 2:** Write the **Decision** section against the exit criteria below — a clear GO (Option B), KILL→fallback (Option C), or GO-with-named-caveats. Cite the specific numbers. **When evaluating exit-criterion 2, join the RTF (Table 1) and WER (Table 3) from the SAME model row** — a fast model that's inaccurate, or an accurate model that's slow, does not satisfy it; one model must clear both bars.
- [ ] **Step 3:** Write the **Attribution** section (every model/corpus/library: source URL + license).
- [ ] **Step 4:** Message the team-lead with: doc path, the decision, and the load-bearing numbers.
  ```sh
  git add -A && git commit -m "spike(stt): RESULTS decision document — GO/KILL for whisper.cpp Rust-side"
  ```

---

## Exit criteria (the decision gate)

**GO — commit Plan 06 to whisper.cpp Rust-side (Option B)** requires ALL of:

1. **Feasibility:** `whisper-rs` with the `metal` feature builds and runs in the nix shell on Apple Silicon Mac (Task 0 Step 4 passed).
2. **Performance:** at least one `base`/`small`-class model achieves **RTF < 0.5** on the Mac (Table 1) **and** an **acceptable WER** on the noisy proxy corpus (**≤ ~20% noisy, ≤ ~10% clean** — spike-grade bars, relative not absolute) — **both from the same model row** (Table 1 RTF + Table 3 WER for one model, not a fast model and a separate accurate one).
3. **Append-only:** a **finalized append-only stream is derivable** from chunked decode (Table 2) with **finalize latency ≤ ~3 s** and bounded, dedup-able boundary overlap (Task 3 unit test green).
4. **Biasing:** `initial_prompt` shows a **measurable target-term recall lift (≥ ~10 pp)** without runaway hallucination (Table 3) — OR, if lift is marginal, the result clearly scopes the trie/logit-bias follow-on as *tractable* rather than *prohibitive*.
5. **iPhone (if the optional tier ran):** the chosen model is **real-time-capable (RTF < 1.0) on device** and survives ~10 min sustained without a thermal kill. (If not run, GO is provisional pending a device check in Plan 06.)

**KILL — fall back to the staged-hybrid Option C (Apple `SpeechAnalyzer` for v1, Rust path deferred)** if ANY of:

- whisper-rs cannot be made to build/run in nix after ~2–3 documented attempts; **or**
- no usable model class hits **RTF < 0.5** at tolerable WER on the Mac proxy (implies the iPhone thermal envelope is hopeless); **or**
- append-only derivation needs unbounded re-transcription or finalize latency is unacceptably large (live extraction UX unviable); **or**
- biasing shows **no lift / net-negative (hallucination)** AND the trie/logit-bias follow-on looks prohibitively deep — at which point Apple's simpler platform path (accepting the biasing gap for v1) becomes the pragmatic ship.

**GO-with-caveats** (the likely realistic outcome): whisper.cpp is feasible and performant but the `initial_prompt` biasing is weak — meaning Option B is right *and* Plan 06 must budget the trie/logit-bias hotword decoder as the real differentiator work. The spike's value is turning that from an assumption into a measured cost.

---

## Deferred (named, out of spike scope)

1. **Hotword logit-bias / prefix-trie decoder.** The high-payoff biasing technique (survey §4: 19–22% B-WER reduction in the literature; whisper.cpp issue #1979). Requires implementing against whisper.cpp decoder internals — a Plan 06/07 engineering item, not a spike. The spike only measures whether the cheap `initial_prompt` surface is *enough* (it probably isn't) to decide whether this work is mandatory.
2. **UniFFI surface for the Rust STT crate.** The real iOS integration path (Rust core behind UniFFI, `&mut self`→actor wrapper per Plan 05 Deferred 3, background-audio session, streaming callback). Deliberately avoided in the spike's iOS tier (Task 5 uses whisper.cpp's stock example app instead). Plan 07.
3. **Android.** The Rust-side path's cross-platform payoff. Not tested here; noted as a reason Option B beats Option A if the numbers are close.
4. **Battery/thermal instrumentation depth.** Task 5 captures coarse battery Δ / thermal state / background-kill. Fine-grained power profiling (Instruments Energy Log, per-component NE duty cycle) is Plan 06/07.
5. **Diarization** (FluidAudio/pyannote path, survey §7). Nice-to-have, explicitly not v1-critical.
6. **CoreML/ANE encoder path.** The spike uses the Metal backend; whisper.cpp's CoreML encoder (claimed >3× faster) needs a separate model conversion step. Worth a Table-1 row *if trivial to add*, otherwise a Plan 06 optimization.
7. **Record-then-transcribe vs. continuous.** The survey's dominant shipping pattern for hour-long capture. The spike measures continuous chunked decode (the harder live path); the batch-after pattern is a fallback the architecture should keep open, not something to benchmark here.

---

## Self-Review Notes

- **Kill-question coverage:** feasibility ✓ (Task 0 Step 4 hard gate + Task 2); streaming/append-only ✓ (Task 3, tied to Plan 05 cursor contract); biasing ✓ (Task 4 `initial_prompt` experiment, hallucination-aware); accuracy proxy ✓ (Task 4, obtainable self-recorded + LibriSpeech corpus, "good enough" bar stated); iPhone reality ✓ (Task 5 optional device tier, simulator-insufficiency called out, Mac-first). All five map to explicit `RESULTS.md` tables + exit criteria.
- **Quarantine integrity:** the only file touched outside `spikes/` is the workspace `Cargo.toml` (one `exclude` line), and Task 0 Step 2 verifies `cargo test --workspace` / `clippy` stay green with the spike present. This is the load-bearing "don't pollute the real gates" guarantee — if `exclude` doesn't suffice (cargo version quirk), the fallback is to place the spike *outside* the repo root or in a sibling dir, but excluded-nested is cleanest and standard.
- **Judgment calls for reviewers:** (a) `whisper-rs` over direct FFI — measure the real candidate, not a proxy. (b) iOS tier via whisper.cpp's own example app, NOT UniFFI — a spike answers "can the engine run on the phone," not "is our binding right"; UniFFI is Plan 07's cost to pay. (c) `initial_prompt` biasing in-scope but trie/logit-bias deferred — the spike's job is to *prove `initial_prompt` is insufficient* and thereby justify (or not) the deep work, not to do the deep work. (d) self-recorded-with-noise corpus called "good enough" — real jobsite audio is aspirational; relative deltas are what the decision needs. (e) RTF < 0.5 on Mac as the iPhone proxy threshold — the Mac is optimistic (3–5× faster, no thermal cap), so a comfortable Mac margin is required for iPhone plausibility.
- **What I'm least sure of:**
  - **The `metal` feature building cleanly in a bare `nix-shell`** — whisper.cpp's Metal path may want the Metal framework and `xcrun`/Metal shader compilation that nix's clang doesn't fully provide outside a full Xcode toolchain. This is *itself* a feasibility finding (Task 0 Step 4); if it fights, the honest answer might be "Rust-side needs Xcode-managed native build, not pure nix" — which is a real friction to report, not a spike failure to hide. Flagged as the highest-risk step.
  - **Peak-memory measurement on macOS** — `/usr/bin/time -l` scraping is crude; `getrusage(RUSAGE_SELF).ru_maxrss` is per-process and on macOS is in *bytes* (Linux: kilobytes) — easy to misreport by 1024×. Task 2 must state units explicitly.
  - **Finalize-latency definition** — "audio-time from spoken to finalized" depends on the finalize rule chosen; the number is only meaningful alongside the rule. Task 3 must report both together, not the latency alone.
  - **Whether `RTF < 0.5` Mac / `< 1.0` iPhone are the right bars** — these are my spike-grade proposals; dam may want to move them once Table 1 shows the actual spread. The criteria are explicit precisely so they can be renegotiated with data, not vibes.
