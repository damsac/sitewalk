# iOS ↔ Core Handoff — the FFI Bridge

**For: dam. From: sac.** The iOS app in `apps/ios/` is complete up to one seam:
a Swift protocol that stands exactly where the Rust core belongs. This doc is
the map of that seam. Implement the bridge behind the protocol and the app is
live — no UI changes needed.

## What's already built (UI side, all verified on simulator)

- **Design system** — `Sources/Theme`, `Sources/Components`: Field Instrument
  tokens (paper/ink/safety-orange), bundled brand fonts, every component from
  the design study (`docs/design/mockup.html`), including document edit/gap
  states. Design rationale: `docs/design/BRIEF.md`; product scope: `docs/CORE.md`.
- **Full interactive flow** — `Sources/Flow`: board → walk (pause/resume,
  photo pins, live board) → build beat → interactive review (tap-to-edit
  amounts, gap filling, recomputed totals) → PDF export + share → job marked
  sent. Runs end-to-end today on the demo engine.
- **Speech-to-text** — `Sources/Engine/TranscriptSource.swift`: on-device
  SFSpeechRecognizer (`live=1` launch arg) or deterministic scripted source.
  STT deliberately stays in Swift: the engine only ever receives text.
- **PDF export** — `Sources/Flow/DocumentPDF.swift`: US Letter render of the
  same document components ("one schema, many renderings").

## The seam

`Sources/Engine/WalkEngine.swift`:

```swift
@MainActor protocol WalkEngine: AnyObject {
    var events: AsyncStream<WalkEvent> { get }   // .itemCaptured(...)
    func begin(trade: TradeFixture)
    func append(transcript: String)
    func finish() async -> DocumentModel
}
```

`DemoWalkEngine.swift` is the placeholder implementation (keyword matcher).
**The single swap point** is `AppModel.init(engine:)` — construct the bridge,
pass it in, delete nothing else.

## Protocol ↔ murmur-core mapping

| Swift (UI expects) | murmur-core (already exists) |
|---|---|
| `begin(trade:)` | `store.start_session(job_id)` (+ template key for the session) |
| `append(transcript:)` | `store.append_transcript` + `LiveExtractor` incremental pass |
| `events` → `.itemCaptured` | items the live pass lands (map `CapturedItem` → tag kind/label, text, right, photo count) |
| `finish()` | `end_and_record_session` + `SessionProcessor.process` → `Artifact` → `DocumentModel` rows |
| *(future)* gap fill via voice | multi-turn correction pass |
| *(future)* price hints | `harness` memory / reflection output |

Threading: everything UI-facing is `@MainActor`; deliver events on main.
Suggested bridge: UniFFI on `murmur-core` (RMP-style, like sapling), with an
adapter turning the Rust callback/stream into `AsyncStream<WalkEvent>`.

## Behavior contracts the UI already promises (from docs/CORE.md)

1. `finish()` must resolve in **< 8 s** — the build beat animation is timed
   around that; no spinner exists, by design.
2. Rows the engine isn't sure about come back with `isGap = true` and a `——`
   amount — **never a guessed value**. The UI renders and resolves gaps.
3. Every walk works offline: if the LLM pass can't run, `append` must still be
   safe to call and `finish()` should degrade (e.g. queue + partial document),
   not fail. Capture never loses audio — STT is on-device.

## Run it

```sh
cd apps/ios
xcodegen generate
xcodebuild -project SitewalkGallery.xcodeproj -scheme SitewalkGallery \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' build
```

Launch args: *(none)* = app flow on demo engine · `autoflow=1` = scripted walk
plays itself end-to-end · `autoflow=1 autopdf=1` = also renders the PDF ·
`live=1` = real mic + on-device STT · `screen=components|jobs|capture|document`
= static design gallery.

## Also worth upstreaming (currently uncommitted on sac's machine)

Two small dev-ergonomics patches used to run the `walk` example against PPQ:
`AnthropicProvider` additionally sends `Authorization: Bearer <key>`, and the
walk example honors `ANTHROPIC_BASE_URL` (PPQ exposes an Anthropic-compatible
`/v1/messages`; Bearer auth only). Recommend adding both properly in harness.

## Open questions for the bridge design

1. Event cadence: per-item callbacks vs. batched per live pass?
2. Where do document numbers (EST-0047) get minted — core or UI?
3. Photo attachment: UI pins photos to items locally today; when does the core
   learn about them (sync schema)?
4. Template keys: UI uses `landscape | property | inspection` — align with
   core's template naming before it ossifies.
