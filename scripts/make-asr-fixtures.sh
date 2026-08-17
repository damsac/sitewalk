#!/usr/bin/env bash
# Generates the speech-accuracy corpus audio from the manifest's reference
# text, using macOS `say` + `afconvert`.
#
# WHAT THIS MEASURES, AND WHAT IT DOES NOT.
#
# Synthetic speech is clean, evenly paced, and pronounces trade terms the way
# a dictionary does. It exercises the decode path, the vocabulary biasing and
# the streaming finalizer, and it is reproducible on any Mac with no recording
# session — which is why the corpus can exist today rather than after someone
# finds an afternoon.
#
# It says NOTHING about the conditions this app actually runs in: wind across
# a phone mic, a mower fifty feet away, a operator talking while walking away
# from the handset, an accent the model was not trained on. Those are what
# real recordings are for, and they are the corpus that matters most.
#
# Real recordings drop into this directory under the same filenames and take
# precedence: this script SKIPS any file that already exists. Delete a file to
# regenerate it synthetically; add a real one to replace it permanently.
#
# Usage: scripts/make-asr-fixtures.sh [voice]
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/crates/evals/fixtures/asr"
MANIFEST="$DIR/manifest.json"
VOICE="${1:-Samantha}"

command -v say >/dev/null || { echo "needs macOS \`say\`"; exit 1; }
command -v afconvert >/dev/null || { echo "needs macOS \`afconvert\`"; exit 1; }
[ -f "$MANIFEST" ] || { echo "no manifest at $MANIFEST"; exit 1; }

# id<TAB>audio<TAB>reference, one case per line.
python3 - "$MANIFEST" <<'PY' | while IFS=$'\t' read -r id audio reference; do
import json, sys
for c in json.load(open(sys.argv[1]))["cases"]:
    print("\t".join([c["id"], c["audio"], c["reference"]]))
PY
    out="$DIR/$audio"
    if [ -f "$out" ]; then
        echo "  keep  $audio (already present — a real recording is never overwritten)"
        continue
    fi
    tmp="$(mktemp -t asrfix).aiff"
    say -v "$VOICE" -o "$tmp" "$reference"
    # 16 kHz mono 16-bit PCM: what SttConfig.sample_rate expects.
    afconvert -f WAVE -d LEI16@16000 -c 1 "$tmp" "$out"
    rm -f "$tmp"
    echo "  made  $audio ($id)"
done

echo
echo "Corpus at $DIR"
echo "Run:  MURMUR_WHISPER_MODEL=/path/to/ggml-small.en-q5_1.bin \\"
echo "        cargo test -p evals --features stt-eval --test stt_accuracy -- --ignored --nocapture"
