import Foundation
import AVFoundation
import Speech

// Speech-to-text stays in Swift (on-device, free, works offline) — the engine
// only ever sees text. Two sources share one interface:
//
//   ScriptedSource — deterministic canned walk for demos, simulator work, and
//                    screenshot verification. Also the operator-facing demo mode.
//   SpeechSource   — AVAudioEngine + SFSpeechRecognizer, on-device when available.

@MainActor
protocol TranscriptSource: AnyObject {
    /// Stream of transcript increments (words/phrases as they arrive).
    var chunks: AsyncStream<String> { get }
    func start()
    func pause()
    func resume()
    func stop()
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
                try? await Task.sleep(for: .milliseconds(260))
            }
            self?.continuation.finish()
        }
    }

    func pause() { paused = true }
    func resume() { paused = false }
    func stop() {
        task?.cancel()
        continuation.finish()
    }
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
            }
        }
    }

    func pause() { audioEngine.pause() }
    func resume() { try? audioEngine.start() }

    func stop() {
        audioEngine.inputNode.removeTap(onBus: 0)
        audioEngine.stop()
        request?.endAudio()
        recognition?.cancel()
        continuation.finish()
    }
}
