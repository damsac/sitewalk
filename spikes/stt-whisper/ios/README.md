# iPhone reality-check tier (Table 4) — PENDING, hardware-gated

**Status: NOT RUN.** This tier needs dam's physical iPhone; the spike worker only ran the Mac
tiers (T0–T4). The iOS **simulator is explicitly insufficient** for the numbers that matter here
(no Metal/ANE, no real battery/thermal — CPU-only). This file is the recipe so dam can fill
Table 4 in an hour with a device.

## Why this path (whisper.cpp's own example app, NOT UniFFI)

The iPhone tier's only job is **battery / thermal / RTF on real silicon**. whisper.cpp ships a
working SwiftUI app that does exactly that with near-zero code. UniFFI (wrapping our Rust crate
behind a Swift binding) is **Plan 07** work — the FFI streaming boundary and the
`&mut self`→actor wrapper (Plan 05 Deferred 3) are real integration cost we are deliberately
*not* front-loading into a spike. A spike answers "can the engine run acceptably on the phone,"
not "is our binding architecture right."

## Recipe (path B)

```sh
git clone https://github.com/ggml-org/whisper.cpp     # (ggerganov/whisper.cpp also works)
cd whisper.cpp
./build-xcframework.sh                                 # produces whisper.xcframework (Metal/CoreML)
open examples/whisper.swiftui/whisper.swiftui.xcodeproj
# In the app: add a quantized model (base.en or small.en q5_1 — same files download-models.sh pulls),
# set the signing team, build to the device (not the simulator).
```

## What to measure (fill Table 4 in RESULTS.md)

1. **RTF** — decode a ~60 s clip; the app logs decode time. RTF = decode_secs / 60. Target < 1.0.
   (Mac showed 0.006–0.04; expect 3–5× slower + thermal-limited on iPhone — still likely < 0.2.)
2. **Battery Δ** — run a ~10 min sustained decode loop; note battery % before/after.
3. **Thermal state** — log `ProcessInfo.processInfo.thermalState` at the 10 min mark
   (nominal / fair / serious / critical).
4. **Background kill** — background/lock the app mid-loop; does iOS kill it? (The survey's biggest
   open question for hour-long locked capture.)

## Exit-criterion 5

The chosen model should be **real-time-capable (RTF < 1.0)** on device and survive ~10 min
sustained without a thermal kill. Until this runs, the GO verdict is **provisional pending a
device check** (see RESULTS.md Decision).

## Deferred (Plan 07, not here)

Real iOS integration = UniFFI surface over the Rust core + background-audio session + chunked
live feed + the LocalAgreement finalize (see Table 2). This throwaway app proves only the engine.
