import SwiftUI
import UIKit
import os
#if canImport(MurmurCoreFFI)
import MurmurCoreFFI
#endif

// Key-free breadcrumb so you can confirm from the console which engine is live
// (real murmur-core vs. the scripted demo) without ever logging the API key.
private let engineLog = Logger(subsystem: "com.damsac.sitewalk", category: "engine")

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
private func floatLaunchArg(_ prefix: String, in args: [String], default fallback: Float) -> Float {
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
private struct SttKnobs {
    var useGpu: Bool
    var vadRms: Float
    var noSpeechProb: Float
}

private func resolveSttKnobs(_ args: [String]) -> SttKnobs {
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
private func resolveLive(_ args: [String]) -> Bool? {
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
private func resolveSttModelPath(_ args: [String]) -> String? {
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
private func resolveEngine(demo: Bool) -> WalkEngine? {
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

@main
struct GalleryApp: App {
    var body: some Scene {
        WindowGroup {
            RootRouter()
        }
    }
}

struct RootRouter: View {
    private static let args = ProcessInfo.processInfo.arguments

    var body: some View {
        if Self.args.contains(where: { $0.hasPrefix("screen=") }) {
            GalleryRoot()
        } else {
            AppRoot(
                live: resolveLive(Self.args),
                wavwalk: Self.args.contains("wavwalk=1"),
                demo: Self.args.contains("demo=1"),
                voiceProcessing: Self.args.contains("voiceproc=1"),
                autoflowRounds: Self.args
                    .first(where: { $0.hasPrefix("autoflow=") })
                    .flatMap { Int($0.dropFirst("autoflow=".count)) } ?? 0
            )
        }
    }
}

struct AppRoot: View {
    @State private var model: AppModel
    private let live: Bool?
    private let wavwalk: Bool
    private let autoflowRounds: Int

    @MainActor
    init(live: Bool?, wavwalk: Bool = false, demo: Bool, voiceProcessing: Bool = false,
         autoflowRounds: Int) {
        self.live = live
        self.wavwalk = wavwalk
        self.autoflowRounds = autoflowRounds
        // Mode is a USER choice (persisted, board chip) unless a launch arg
        // forces it: wavwalk/live=1 → voice; demo=1/live=0 → demo; autoflow
        // without an explicit voice arg → demo (scripted determinism for
        // screenshots/CI). Forced modes lock the chip and never persist.
        let forcedMode: AppModel.WalkMode?
        if wavwalk || live == true {
            forcedMode = .voice
        } else if demo || live == false {
            forcedMode = .demo
        } else if autoflowRounds > 0 {
            forcedMode = .demo
        } else {
            forcedMode = nil
        }
        _model = State(
            initialValue: AppModel(
                engine: resolveEngine(demo: demo),
                forcedMode: forcedMode,
                wavFixture: wavwalk,
                voiceProcessing: voiceProcessing
            )
        )
    }

    var body: some View {
        NavigationStack(path: Bindable(model).path) {
            BoardView(model: model)
                .navigationDestination(for: AppModel.Phase.self) { phase in
                    switch phase {
                    case .walking:
                        WalkView(model: model)
                    case .building:
                        BuildView(model: model)
                    case .review:
                        ReviewView(model: model)
                    case .board:
                        BoardView(model: model)
                    }
                }
        }
        .tint(Theme.C.ink)
        .task {
            // INVARIANT: stays FIRST in this .task (no `await` before it) and
            // is never re-fired while a walk is live — one suspension point
            // ahead of it would Fail a live session. See runAppOpenSweeps().
            model.runAppOpenSweeps()
            // (Removed: the legacy SpeechSource permission ask for live=1.
            // STT is Rust-side whisper — Apple Speech Recognition is never
            // used on the walk path, and its system dialog carries "sent to
            // Apple" copy that is untrue for this product. Mic permission is
            // requested where it belongs: AppModel.startWalk, voice mode.)
            for round in 0..<autoflowRounds {
                if round > 0 {
                    model.completeSend()
                    try? await Task.sleep(for: .seconds(1))
                }
                try? await Task.sleep(for: .seconds(1))
                model.startWalk()
                // Let the scripted walk play out, then finish it.
                try? await Task.sleep(for: .seconds(8))
                // Screenshot-automation hook: exercise the walk-time photo
                // path (button → capturePhoto → FFI → gallery) unattended.
                if ProcessInfo.processInfo.arguments.contains("autophoto=1"),
                   model.phase == .walking {
                    let renderer = UIGraphicsImageRenderer(size: CGSize(width: 320, height: 240))
                    let image = renderer.image { ctx in
                        UIColor(red: 0.91, green: 0.33, blue: 0.12, alpha: 1).setFill()
                        ctx.fill(CGRect(x: 0, y: 0, width: 320, height: 240))
                        UIColor.white.setFill()
                        ctx.fill(CGRect(x: 24, y: 100, width: 272, height: 40))
                    }
                    if let data = image.jpegData(compressionQuality: 0.8) {
                        model.addPhoto(data)
                    }
                }
                try? await Task.sleep(for: .seconds(8))
                if model.phase == .walking {
                    model.finishWalk()
                }
                try? await Task.sleep(for: .seconds(3))
            }
            // Screenshot-automation hook: render the PDF unattended.
            if autoflowRounds > 0, ProcessInfo.processInfo.arguments.contains("autopdf=1") {
                try? await Task.sleep(for: .seconds(1))
                if model.phase == .review {
                    model.makePDF()
                }
            }
        }
    }
}

// MARK: - Static design gallery (kept for design QA and previews)

struct GalleryRoot: View {
    enum Dest: String, Hashable, CaseIterable {
        case components, jobs, capture, document, vocab

        var title: String {
            switch self {
            case .components: return "COMPONENT KIT"
            case .jobs: return "01 · JOBS BOARD"
            case .capture: return "02 · CAPTURE"
            case .document: return "04 · DOCUMENT REVIEW"
            case .vocab: return "05 · FIELD VOCABULARY"
            }
        }
    }

    static func initialPath() -> [Dest] {
        if let arg = ProcessInfo.processInfo.arguments.first(where: { $0.hasPrefix("screen=") }),
           let dest = Dest(rawValue: String(arg.dropFirst("screen=".count))) {
            return [dest]
        }
        return []
    }

    @State private var path: [Dest] = GalleryRoot.initialPath()

    var body: some View {
        NavigationStack(path: $path) {
            VStack(alignment: .leading, spacing: 0) {
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 10) {
                        Rectangle().fill(Theme.C.orange).frame(width: 13, height: 13)
                        Text("SITEWALK")
                            .font(Theme.F.ui(24, .extraBold))
                            .tracking(3.5)
                    }
                    Text("Design system gallery — DS-01")
                        .font(Theme.F.mono(9))
                        .foregroundStyle(Theme.C.ink60)
                }
                .padding(.horizontal, Theme.S.screenPad)
                .padding(.top, 18)
                .padding(.bottom, 16)
                .overlay(alignment: .bottom) { Theme.C.ink.frame(height: 2) }

                ForEach(Dest.allCases, id: \.self) { dest in
                    NavigationLink(value: dest) {
                        HStack {
                            Text(dest.title)
                                .font(Theme.F.mono(11, .medium))
                                .tracking(1.2)
                                .foregroundStyle(Theme.C.ink)
                            Spacer()
                            Text("→")
                                .font(Theme.F.mono(11))
                                .foregroundStyle(Theme.C.orangeDeep)
                        }
                        .padding(.horizontal, Theme.S.screenPad)
                        .padding(.vertical, 16)
                        .overlay(alignment: .bottom) { Theme.C.hairline.frame(height: 1) }
                    }
                }

                Spacer()
            }
            .background(Theme.C.paper.ignoresSafeArea())
            .navigationDestination(for: Dest.self) { dest in
                switch dest {
                case .components: ComponentsPage()
                case .jobs: JobsBoardScreen(trade: Fixtures.landscape)
                case .capture: CaptureScreen(trade: Fixtures.landscape)
                case .document: DocumentReviewScreen(trade: Fixtures.landscape)
                case .vocab: VocabularyView(model: AppModel())
                }
            }
        }
        .tint(Theme.C.ink)
    }
}
