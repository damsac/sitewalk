# Sitewalk iOS

SwiftUI app. Complete interactive flow on a demo engine; waiting on the FFI
bridge to `murmur-core` — see `../../docs/HANDOFF-ios-ffi.md`.

```sh
xcodegen generate    # .xcodeproj is generated, never committed
xcodebuild -project SitewalkGallery.xcodeproj -scheme SitewalkGallery \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' build
```

Design source of truth: `../../docs/design/BRIEF.md` (rationale) and
`../../docs/design/mockup.html` (visual reference, open in a browser).

Launch args: `autoflow=1` (scripted walk plays itself), `autopdf=1` (+ PDF),
`live=1` (real mic + on-device STT), `screen=<page>` (design gallery).
