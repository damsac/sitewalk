import Foundation
import AVFoundation
import os

// A PCM source drives the Rust STT path: it produces 16 kHz mono f32 frames and
// hands them to an injected `pushSamples` closure. Two conformers: the live mic
// (`AudioCaptureSource`) and a bundled fixture WAV (`WavFileAudioSource`, the
// mic-free D7 test path). Same lifecycle surface as `TranscriptSource` so
// `AppModel` can pause/resume/stop either uniformly.
@MainActor
protocol PCMAudioSource: AnyObject {
    func start()
    func pause()
    func resume()
    func stop()
}

// Live mic capture for the STT-in-Rust path (Plan 08 D1). The audio session,
// permissions, and interruption handling stay in Swift (exactly as SpeechSource
// does today); an AVAudioEngine tap + AVAudioConverter down-mix/resample to
// 16 kHz mono f32 (what whisper wants — SttConfig.sample_rate), and the PCM is
// pushed across FFI OFF the render thread. Rust never touches the mic.
//
// This is deliberately NOT a `TranscriptSource`: it produces PCM, not text.
// STT now happens Rust-side (whisper), so there is no SFSpeechRecognizer here.
// The tap callback does the minimum (convert + copy) and hands samples to a
// serial background queue that calls the injected `pushSamples` closure (wired
// by the adapter to `WalkSession.pushAudio`) — the cheap enqueue, never the
// long Metal decode, runs off the render thread (D1 cadence/backpressure).
@MainActor
final class AudioCaptureSource: PCMAudioSource {
    /// Delivered 16 kHz mono f32 frames, OFF the render thread.
    private let pushSamples: @Sendable ([Float]) -> Void
    /// A/B knob (Plan 08 Task 10): when true, enable Apple's on-device voice
    /// processing (noise/echo suppression) on the input node via
    /// `setVoiceProcessingEnabled(true)`. It is an A/B knob, NOT a decided
    /// default — aggressive suppression can HURT whisper (spectral artifacts)
    /// as easily as help, so the choice is deferred to the Task 12 noise SNR
    /// eval. Sourced from the `voiceproc=1` launch arg.
    ///
    /// NOTE — the OTHER route (not this knob): the OS mic-mode "Voice
    /// Isolation" a user picks in Control Center / the AVAudioSession input
    /// mode is USER-controlled, not app-set. The app-controllable surface is
    /// `AVAudioEngine.inputNode.setVoiceProcessingEnabled`, which is what this
    /// knob toggles.
    private let voiceProcessing: Bool
    private let audioEngine = AVAudioEngine()
    /// 16 kHz mono f32 — whisper's expected input (SttConfig.sample_rate).
    private let targetFormat = AVAudioFormat(
        commonFormat: .pcmFormatFloat32,
        sampleRate: 16_000,
        channels: 1,
        interleaved: false
    )!
    /// Serial, non-render queue: the FFI enqueue (`pushSamples`) runs here, so
    /// it never blocks the audio render thread (D1).
    private let deliveryQueue = DispatchQueue(label: "studio.sitewalk.audio-delivery")

    init(pushSamples: @escaping @Sendable ([Float]) -> Void, voiceProcessing: Bool = false) {
        self.pushSamples = pushSamples
        self.voiceProcessing = voiceProcessing
    }

    /// Mic only — no Speech authorization needed now (STT is on-device whisper).
    static func requestPermissions() async -> Bool {
        await AVAudioApplication.requestRecordPermission()
    }

    func start() {
        let session = AVAudioSession.sharedInstance()
        try? session.setCategory(.record, mode: .measurement, options: .duckOthers)
        try? session.setActive(true, options: .notifyOthersOnDeactivation)

        let input = audioEngine.inputNode

        // Voice-processing A/B knob (Task 10). Toggle BEFORE reading the input
        // format: enabling voice processing can change the node's output format
        // (Apple's VPIO unit re-negotiates), so the AVAudioConverter must be
        // derived from the POST-toggle `outputFormat` or the tap would resample
        // from a stale rate. A failure to enable is non-fatal — fall back to the
        // raw path (still 16 kHz mono f32 out).
        if voiceProcessing {
            do {
                try input.setVoiceProcessingEnabled(true)
            } catch {
                Logger(subsystem: Bundle.main.bundleIdentifier ?? "sitewalk", category: "audio")
                    .error("setVoiceProcessingEnabled(true) failed, continuing raw: \(error, privacy: .public)")
            }
        }

        // Re-derive AFTER the voice-processing toggle (format may have changed).
        let hwFormat = input.outputFormat(forBus: 0)
        guard let converter = AVAudioConverter(from: hwFormat, to: targetFormat) else { return }

        // Capture only Sendable locals in the render-thread closure (mirrors
        // SpeechSource) so nothing hops the @MainActor boundary on the hot path.
        let target = targetFormat
        let deliver = deliveryQueue
        let push = pushSamples

        input.installTap(onBus: 0, bufferSize: 4096, format: hwFormat) { buffer, _ in
            // Render thread: convert to 16 kHz mono f32, copy the samples out,
            // hand off. No blocking, no FFI here.
            let ratio = target.sampleRate / hwFormat.sampleRate
            let capacity = AVAudioFrameCount(Double(buffer.frameLength) * ratio) + 1024
            guard let out = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: capacity) else { return }

            var supplied = false
            var convError: NSError?
            converter.convert(to: out, error: &convError) { _, status in
                if supplied {
                    status.pointee = .noDataNow
                    return nil
                }
                supplied = true
                status.pointee = .haveData
                return buffer
            }
            guard convError == nil, let channel = out.floatChannelData, out.frameLength > 0 else { return }
            let samples = Array(UnsafeBufferPointer(start: channel[0], count: Int(out.frameLength)))
            deliver.async { push(samples) }
        }

        audioEngine.prepare()
        try? audioEngine.start()
    }

    func pause() { audioEngine.pause() }
    func resume() { try? audioEngine.start() }

    func stop() {
        audioEngine.inputNode.removeTap(onBus: 0)
        audioEngine.stop()
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }
}
