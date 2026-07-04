# STT whisper.cpp Rust-side spike — RESULTS

**The deliverable of Plan 06-spike.** Decides dam's stated preference — *"go straight to
whisper.cpp Rust-side only"* (Option B) — against measured evidence, vs. the staged-hybrid
fallback (Option C: Apple `SpeechAnalyzer` for v1).

- **Host:** Apple Silicon Mac (dam's dev machine), macOS. Metal backend.
- **Engine:** `whisper-rs =0.16.0` (pinned) → `whisper-rs-sys 0.15.0` → vendored whisper.cpp.
- **Status:** Mac tiers (T0–T4, T6) executed by the spike worker. iPhone tier (T5) **pending — needs dam's device.**

---

## Table 1 — Feasibility & performance (Mac, Apple Silicon, Metal backend)

Host: **Apple M4 Max**, macOS 26.2, Metal backend (`use gpu = 1`, `Metal total size` confirmed
in whisper.cpp stderr for every model — no CPU fallback). Audio: `jargon1.wav`, 59.8 s, 16 kHz
mono. Each model measured in its own process (peak RSS is that model's own high-water mark).

| Model | Quant | Size (MB) | Load (s) | RTF | Peak RSS (MB) | Backend | Notes |
|-------|-------|-----------|----------|-----|---------------|--------|-------|
| tiny.en | q5_1 | 32 | 0.08 | **0.006** | 161 | metal | decode 0.36 s / 59.8 s |
| base.en | q5_1 | 60 | 0.09 | **0.009** | 205 | metal | decode 0.51 s / 59.8 s |
| small.en | q5_1 | 190 | 0.13 | **0.021** | 392 | metal | decode 1.25 s / 59.8 s |
| large-v3-turbo | q5_0 | 574 | 0.27 | **0.041** | 786 | metal | decode 2.47 s / 59.8 s |
| distil-large-v3 | **f16** | 1520 | 0.66 | **0.029** | 1703 | metal | decode 1.72 s; f16 (no q5 ggml published) |

> RTF = wall-clock decode time ÷ audio duration, measured on the **second** decode (first is a
> discarded Metal-shader-JIT warm-up). RTF < 1.0 = faster than real-time. Peak RSS from
> `getrusage` `ru_maxrss` (**bytes** on macOS — conversion baked into `peak_rss_mb()`).
>
> **Load-time note:** the first whisper.cpp process on this machine paid a one-time
> `ggml_metal_library_init: loaded in 7.35 sec` (embedded Metal shader library compile). That
> shader cache is OS-level and warm for subsequent processes, so the load times above (0.08–0.66 s)
> are **steady-state** (cache-warm). First-ever cold launch on a fresh machine adds ~7 s once.
>
> **Result:** every model — including the largest — is **far under RTF 0.5** on this Mac
> (fastest usable model `base.en` at RTF 0.009, ~55× faster than real-time). The Mac is the
> optimistic proxy; even a pessimistic 5–10× iPhone slowdown keeps `base`/`small` comfortably
> real-time. Feasibility + performance bars: **cleared with large margin.**

## Table 2 — Streaming / append-only (chosen model: small.en, `jargon1.wav`)

| Chunk (s) | Overlap (s) | Boundary re-transcription % | Finalize latency (s) | Append-only derivable? | Notes (streaming WER vs. reference) |
|-----------|-------------|-----------------------------|----------------------|------------------------|-------|
| 3 | 1 | 87 | 2.0 max / 1.1 avg | invariant yes (unit-tested) | naive-finalize 80%; **dedup-reassembly 28%** |
| 5 | 1 | 95 | 3.0 max / 1.8 avg | invariant yes (unit-tested) | naive-finalize 80%; **dedup-reassembly 19%** |
| 5 | 2 | 75 | 3.0 max / 2.3 avg | invariant yes (unit-tested) | naive-finalize 77%; **dedup-reassembly 29%** |
| 10 | 2 | 75 | 8.0 max / 4.6 avg | invariant yes (unit-tested) | naive-finalize 25%; **dedup-reassembly 20%** |
| 15 | 3 | 77 | 13.0 max / 7.0 avg | invariant yes (unit-tested) | naive-finalize 18%; **dedup-reassembly 11%** |
| 30 | 5 | 30 | 27.3 max / 14.9 avg | invariant yes (unit-tested) | naive-finalize 12%; **dedup-reassembly 5%** (≈ batch) |

> **Bounded re-decode:** yes — each chunk is decoded exactly once with a fixed overlap; no
> unbounded re-transcription. **Append-only invariant:** proven and unit-tested
> (`finalized_stream_is_append_only`, `no_double_emit_of_overlap`) — a word committed by an early
> chunk is never revised by a later one, even when the later chunk re-transcribes the overlap
> differently.
>
> **The finalize rule matters enormously.** Two reassembly rules were measured:
> - **Naive time-horizon finalize** (commit segments older than `chunk_end − overlap`): lossy on
>   short chunks (77–80% WER at 3–5 s) — whisper re-segments each chunk differently, so
>   segment-level time-tiling drops content that falls in the deferral gaps.
> - **LocalAgreement-style text-overlap dedup** (merge consecutive chunks on longest suffix/prefix
>   token match — a ~30-line function, `reassemble_dedup`, the technique `whisper_streaming` uses):
>   recovers most of the loss. At **5 s chunk / 1 s overlap → 19% WER with max finalize latency
>   3.0 s (avg 1.8 s)**. At 3 s/1 s → 28% WER at ≤2 s latency.
>
> **Boundary re-transcription is high (75–95%) for short chunks** — whisper heavily re-segments
> the overlap window, confirming that finalize must operate at the **word/token level**, not the
> segment level.
>
> **The latency ↔ accuracy tension:** short chunks give low finalize latency but higher streaming
> WER (whisper is trained on 30 s windows, so short isolated chunks transcribe worse); 30 s chunks
> reach batch-quality (5%) but 15–27 s finalize latency. **The ≤3 s-latency sweet spot (5 s/1 s)
> costs ~19% WER — ~4× the batch WER (5%) of the same model.**
>
> **Verdict for the Plan 05 cursor contract:** an append-only, bounded-overlap, dedup-able,
> ≤3 s-latency finalized stream **is derivable** — but only with a LocalAgreement word-level
> finalize (not the naive segment rule), and live streaming WER is materially worse than batch.
> **Implication:** live in-session extraction is viable as a *preview/provisional* stream;
> end-of-session `process()` on the full audio (Table 3 WER, ~5%) should remain the
> **authoritative** pass. The LocalAgreement finalize is tractable (the spike's 30-line proto
> already recovers most content) — a Plan 06 engineering item, not a blocker.

## Table 3 — Accuracy & biasing (per model × condition)

Corpus: `jargon1.wav` (59.8 s, construction/trade jargon), macOS `say` TTS (spike-grade proxy —
see below). Reference = the verbatim script. 21 of the 44 curated terms appear in this clip's
reference (the rest belong to `jargon2`). "noisy" = synthetic additive white noise at +10 dB SNR
(reproducible, fixed-seed) — a proxy for jobsite ambience.

| Model | Audio clip | Noise cond. | WER % | Target-term recall (no bias) | Target-term recall (initial_prompt) | Recall Δ (pp) | Hallucination flag | Notes |
|-------|-----------|-------------|-------|------------------------------|-------------------------------------|---------------|--------------------|-------|
| tiny.en | jargon1 | quiet | 9.9 | — | — | — | no | WER-only run |
| tiny.en | jargon1 | noisy +10dB | 18.1 | — | — | — | no | WER-only run |
| base.en | jargon1 | quiet | 5.8 | 81% (17/21) | 90% | **+10** | no | 44-term initial_prompt |
| base.en | jargon1 | noisy +10dB | 11.7 | 71% (15/21) | 90% | **+19** | no | 44-term initial_prompt |
| small.en | jargon1 | quiet | 4.7 | 86% (18/21) | 100% | **+14** | no | 44-term initial_prompt |
| small.en | jargon1 | noisy +10dB | 11.7 | 71% (15/21) | 90% | **+19** | no | 44-term initial_prompt |
| large-v3-turbo | jargon1 | quiet | 6.4 | 86% (18/21) | 90% | +5 | no | 44-term initial_prompt |
| large-v3-turbo | jargon1 | noisy +10dB | 8.8 | 86% (18/21) | 95% | +10 | no | 44-term initial_prompt |

> **Accuracy (kill-question 4):** every candidate model clears the spike bars (≤10% clean,
> ≤20% noisy). `small.en` best on clean (4.7%); `large-v3-turbo` most noise-robust (8.8% noisy).
> `base.en`/`small.en` clear both bars.
>
> **Biasing (kill-question 3):** `initial_prompt` injection of the curated vocabulary gives a
> **measurable positive term-recall lift with zero hallucination** across all models — +10 pp
> (base, quiet), +14 pp (small, quiet), and +19 pp for base/small under noise (where un-biased
> recall drops and the prompt recovers it). This is a **stronger result than the plan predicted**
> (the survey expected `initial_prompt` to be mechanically mismatched / near-useless). The
> hallucination heuristic (length blow-up or ≥5× token repetition) fired on **none** of the runs.
>
> **Caveats that keep this spike-grade, not production:** (1) TTS audio is cleaner and more
> uniform than a human on a real jobsite — absolute WER is optimistic and real biasing may induce
> more hallucination; (2) only the 21 clip-relevant terms could lift (the prompt carried all 44);
> a full 100-term list against unrelated audio is the case most likely to hallucinate and is
> untested here; (3) recall is measured on contiguous n-gram presence, a coarse proxy. The
> **direction and magnitude** are what the decision needs, and both favor Option B. The deeper
> trie/logit-bias decoder (survey §4, 19–22% B-WER lit. gains) remains the higher-payoff
> follow-on, but this result shows it is an **optimization, not a prerequisite** for a usable v1.

## Table 4 — iPhone tier (optional, real device)

**PENDING — not run.** Requires dam's physical iPhone (T5, hardware-gated). The iOS simulator
is explicitly insufficient (no Metal/ANE, no real battery/thermal). See `ios/README.md` for the
build recipe (whisper.cpp's bundled `examples/whisper.swiftui`, path B — no UniFFI).

| Device | iOS | Model | RTF | Battery Δ (%/10 min) | Thermal state @ 10 min | Killed in background? | Notes |
|--------|-----|-------|-----|----------------------|------------------------|-----------------------|-------|
| — | — | — | — | — | — | — | pending device |

---

## Feasibility (kill-question 1)

**PASS — `whisper-rs =0.16.0` with the `metal` feature builds and runs on this Apple Silicon Mac.**

- `nix-shell` (spike-local `shell.nix`: `cargo rustc cmake clang` + `LIBCLANG_PATH`) built the
  full native stack cleanly: `whisper-rs-sys 0.15.0` compiled vendored whisper.cpp via cmake +
  bindgen; `stt-whisper-spike` linked and ran. Release build: ~32 s cold.
- **Environment note (not KILL evidence):** the plan's `shell.nix` uses `import <nixpkgs>`, but
  this machine is a channel-less flake system — `<nixpkgs>` is not on `NIX_PATH`. Bare
  `nix-shell` fails with *"file 'nixpkgs' was not found in the Nix search path."* Resolved by
  invoking `nix-shell -I nixpkgs=flake:nixpkgs` (resolves nixpkgs via the flake registry). The
  system Xcode CLI-tools fallback was therefore **not needed** — the nix path works. Recorded
  because it's a real friction for reproducing the spike shell on this host.

---

## Decision

### **GO — commit Plan 06 to whisper.cpp Rust-side (Option B), with two named caveats.**

Four of the five exit criteria are cleanly met on the Mac tier; the fifth (iPhone) is unrun and
makes the GO **provisional pending a device check**. The evidence strongly favors Option B over
the staged-hybrid Option C.

| # | Criterion | Bar | Result | Verdict |
|---|-----------|-----|--------|---------|
| 1 | Feasibility | `whisper-rs` + `metal` builds & runs in nix on Apple Silicon | Built & ran; Metal engaged (M4 Max, `use gpu=1`) | **PASS** |
| 2 | Performance | RTF < 0.5 **and** WER ≤10% clean / ≤20% noisy, **same model row** | `base.en`: RTF 0.009, WER 5.8% clean / 11.7% noisy. `small.en`: RTF 0.021, WER 4.7% / 11.7% | **PASS** |
| 3 | Append-only | derivable, finalize ≤3 s, bounded dedup-able overlap | Invariant proven+tested; bounded overlap; 5 s/1 s → ≤3 s latency at 19% WER via LocalAgreement finalize | **PASS (caveat)** |
| 4 | Biasing | `initial_prompt` recall lift ≥10 pp without runaway hallucination | +10 to +19 pp (base/small); 0 hallucinations flagged | **PASS** |
| 5 | iPhone | RTF < 1.0 on device, survives 10 min sustained | **not run — no device** | **PENDING** |

**Criterion 2 is satisfied from a single model row** (as required): `base.en` clears RTF < 0.5
(0.009) *and* WER bars (5.8% clean, 11.7% noisy) simultaneously; `small.en` does too and is more
accurate. We are not stitching a fast model to a separate accurate one.

### The two caveats (the spike's real value — assumptions turned into measured cost)

1. **Live-extraction finalize needs word-level LocalAgreement, and live WER is ~4× batch.** The
   append-only stream (Plan 05 cursor contract) *is* derivable at ≤3 s latency, but only with a
   LocalAgreement-style word/token finalize — the naive segment-level rule is lossy (80% WER at
   5 s chunks vs. 19% with dedup). And even done right, live streaming WER (~19% at the ≤3 s point)
   is materially worse than end-of-session batch (~5%). **Plan 06 should treat the live stream as a
   *provisional preview* and keep end-of-session `process()` as the authoritative pass.** The
   LocalAgreement finalizer is tractable (a 30-line prototype here already recovers most content),
   but it is real Plan 06 work, not free.

2. **`initial_prompt` biasing works better than expected — but the deep decoder is still the
   ceiling.** Contrary to the survey's prediction, `initial_prompt` gave a solid, hallucination-free
   recall lift (+10 to +19 pp). This means the cheap biasing surface is **usable for v1** — the
   trie/logit-bias hotword decoder (survey §4, 19–22% lit. gains) is an **optimization, not a
   prerequisite**. That lowers Option B's near-term cost. *Caveat on the caveat:* this was measured
   on clean TTS audio with only the clip-relevant terms; a full 100-term list against real noisy
   jobsite audio is the case most likely to hallucinate and remains untested.

### Why not Option C (Apple `SpeechAnalyzer`)

The performance and accuracy headroom is large, biasing works on the Rust path *today* (Apple has
no contextual-biasing surface — the product's stated differentiator is undeliverable platform-side),
and the cross-platform payoff (Android, Deferred 3) is preserved. The only unretired risk is
on-device battery/thermal (criterion 5), which Option C would also have to prove. Nothing measured
here favors deferring the Rust path.

### Required next step before Plan 06 locks

Run the **iPhone tier** (`ios/README.md`, ~1 hr with a device): confirm `base.en`/`small.en` at
RTF < 1.0 and no thermal kill over 10 min. The Mac is a 3–5× optimistic proxy; the margins here
(RTF 0.009–0.02) are wide enough that this is expected to pass, but it is the one unretired GO
condition.

---

## Attribution

- **whisper.cpp** — MIT. Vendored by `whisper-rs-sys` as a git submodule.
- **whisper-rs** (tazz4843) `=0.16.0` — MIT. https://crates.io/crates/whisper-rs
- **whisper-rs-sys** `0.15.0` — MIT.
- **hound** `3.5.1` — MIT/Apache-2.0.
- **ggml Whisper models** — MIT (OpenAI Whisper weights). Fetched by `download-models.sh` from
  `https://huggingface.co/ggerganov/whisper.cpp` (see script note: the plan named `ggml-org`,
  which returns 401 today; ggerganov serves the same MIT weights directly):
  - `ggml-tiny.en-q5_1.bin` — q5_1, 31 MB
  - `ggml-base.en-q5_1.bin` — q5_1, 57 MB
  - `ggml-small.en-q5_1.bin` — q5_1, 182 MB
  - `ggml-large-v3-turbo-q5_0.bin` — q5_0, 548 MB
- **distil-large-v3 ggml** — MIT (HuggingFace distil-whisper).
  `https://huggingface.co/distil-whisper/distil-large-v3-ggml` → `ggml-distil-large-v3.bin`,
  1.5 GB. **Note:** this is the **f16 (unquantized)** ggml conversion — distil-whisper does not
  publish a q5_0 ggml, so Table 1's "Quant" for this row is f16, not q5_0 as the plan template
  assumed. Recorded as a deviation.
