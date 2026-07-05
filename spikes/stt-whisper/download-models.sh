#!/usr/bin/env bash
# Fetch quantized ggml Whisper models on demand into the gitignored models/ dir.
# Source: official whisper.cpp ggml repo.
# NOTE (deviation from plan): the plan named `ggml-org/whisper.cpp` as canonical and said the
# `ggerganov/whisper.cpp` URL "redirects". In practice (2026-07-04) `ggml-org/whisper.cpp`
# returns HTTP 401 (gated/unavailable) while `ggerganov/whisper.cpp` serves every file directly.
# So we pull from ggerganov. Same MIT-licensed ggml weights either way.
# NEVER commit models — they are 18 MB – 1.6 GB. See .gitignore.
#
# Usage:
#   ./download-models.sh                 # default: base.en + small.en (likely sweet spot)
#   ./download-models.sh tiny.en base.en small.en large-v3-turbo distil-large-v3
#
# License: Whisper ggml weights = MIT (OpenAI). distil-whisper = MIT (HuggingFace).
set -euo pipefail

cd "$(dirname "$0")"
mkdir -p models

# Official ggml mirror for standard + turbo models.
GGML_BASE="https://huggingface.co/ggerganov/whisper.cpp/resolve/main"
# distil-whisper ships its own ggml conversion.
DISTIL_URL="https://huggingface.co/distil-whisper/distil-large-v3-ggml/resolve/main/ggml-distil-large-v3.bin"

# model-name -> "filename url" (quantization baked into filename)
resolve() {
  case "$1" in
    tiny.en)          echo "ggml-tiny.en-q5_1.bin        $GGML_BASE/ggml-tiny.en-q5_1.bin" ;;
    base.en)          echo "ggml-base.en-q5_1.bin        $GGML_BASE/ggml-base.en-q5_1.bin" ;;
    small.en)         echo "ggml-small.en-q5_1.bin       $GGML_BASE/ggml-small.en-q5_1.bin" ;;
    large-v3-turbo)   echo "ggml-large-v3-turbo-q5_0.bin $GGML_BASE/ggml-large-v3-turbo-q5_0.bin" ;;
    distil-large-v3)  echo "ggml-distil-large-v3.bin     $DISTIL_URL" ;;
    *) echo "ERROR: unknown model '$1'" >&2; return 1 ;;
  esac
}

MODELS=("$@")
if [ ${#MODELS[@]} -eq 0 ]; then
  MODELS=(base.en small.en)
fi

for m in "${MODELS[@]}"; do
  read -r fname url < <(resolve "$m")
  dest="models/$fname"
  if [ -f "$dest" ]; then
    echo "== $m already present: $dest ($(du -h "$dest" | cut -f1))"
    continue
  fi
  echo "== downloading $m -> $dest"
  curl -L --fail --progress-bar -o "$dest.part" "$url"
  mv "$dest.part" "$dest"
  echo "   done: $(du -h "$dest" | cut -f1)"
done

echo
echo "Models in models/:"
ls -lh models/ | awk 'NR>1 {print "  " $5 "\t" $9}'
