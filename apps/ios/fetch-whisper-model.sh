#!/usr/bin/env bash
# Downloads a ggml whisper model into apps/ios/Sources/Resources/ as an
# APP-TARGET resource (Bundle.main — see GalleryApp.resolveEngine and
# build-ffi.sh's Whisper model provisioning note for why it must live there
# and not under a SwiftPM package's Resources).
#
# Usage:
#   ./fetch-whisper-model.sh [base.en|small.en] [--force]
#   MODEL=base.en ./fetch-whisper-model.sh          # env var form
#
# Default model: small.en (Plan 08 spike RESULTS.md: strictly better than
# base.en on every measured axis — 4.7% vs 5.8% WER clean, 2-4pp better under
# noise, 0 vs 4 hallucinated tokens on jackhammer noise-only audio — at ~free
# RTF headroom on the Mac proxy). See the CAVEAT below before trusting this on
# a device.
#
# Both models' sha256 digests are hardcoded below (not just the default's) so
# reverting the *download* to base.en never needs a source change beyond the
# one launch arg documented in GalleryApp.swift (`sttmodel=base.en`) — this
# script fetches whichever model you ask it for, verifying against a pinned
# digest either way.
#
# CAVEAT (carried into the PR description, not just here): the small.en
# promotion is decided on Mac-proxy RTF numbers only (spike RESULTS.md Table
# 1: RTF 0.021 on an M4 Max). The iPhone T5 device tier in that same doc is
# still marked PENDING — small.en's on-device RTF is UNPROVEN. That is why
# this is a one-arg revert (`sttmodel=base.en`) and not a hard swap: if a
# device sweep shows small.en missing the RTF<1.0 bar, flip the arg back
# without touching this script or GalleryApp.swift's resolution logic.
#
# Caching: if the target file exists and its sha256 already matches the
# pinned digest, the download is skipped. --force re-fetches regardless.
# On a digest mismatch (corrupt/partial download, or an upstream file change)
# the bad file is deleted and the script fails loudly — it never leaves a
# silently-wrong model behind for whisper.cpp to load.
set -euo pipefail
cd "$(dirname "$0")"   # apps/ios
DEST_DIR="Sources/Resources"
BASE_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main"

MODEL="${MODEL:-small.en}"
FORCE=0
for arg in "$@"; do
  case "$arg" in
    base.en|small.en) MODEL="$arg" ;;
    --force) FORCE=1 ;;
    *)
      echo "usage: $0 [base.en|small.en] [--force]" >&2
      exit 1
      ;;
  esac
done

# Pinned sha256 digests (verified against a real download of both files,
# 2026-07 — see the PR that introduced this script). Keep BOTH here even
# though only one is fetched by default: the whole point of the revert path
# is that base.en's digest is always available, no source change needed.
case "$MODEL" in
  base.en)
    FILENAME="ggml-base.en-q5_1.bin"
    SHA256="4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f"
    ;;
  small.en)
    FILENAME="ggml-small.en-q5_1.bin"
    SHA256="bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30"
    ;;
  *)
    echo "unknown model '$MODEL' (expected base.en or small.en)" >&2
    exit 1
    ;;
esac

DEST="$DEST_DIR/$FILENAME"
mkdir -p "$DEST_DIR"

sha256_of() {
  # shasum ships on macOS by default; sha256sum on most Linux dev boxes.
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    sha256sum "$1" | cut -d' ' -f1
  fi
}

if [ -f "$DEST" ] && [ "$FORCE" -ne 1 ]; then
  EXISTING_SHA="$(sha256_of "$DEST")"
  if [ "$EXISTING_SHA" = "$SHA256" ]; then
    echo "==> $DEST already present and verified (sha256 matches) — skipping download."
    exit 0
  fi
  echo "NOTE: $DEST exists but its sha256 doesn't match the pinned digest — re-fetching." >&2
fi

echo "==> downloading $FILENAME (~$([ "$MODEL" = small.en ] && echo 190 || echo 60) MB) from $BASE_URL"
curl -L --fail -o "$DEST.part" "$BASE_URL/$FILENAME"

ACTUAL_SHA="$(sha256_of "$DEST.part")"
if [ "$ACTUAL_SHA" != "$SHA256" ]; then
  echo "ERROR: sha256 mismatch for $FILENAME" >&2
  echo "       expected $SHA256" >&2
  echo "       got      $ACTUAL_SHA" >&2
  rm -f "$DEST.part"
  exit 1
fi

mv "$DEST.part" "$DEST"
echo "==> $DEST downloaded and verified (sha256 matches)."
