import Foundation
import UIKit
import os
#if canImport(MurmurCoreFFI)
import MurmurCoreFFI
#endif

// Key-free breadcrumb so you can confirm from the console which engine is live
// (real murmur-core vs. the scripted demo) without ever logging the API key.
let engineLog = Logger(subsystem: "com.damsac.sitewalk", category: "engine")

// Default launch: the real app flow (board → walk → build → review → sent),
// running on DemoWalkEngine + ScriptedSource until the FFI bridge lands.
//
// Launch arguments (used by design QA and screenshot automation):
//   screen=components|jobs|capture|document  → static design gallery pages
//   live=1 / live=0                          → force mic+STT / scripted (default: live on device, scripted on sim)
//   autoflow=1                               → auto-starts a scripted walk and
//                                              auto-finishes it (state machine demo)
//   demo=1                                   → force DemoWalkEngine even if a
//                                              key is configured (D10)
//   sttgpu=0|1                               → force whisper CPU / GPU (Metal)
//   sttvad=<float>                           → STT per-window RMS pre-gate
//                                              (default 0.0; ~0.01 cuts noise)
//   sttnsp=<float>                           → STT no_speech_prob drop
//                                              threshold (default 0.6)
//   sttmodel=base.en|small.en                → which bundled whisper model to
//                                              load (default base.en — see
//                                              resolveSttModelPath; small.en
//                                              is the accuracy opt-in, demoted
//                                              2026-07-10 after iPhone 16e lag)

/// Engine selection (Plan 07 D10/Task 11): real `MurmurEngine` when an API
/// key + config resolve, `DemoWalkEngine` when launched with `demo=1` OR
/// when no key is configured — so the design gallery and scripted autoflow
/// demos still run with zero backend. `nil` here means "use AppModel's
/// DemoWalkEngine default." Delete nothing (D10) — every existing launch arg
/// keeps working.
/// Reads the value of a `<prefix><float>` launch arg (e.g. `sttvad=0.01`),
/// falling back to `fallback` when the arg is absent OR its value doesn't
/// parse as a Float. A malformed arg logs a breadcrumb and keeps the default —
/// a typo in a QA/device launch arg must never crash the app.
func floatLaunchArg(_ prefix: String, in args: [String], default fallback: Float) -> Float {
    guard let arg = args.first(where: { $0.hasPrefix(prefix) }) else { return fallback }
    let raw = String(arg.dropFirst(prefix.count))
    guard let parsed = Float(raw) else {
        engineLog.notice("ignoring malformed \(prefix, privacy: .public) launch arg (kept default)")
        return fallback
    }
    return parsed
}

/// Resolves the three whisper knobs from launch args (all overridable for the
/// on-device sweep and design QA):
///   `sttgpu=0|1` — force whisper CPU / GPU (Metal). Default: GPU on device,
///     CPU on the simulator (Metal hard-crashes on sim — SIGTRAP via MTLSimDevice).
///   `sttvad=<float>` — per-window RMS pre-gate (default 0.0 = decode
///     everything; ~0.01 suppresses construction noise without dropping speech).
///   `sttnsp=<float>` — no_speech_prob drop threshold (default 0.6). Float args
///     parse defensively (see floatLaunchArg): a typo keeps the default.
struct SttKnobs {
    var useGpu: Bool
    var vadRms: Float
    var noSpeechProb: Float
}

func resolveSttKnobs(_ args: [String]) -> SttKnobs {
    #if targetEnvironment(simulator)
    var useGpu = false
    #else
    var useGpu = true
    #endif
    if args.contains("sttgpu=0") { useGpu = false }
    if args.contains("sttgpu=1") { useGpu = true }
    let vadRms = floatLaunchArg("sttvad=", in: args, default: 0.0)
    let noSpeechProb = floatLaunchArg("sttnsp=", in: args, default: 0.6)
    engineLog.notice("stt gpu=\(useGpu, privacy: .public)")
    engineLog.notice("stt vad_rms=\(vadRms, privacy: .public) no_speech_prob=\(noSpeechProb, privacy: .public)")
    return SttKnobs(useGpu: useGpu, vadRms: vadRms, noSpeechProb: noSpeechProb)
}

/// Live-mic vs. scripted text walk. `live=1`/`live=0` always win. Default:
/// (superseded by the in-app mode chip — this now only reports an EXPLICIT
/// arg; absent args defer to the user's persisted choice in AppModel.)
/// scripted on sim (Metal STT SIGTRAPs on MTLSimDevice; QA assumes scripted),
/// **live** on device. CAVEAT: small.en caused felt lag on iPhone 16e (sac,
/// 2026-07-10); base.en is now default — `sttmodel=small.en`/`live=0` opts back in.
func resolveLive(_ args: [String]) -> Bool? {
    if args.contains("live=1") { return true }
    if args.contains("live=0") { return false }
    return nil
}

/// Resolves the bundled whisper model path from the `sttmodel=base.en|small.en`
/// launch arg (default **base.en**). small.en (spike-validated on every
/// measured WER/hallucination axis, `spikes/stt-whisper/RESULTS.md`) was
/// DEMOTED 2026-07-10 after real-device lag on iPhone 16e — `sttmodel=
/// small.en` (or `STT_MODEL=small.en` before `./generate.sh`) opts back in.
///
/// Falls back to whichever model IS bundled if the requested one isn't
/// present — same "degrade, never crash" posture as the rest of engine
/// resolution. Returns `nil` (text-only) only if neither model is bundled.
func resolveSttModelPath(_ args: [String]) -> String? {
    let requested = args
        .first(where: { $0.hasPrefix("sttmodel=") })
        .map { String($0.dropFirst("sttmodel=".count)) } ?? "base.en"
    let fallback = requested == "base.en" ? "small.en" : "base.en"
    for name in [requested, fallback] {
        if let path = Bundle.main.path(forResource: "ggml-\(name)-q5_1", ofType: "bin") {
            if name != requested {
                engineLog.notice(
                    "sttmodel=\(requested, privacy: .public) requested but not bundled"
                )
                engineLog.notice("using \(name, privacy: .public) instead")
            } else {
                engineLog.notice("stt model=\(name, privacy: .public)")
            }
            return path
        }
    }
    return nil
}

@MainActor
func resolveEngine(demo: Bool) -> WalkEngine? {
    if demo {
        engineLog.notice("engine=demo (forced via demo=1)")
        return nil
    }
    #if canImport(MurmurCoreFFI)
    guard
        let apiKey = Bundle.main.object(forInfoDictionaryKey: "PPQ_API_KEY") as? String,
        !apiKey.isEmpty
    else {
        engineLog.notice("engine=demo (no PPQ_API_KEY configured)")
        return nil // no key configured -> demo path (D10)
    }
    // Env var wins (ad-hoc override via SIMCTL_CHILD_…), else the value baked
    // into Info.plist by generate.sh — so icon-tap launches hit the right
    // provider without any environment plumbing.
    let baseURL = ProcessInfo.processInfo.environment["ANTHROPIC_BASE_URL"]
        ?? (Bundle.main.object(forInfoDictionaryKey: "ANTHROPIC_BASE_URL") as? String)
    // iOS does not pre-create Application Support; murmur-core opens its SQLite
    // store at dbPath and panics if the parent directory is missing. Ensure it
    // exists before handing the path to the engine.
    let appSupport = FileManager.default
        .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    try? FileManager.default.createDirectory(at: appSupport, withIntermediateDirectories: true)
    let dbPath = appSupport
        .appendingPathComponent("murmur.sqlite3")
        .path
    // Bundled whisper model (Plan 08 D5; sttmodel= knob) — resolved via
    // resolveSttModelPath, which also handles the base.en/small.en fallback.
    // If neither model is bundled, the walk degrades to text-only (no crash):
    // the Rust side treats a nil path as a text-only session.
    let args = ProcessInfo.processInfo.arguments
    let sttModelPath = resolveSttModelPath(args)
    if sttModelPath == nil {
        engineLog.notice("stt model not bundled — live walk will run text-only")
    }
    let stt = resolveSttKnobs(args)
    let config = EngineConfig(
        dbPath: dbPath,
        deviceId: UIDevice.current.identifierForVendor?.uuidString ?? "unknown-device",
        apiKey: apiKey,
        baseUrl: (baseURL?.isEmpty ?? true) ? nil : baseURL,
        modelLive: "claude-haiku-4-5",
        modelProcessing: "claude-sonnet-4-5",
        modelReflection: "claude-haiku-4-5",
        sttModelPath: sttModelPath,
        sttFlushOnFinish: true, // D6 default: flush the last utterance on DONE
        sttUseGpu: stt.useGpu,
        sttVadRmsThreshold: stt.vadRms,
        sttNoSpeechProbThreshold: stt.noSpeechProb
    )
    engineLog.notice("engine=real (murmur-core MurmurEngine, key len=\(apiKey.count, privacy: .public))")
    // Throwing constructor (no panics across FFI): if the store can't open,
    // fall back to the demo path rather than crash at launch (D10). The
    // Application Support dir is created above, before this fallible init, so a
    // missing dir can't silently demote a real-key launch to the demo engine.
    return try? MurmurEngine(config: config)
    #else
    engineLog.notice("engine=demo (MurmurCoreFFI not linked)")
    return nil
    #endif
}
