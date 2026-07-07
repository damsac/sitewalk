# Sitewalk iOS

SwiftUI app. Runs the scripted `DemoWalkEngine` out of the box, and upgrades to
the real `murmur-core` engine (via the `crates/ffi` UniFFI bridge) once the
xcframework is built and an API key is present — see `GalleryApp.resolveEngine`.

## Demo build (clean checkout, zero setup)

```sh
xcodegen generate    # uses project.yml — no MurmurCoreFFI dependency
xcodebuild -project SitewalkGallery.xcodeproj -scheme SitewalkGallery \
  -destination 'platform=iOS Simulator,name=iPhone 17' build
```

A fresh clone builds and runs on the demo engine with no dependencies: the base
`project.yml` doesn't reference the (gitignored) MurmurCoreFFI xcframework, so
`#if canImport(MurmurCoreFFI)` is false and the real-core code compiles out.

## Real-core build (one extra step)

```sh
./build-ffi.sh       # regenerates the gitignored MurmurCoreFFI xcframework +
                     # Swift bindings (slow first run). Only needed when the
                     # xcframework is missing or crates/ffi changed.
./generate.sh        # detects the xcframework, injects the API key, and
                     # generates from project-real.yml (real MurmurCoreFFI dep)
xcodebuild -project SitewalkGallery.xcodeproj -scheme SitewalkGallery \
  -destination 'platform=iOS Simulator,name=iPhone 17' build
```

`./generate.sh` writes the gitignored `project.local.yml` (from
`project.local.yml.template`) with `PPQ_API_KEY` pulled from the repo-root
`.env`'s `ANTHROPIC_API_KEY`, then runs `xcodegen generate --spec
project-real.yml` (which merges `project.yml` + `project.local.yml` + the
MurmurCoreFFI package dependency). If the xcframework is absent it falls back to
the demo build and tells you to run `./build-ffi.sh`. The key flows only into the
gitignored `project.local.yml` and the generated (gitignored) `.xcodeproj`;
xcodebuild expands `$(PPQ_API_KEY)` into the built app's Info.plist at build
time. No tracked file ever holds the secret.

`./generate.sh` also fetches the on-device whisper model via
`./fetch-whisper-model.sh` (gitignored binary, sha256-verified, cached after
the first fetch): default **small.en** (~190 MB), with a one-arg revert to
**base.en** (~60 MB) via `STT_MODEL=base.en ./generate.sh` or the runtime
`sttmodel=base.en` launch arg. small.en's promotion is Mac-proxy evidence only
— see `fetch-whisper-model.sh`'s header and `spikes/stt-whisper/RESULTS.md`
(iPhone T5 device tier still PENDING).

> Switching modes in an existing checkout can leave a stale SwiftPM package graph
> in DerivedData — if a build errors with `Unable to find module dependency:
> 'ffiFFI'`, delete DerivedData and rebuild. A clean checkout is unaffected.

Confirm which engine is live from the console (no key is ever logged):

```sh
xcrun simctl spawn booted log show --last 2m \
  --predicate 'subsystem == "com.damsac.sitewalk"' --info | grep engine=
# engine=real (murmur-core MurmurEngine, key len=...)   <- real core active
```

Design source of truth: `../../docs/design/BRIEF.md` (rationale) and
`../../docs/design/mockup.html` (visual reference, open in a browser).

Launch args: `autoflow=1` (scripted walk plays itself), `autopdf=1` (+ PDF),
`live=1` (real mic + on-device STT), `screen=<page>` (design gallery),
`sttmodel=base.en|small.en` (which bundled whisper model to load; default
small.en).
