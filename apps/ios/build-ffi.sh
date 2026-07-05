#!/usr/bin/env bash
# Regenerates apps/ios/Packages/MurmurCoreFFI's binary artifacts:
#   - crates/ffi built for aarch64-apple-ios-sim and aarch64-apple-ios
#   - Swift bindings generated via the uniffi-bindgen dev binary
#   - Frameworks/ffiFFI.xcframework (device + sim slices)
#
# These artifacts are GITIGNORED (large binaries); Sources/MurmurCoreFFI/ffi.swift
# and Package.swift ARE committed. Run this after any crates/ffi surface change,
# or any time Frameworks/ffiFFI.xcframework is missing.
#
# Why this isn't `nix develop -c cargo build --target ...` alone (Plan 07
# Task 9 / flake.nix multi-target toolchain): the nix-wrapped clang/cc-wrapper
# hardcodes macOS SDK paths and a `-mmacos-version-min` flag that conflicts
# with `-target arm64-apple-ios...-simulator`, and its default library search
# resolves against the MacOSX SDK's libiconv.tbd even when --target is iOS,
# which fails at the final link step ("building for iOS Simulator, but
# linking in .tbd built for macOS/Mac Catalyst"). The fix used here: keep the
# nix devShell's rust-overlay toolchain (cargo/rustc/clippy — unchanged, still
# the single source for host builds and `cargo test --workspace`), but for the
# iOS cross builds only, point CC/AR/the linker at the *system* Xcode
# toolchain (`/usr/bin/clang`, `/usr/bin/ar`) and SDKROOT at the real
# iphoneos/iphonesimulator SDK via `xcrun`. This is the "system Xcode
# fallback" path referenced in the Plan 07 Task 9 report — the pure-nix path
# (unset SDKROOT/NIX_*FLAGS only) still fails on SDK linkage.
set -euo pipefail

cd "$(dirname "$0")/../.."   # repo root
FFI_DIR="apps/ios/Packages/MurmurCoreFFI"
BINDINGS_DIR="$(mktemp -d)"
trap 'rm -rf "$BINDINGS_DIR"' EXIT

echo "==> building crates/ffi for aarch64-apple-ios-sim"
nix develop -c bash -c '
  set -euo pipefail
  export DEVELOPER_DIR=/Applications/Xcode-26.2.0.app/Contents/Developer
  export SDKROOT=$(/usr/bin/xcrun --sdk iphonesimulator --show-sdk-path)
  # Match the app deployment target (project.yml: iOS 17.0). Without this, rustc
  # links the cdylib probe at its default min (arm64-apple-ios10.0), and the
  # whisper.cpp objects that cmake built against the real iOS SDK min fail to
  # link with a missing ___chkstk_darwin symbol for architecture arm64.
  export IPHONEOS_DEPLOYMENT_TARGET=17.0
  export CC_aarch64_apple_ios_sim=/usr/bin/clang
  export CXX_aarch64_apple_ios_sim=/usr/bin/clang++
  export AR_aarch64_apple_ios_sim=/usr/bin/ar
  export CARGO_TARGET_AARCH64_APPLE_IOS_SIM_LINKER=/usr/bin/clang
  unset NIX_CFLAGS_COMPILE NIX_LDFLAGS NIX_CFLAGS_COMPILE_FOR_BUILD NIX_LDFLAGS_FOR_BUILD
  # --features whisper: pulls whisper-rs + vendored whisper.cpp + Metal (Plan 08
  # Task 8). Needs cmake/clang (dev shell, Plan 06 Task 1). The vendored
  # whisper.cpp Metal shaders compile against the iphonesimulator SDK via the
  # SDKROOT/system-clang cross-link set above. `cargo test --workspace` never
  # sees this feature — it stays hermetic (no model, no cmake, no Metal).
  cargo build -p ffi --release --target aarch64-apple-ios-sim --features whisper
'

echo "==> building crates/ffi for aarch64-apple-ios (device)"
nix develop -c bash -c '
  set -euo pipefail
  export DEVELOPER_DIR=/Applications/Xcode-26.2.0.app/Contents/Developer
  export SDKROOT=$(/usr/bin/xcrun --sdk iphoneos --show-sdk-path)
  # Match the app deployment target (project.yml: iOS 17.0) — see the sim
  # invocation above; without it the device cdylib link fails on a missing
  # ___chkstk_darwin symbol for architecture arm64.
  export IPHONEOS_DEPLOYMENT_TARGET=17.0
  export CC_aarch64_apple_ios=/usr/bin/clang
  export CXX_aarch64_apple_ios=/usr/bin/clang++
  export AR_aarch64_apple_ios=/usr/bin/ar
  export CARGO_TARGET_AARCH64_APPLE_IOS_LINKER=/usr/bin/clang
  unset NIX_CFLAGS_COMPILE NIX_LDFLAGS NIX_CFLAGS_COMPILE_FOR_BUILD NIX_LDFLAGS_FOR_BUILD
  # --features whisper (device slice) — see the sim invocation above.
  cargo build -p ffi --release --target aarch64-apple-ios --features whisper
'

echo "==> generating Swift bindings (uniffi-bindgen, host build)"
nix develop -c cargo run -p ffi --features uniffi-bindgen-cli --bin uniffi-bindgen -- \
  generate --library target/aarch64-apple-ios-sim/release/libffi.a \
  --language swift --out-dir "$BINDINGS_DIR"

cp "$BINDINGS_DIR/ffi.swift" "$FFI_DIR/Sources/MurmurCoreFFI/ffi.swift"

echo "==> assembling ffiFFI.xcframework"
rm -rf "$FFI_DIR/Frameworks/ffiFFI.xcframework"
for slice in sim device; do
  hdir="$BINDINGS_DIR/headers-$slice"
  mkdir -p "$hdir"
  cp "$BINDINGS_DIR/ffiFFI.h" "$hdir/"
  cp "$BINDINGS_DIR/ffiFFI.modulemap" "$hdir/module.modulemap"
done

xcodebuild -create-xcframework \
  -library target/aarch64-apple-ios-sim/release/libffi.a -headers "$BINDINGS_DIR/headers-sim" \
  -library target/aarch64-apple-ios/release/libffi.a -headers "$BINDINGS_DIR/headers-device" \
  -output "$FFI_DIR/Frameworks/ffiFFI.xcframework"

# ---------------------------------------------------------------------------
# Whisper model provisioning (Plan 08 D5/Task 8)
# ---------------------------------------------------------------------------
# The FFI libs are now built WITH the `whisper` feature, so the app can run
# on-device STT. It needs the GGML model bundled as an app resource:
#
#   ggml-base.en-q5_1.bin  (~60 MB, MIT, huggingface.co/ggerganov/whisper.cpp)
#
# This binary is GITIGNORED (large — like the xcframework). Fetch it into the
# package resources once:
#
#   mkdir -p apps/ios/Packages/MurmurCoreFFI/Resources
#   curl -L -o apps/ios/Packages/MurmurCoreFFI/Resources/ggml-base.en-q5_1.bin \
#     https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en-q5_1.bin
#
# The REAL-core build spec (project-real.yml + the gitignored project.local.yml)
# bundles `Resources/ggml-base.en-q5_1.bin` into the app target's resources so
# `Bundle.main.path(forResource: "ggml-base.en-q5_1", ofType: "bin")` resolves
# at runtime (GalleryApp.resolveEngine). If the model is absent the live walk
# degrades to text-only — no crash (the Rust side treats a nil path as
# text-only). Keep CODE_SIGNING_ALLOWED: NO for the simulator.
#
# NOTE: the tracked demo spec (project.yml) deliberately does NOT bundle the
# model — a clean checkout must build the scripted DemoWalkEngine app from that
# file alone (no ~60 MB gitignored dependency). The model rides the real build.

echo "==> done. Run 'cd apps/ios && xcodegen generate' next."
