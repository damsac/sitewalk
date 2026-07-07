import SwiftUI
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
//   live=1                                   → real mic + on-device STT
//   autoflow=1                               → auto-starts a scripted walk and
//                                              auto-finishes it (state machine demo)
//   demo=1                                   → force DemoWalkEngine even if a
//                                              key is configured (D10)
//   sttgpu=0|1                               → force whisper CPU / GPU (Metal)
//   sttvad=<float>                           → STT per-window RMS pre-gate
//                                              (default 0.0; ~0.01 cuts noise)
//   sttnsp=<float>                           → STT no_speech_prob drop
//                                              threshold (default 0.6)

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
///     CPU on the simulator (Metal hard-crashes on sim — SIGTRAP in
///     ggml_metal_buffer_set_tensor via MTLSimDevice; D7's "degrades to CPU"
///     assumption was falsified, so a sim build can never crash by default).
///   `sttvad=<float>` — per-window RMS pre-gate (default 0.0 = decode
///     everything; ~0.01 suppresses construction noise without dropping speech).
///   `sttnsp=<float>` — no_speech_prob drop threshold (default 0.6).
/// Float args parse defensively (see floatLaunchArg): a typo keeps the default.
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
    // Bundled whisper model (Plan 08 D5) — resolved from the app bundle. If the
    // resource is missing the walk degrades to text-only (no crash): the Rust
    // side treats a nil path as a text-only session.
    let sttModelPath = Bundle.main.path(forResource: "ggml-base.en-q5_1", ofType: "bin")
    if sttModelPath == nil {
        engineLog.notice("stt model not bundled — live walk will run text-only")
    }
    let stt = resolveSttKnobs(ProcessInfo.processInfo.arguments)
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
                live: Self.args.contains("live=1"),
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
    private let live: Bool
    private let wavwalk: Bool
    private let autoflowRounds: Int

    @MainActor
    init(live: Bool, wavwalk: Bool = false, demo: Bool, voiceProcessing: Bool = false,
         autoflowRounds: Int) {
        self.live = live
        self.wavwalk = wavwalk
        self.autoflowRounds = autoflowRounds
        // Both live (mic) and wavwalk (fixture) are real whisper walks — neither
        // is the scripted text path. wavwalk drives the STT path from a bundled
        // WAV instead of the mic (Plan 08 D7).
        let whisperWalk = live || wavwalk
        _model = State(
            initialValue: AppModel(
                engine: resolveEngine(demo: demo),
                scripted: !whisperWalk,
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
                        WalkView(model: model, scriptedLabel: !(live || wavwalk))
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
            // Reconciling sweep (Plan 11 D4): app-open ONLY, never background —
            // a concurrent sweep could race an in-flight capture (bytes written,
            // row not yet committed) and delete a just-captured photo. App-open
            // is a quiescent point (no capture in flight).
            model.sweepPhotoBytes()
            if live {
                _ = await SpeechSource.requestPermissions()
            }
            for round in 0..<autoflowRounds {
                if round > 0 {
                    model.completeSend()
                    try? await Task.sleep(for: .seconds(1))
                }
                try? await Task.sleep(for: .seconds(1))
                model.startWalk()
                // Let the scripted walk play out, then finish it.
                try? await Task.sleep(for: .seconds(16))
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
        case components, jobs, capture, document

        var title: String {
            switch self {
            case .components: return "COMPONENT KIT"
            case .jobs: return "01 · JOBS BOARD"
            case .capture: return "02 · CAPTURE"
            case .document: return "04 · DOCUMENT REVIEW"
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
                }
            }
        }
        .tint(Theme.C.ink)
    }
}
