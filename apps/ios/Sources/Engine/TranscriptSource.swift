import Foundation

// TranscriptSource feeds TEXT to the engine (the scripted/demo path):
//
//   ScriptedSource — deterministic canned walk for demos, simulator work, and
//                    screenshot verification. Also the operator-facing demo mode.
//
// Plan 08 note: STT for the LIVE walk (`live=1`) is RUST-side whisper —
// `AudioCaptureSource` captures mic PCM and pushes it across FFI via
// `WalkSession.push_audio`, and the transcript arrives back as
// `WalkEvent.transcriptCommitted`. `SpeechSource` (the SFSpeechRecognizer
// fallback retained under Plan 08 D10 "delete nothing") was never constructed
// anywhere and was deleted — together with the NSSpeechRecognitionUsageDescription
// plist key — per dam's 2026-07-14 ruling. `AudioCaptureSource` is
// intentionally NOT a `TranscriptSource` — it produces PCM, not text.

@MainActor
protocol TranscriptSource: AnyObject {
    /// Stream of transcript increments (words/phrases as they arrive).
    var chunks: AsyncStream<String> { get }
    func start()
    func pause()
    func resume()
    /// Graceful end: FLUSH any in-flight transcription, then finish the
    /// stream. Used on DONE — the last words of a walk are often the price
    /// (CANON: flush over speed). May finish the stream up to ~1.5s late.
    func stop()
    /// Hard end: drop everything in flight and finish the stream NOW.
    /// Used on discard — nothing is kept, latency wins.
    func abort()
}

// MARK: - Scripted (demo) source

@MainActor
final class ScriptedSource: TranscriptSource {
    let chunks: AsyncStream<String>
    private let continuation: AsyncStream<String>.Continuation
    private let script: [String]
    private var task: Task<Void, Never>?
    private var index = 0
    private var paused = false

    init(trade: TradeFixture) {
        var cont: AsyncStream<String>.Continuation!
        chunks = AsyncStream { cont = $0 }
        continuation = cont
        // Feed the fixture transcript in word groups, walk-and-talk pace.
        script = trade.transcript
            .split(separator: " ")
            .map(String.init)
    }

    func start() {
        task = Task { [weak self] in
            while let self, self.index < self.script.count {
                if !self.paused {
                    self.continuation.yield(self.script[self.index] + " ")
                    self.index += 1
                }
                // Break on cancellation — `try?` alone swallows the thrown
                // CancellationError and, with sleep returning instantly once
                // cancelled, the paused loop becomes a main-actor busy-spin
                // (issue #155).
                do {
                    try await Task.sleep(for: .milliseconds(260))
                } catch {
                    break
                }
            }
            self?.continuation.finish()
        }
    }

    func pause() { paused = true }
    func resume() { paused = false }
    // Scripted text has nothing in flight to flush: both ends are immediate.
    func stop() {
        task?.cancel()
        continuation.finish()
    }
    func abort() { stop() }
}
