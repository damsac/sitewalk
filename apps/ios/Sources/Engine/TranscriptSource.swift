import Foundation
import AVFoundation
import Speech

// TranscriptSource feeds TEXT to the engine (the scripted/demo path). Two
// sources share one interface:
//
//   ScriptedSource — deterministic canned walk for demos, simulator work, and
//                    screenshot verification. Also the operator-facing demo mode.
//   SpeechSource   — AVAudioEngine + SFSpeechRecognizer, on-device when available.
//
// Plan 08 note: STT for the LIVE walk (`live=1`) is now RUST-side whisper —
// `AudioCaptureSource` captures mic PCM and pushes it across FFI via
// `WalkSession.push_audio`, and the transcript arrives back as
// `WalkEvent.transcriptCommitted`. `SpeechSource` (SFSpeechRecognizer) is
// retained as a fallback and is no longer the live default (delete nothing,
// D10). `AudioCaptureSource` is intentionally NOT a `TranscriptSource` — it
// produces PCM, not text.

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

// MARK: - Live microphone source

@MainActor
final class SpeechSource: TranscriptSource {
    let chunks: AsyncStream<String>
    private let continuation: AsyncStream<String>.Continuation

    private let audioEngine = AVAudioEngine()
    private let recognizer = SFSpeechRecognizer(locale: Locale(identifier: "en-US"))
    private var request: SFSpeechAudioBufferRecognitionRequest?
    private var recognition: SFSpeechRecognitionTask?
    private var delivered = ""

    init() {
        var cont: AsyncStream<String>.Continuation!
        chunks = AsyncStream { cont = $0 }
        continuation = cont
    }

    static func requestPermissions() async -> Bool {
        let mic = await AVAudioApplication.requestRecordPermission()
        let speech = await withCheckedContinuation { cont in
            SFSpeechRecognizer.requestAuthorization { cont.resume(returning: $0 == .authorized) }
        }
        return mic && speech
    }

    func start() {
        let request = SFSpeechAudioBufferRecognitionRequest()
        request.shouldReportPartialResults = true
        if recognizer?.supportsOnDeviceRecognition == true {
            request.requiresOnDeviceRecognition = true
        }
        self.request = request

        let session = AVAudioSession.sharedInstance()
        try? session.setCategory(.record, mode: .measurement, options: .duckOthers)
        try? session.setActive(true, options: .notifyOthersOnDeactivation)

        let input = audioEngine.inputNode
        let format = input.outputFormat(forBus: 0)
        input.installTap(onBus: 0, bufferSize: 1024, format: format) { buffer, _ in
            request.append(buffer)
        }
        audioEngine.prepare()
        try? audioEngine.start()

        recognition = recognizer?.recognitionTask(with: request) { [weak self] result, _ in
            guard let self, let result else { return }
            let text = result.bestTranscription.formattedString
            // Deliver only the newly appended tail.
            Task { @MainActor in
                if text.count > self.delivered.count, text.hasPrefix(self.delivered) {
                    let tail = String(text.dropFirst(self.delivered.count))
                    self.delivered = text
                    self.continuation.yield(tail)
                } else if text != self.delivered {
                    self.delivered = text
                    self.continuation.yield(" " + text)
                }
                // Final result after a flush → close immediately rather than
                // waiting out stop()'s grace ceiling.
                if result.isFinal {
                    self.continuation.finish()
                }
            }
        }
    }

    func pause() { audioEngine.pause() }
    func resume() { try? audioEngine.start() }

    /// Flush: stop the mic but let recognition FINISH the audio it already
    /// has — `cancel()` here dropped the final utterance (issue #155; CANON:
    /// flush over speed). The stream closes when the final result lands, with
    /// a 1.5s grace ceiling so a stalled recognizer can't hang finishWalk().
    func stop() {
        audioEngine.inputNode.removeTap(onBus: 0)
        audioEngine.stop()
        request?.endAudio()
        recognition?.finish()
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(1.5))
            self?.continuation.finish()
        }
    }

    /// Drop everything in flight — discard path, nothing is kept.
    func abort() {
        audioEngine.inputNode.removeTap(onBus: 0)
        audioEngine.stop()
        request?.endAudio()
        recognition?.cancel()
        continuation.finish()
    }
}
