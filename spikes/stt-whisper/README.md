# STT whisper.cpp Rust-side spike

**Disposable, quarantined measurement spike.** This is NOT a workspace member (see
`exclude = ["spikes"]` in the repo-root `Cargo.toml`). Nothing here gets wired into
`crates/`. The deliverable is [`RESULTS.md`](./RESULTS.md) — a GO/KILL decision document.

See the plan: `docs/plans/2026-07-04-rust-core-06-spike-stt-benchmark.md`.

## Setup

```sh
cd spikes/stt-whisper
nix-shell                    # cmake + clang + libclang for whisper.cpp native build
./download-models.sh         # fetch base.en + small.en (default) into gitignored models/
```

If the nix build of `whisper-rs` fails, the plan's mandatory fallback is the system
Xcode command-line toolchain:

```sh
export DEVELOPER_DIR=$(xcode-select -p)
export SDKROOT=$(xcrun --sdk macosx --show-sdk-path)
cargo build                  # no nix shell
```

## Experiments (CLI subcommands)

```sh
# Table 1 — feasibility + RTF + peak memory
cargo run --release -- bench --model models/ggml-base.en-q5_1.bin --audio audio/clip.wav

# Table 2 — chunked pseudo-streaming + append-only finalize
cargo run --release -- stream --model models/ggml-base.en-q5_1.bin --audio audio/clip.wav --chunk 5 --overlap 1

# Table 3 — WER
cargo run --release -- accuracy --model models/ggml-base.en-q5_1.bin --audio audio/clip.wav --reference audio/references/clip.txt

# Table 3 — initial_prompt biasing
cargo run --release -- bias --model models/ggml-base.en-q5_1.bin --audio audio/clip.wav \
    --reference audio/references/clip.txt --terms audio/references/terms.txt
```

## Layout

```
Cargo.toml         standalone package; whisper-rs with metal feature
shell.nix          native toolchain for whisper.cpp build
download-models.sh fetch ggml models at run time (models/ gitignored)
src/main.rs        CLI dispatch: bench | stream | accuracy | bias
src/bench.rs       load/RTF/peak-RSS harness
src/stream.rs      chunked decode + overlap/finalize measurement
src/wer.rs         WER + target-term recall (unit-tested)
audio/scripts/     committed read-aloud jargon scripts (text)
audio/references/  committed hand transcripts (ground truth) + term lists
ios/README.md      optional real-device tier notes (Table 4)
```
