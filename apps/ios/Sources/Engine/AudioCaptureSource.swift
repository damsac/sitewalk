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
// permissions, and interruption handling stay in Swift (exactly as the old
// SpeechSource fallback did); an AVAudioEngine tap + AVAudioConverter down-mix/resample to
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
    private let log = Logger(
        subsystem: Bundle.main.bundleIdentifier ?? "sitewalk", category: "audio"
    )
    /// Called when the mic could not be opened at all. Without this the walk
    /// would sit on a live-looking RECORDING screen capturing silence, which is
    /// worse than saying so — the operator would talk through a whole site and
    /// get nothing.
    var onUnavailable: (@Sendable () -> Void)?
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
        // Configure and ACTIVATE the session, surfacing failure instead of
        // swallowing it.
        //
        // The two `try?`s this replaces hid the cause of a launch-race abort
        // (crash report, build 93, 0.87s after process start): START WALK
        // pressed within a second of opening the app runs this before the
        // session can activate, `inputNode.outputFormat` then reports 0 Hz /
        // 0 channels, and `installTap` raises on the invalid format — which is
        // an abort(), not a catchable error.
        //
        // `.duckOthers` is also not a documented option for `.record`, so the
        // ideal configuration is tried first and a plain `.record` is the
        // fallback rather than leaving the session in whatever state a rejected
        // setCategory left behind.
        let session = AVAudioSession.sharedInstance()
        do {
            try session.setCategory(.record, mode: .measurement, options: .duckOthers)
        } catch {
            log.error("setCategory(.record, .measurement, .duckOthers) refused: \(error, privacy: .public) — retrying plain .record")
            do {
                try session.setCategory(.record, mode: .measurement)
            } catch {
                log.error("setCategory(.record, .measurement) refused too: \(error, privacy: .public)")
            }
        }

        // Activation can legitimately fail while the app is still becoming
        // active, or if another app holds the mic. Retry briefly rather than
        // proceeding into an abort — a walk is worth a few milliseconds.
        var activated = false
        for attempt in 0..<3 {
            do {
                try session.setActive(true, options: .notifyOthersOnDeactivation)
                activated = true
                break
            } catch {
                log.error("setActive attempt \(attempt + 1, privacy: .public) failed: \(error, privacy: .public)")
                // 40ms, twice — enough to clear a foreground transition without
                // making START WALK feel unresponsive.
                Thread.sleep(forTimeInterval: 0.04)
            }
        }
        guard activated else {
            log.error("audio session would not activate — no tap installed, walk will capture nothing")
            onUnavailable?()
            return
        }

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
        // A 0 Hz / 0-channel format means the session never actually activated —
        // another app holds the mic, a call is up, or `.measurement` mode was
        // refused. The two `try?`s above swallow that, and installing a tap with
        // an invalid format is another raise-and-abort. Bail loudly instead.
        guard hwFormat.sampleRate > 0, hwFormat.channelCount > 0 else {
            // THE abort from the build-93 crash report. installTap raises on an
            // invalid format, and a raise is an abort — so this guard is the
            // difference between a dead mic and a dead app.
            log.error("mic unavailable: input format \(hwFormat.sampleRate, privacy: .public) Hz, \(hwFormat.channelCount, privacy: .public) ch — not installing a tap")
            onUnavailable?()
            return
        }
        guard let converter = AVAudioConverter(from: hwFormat, to: targetFormat) else { return }

        // Capture only Sendable locals in the render-thread closure so nothing
        // hops the @MainActor boundary on the hot path.
        let target = targetFormat
        let deliver = deliveryQueue
        let push = pushSamples

        // Remove any existing tap FIRST. `installTap` on a bus that already
        // has one raises an ObjC exception, which is an abort() — not a
        // recoverable error — and it took the app down on build 93:
        //
        //   AVAudioNode installTapOnBus: → NSException → objc_exception_throw
        //   → abort(), SIGABRT, on the first thing START WALK does.
        //
        // `removeTap` on a bus with no tap is a documented no-op, so this is
        // free insurance against every path that could double up.
        input.removeTap(onBus: 0)

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
