#!/usr/bin/env bash
# Fails if the committed Swift bindings (Sources/MurmurCoreFFI/ffi.swift) are
# stale relative to the current crates/ffi Rust source.
#
# Unlike build-ffi.sh, this does NOT cross-compile for iOS. uniffi's
# proc-macro mode (uniffi::setup_scaffolding!) embeds the interface metadata
# that uniffi-bindgen reads directly into the compiled library at a fixed
# link-time section, regardless of target platform — the Swift *source* that
# comes out is a function of the Rust interface surface only, not of the host
# triple. So a plain host build (`cargo build -p ffi --release`, whisper
# feature OFF — same hermetic/no-model build already exercised by
# `cargo test --workspace`) is enough to regenerate ffi.swift and compare it
# byte-for-byte against the committed copy. This runs on Linux CI (cheap) as
# well as macOS dev machines; see the CI workflow's "FFI bindings drift" job.
set -euo pipefail

cd "$(dirname "$0")/../.."   # repo root
FFI_SWIFT="apps/ios/Packages/MurmurCoreFFI/Sources/MurmurCoreFFI/ffi.swift"
OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

echo "==> building crates/ffi (host, release, no whisper feature)"
nix develop -c cargo build -p ffi --release

LIBDIR="target/release"
if [ -f "$LIBDIR/libffi.dylib" ]; then
  LIB="$LIBDIR/libffi.dylib"
elif [ -f "$LIBDIR/libffi.so" ]; then
  LIB="$LIBDIR/libffi.so"
else
  echo "error: no libffi.{dylib,so} found in $LIBDIR after build" >&2
  exit 1
fi

echo "==> generating Swift bindings from $LIB"
nix develop -c cargo run -p ffi --features uniffi-bindgen-cli --bin uniffi-bindgen -- \
  generate --library "$LIB" --language swift --out-dir "$OUT_DIR"

if ! diff -u "$FFI_SWIFT" "$OUT_DIR/ffi.swift" > "$OUT_DIR/diff.txt"; then
  echo ""
  echo "error: committed Swift bindings are STALE." >&2
  echo "The Rust FFI surface (crates/ffi) has changed but $FFI_SWIFT was not regenerated." >&2
  echo "Fix: run ./apps/ios/build-ffi.sh and commit the updated ffi.swift." >&2
  echo ""
  echo "diff (committed vs freshly generated):" >&2
  cat "$OUT_DIR/diff.txt" >&2
  exit 1
fi

echo "==> ffi.swift is up to date."
