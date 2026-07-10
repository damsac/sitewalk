import SwiftUI
import UIKit

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
    @State private var needsOnboarding: Bool
    private let live: Bool?
    private let wavwalk: Bool
    private let autoflowRounds: Int

    @MainActor
    init(live: Bool?, wavwalk: Bool = false, demo: Bool, voiceProcessing: Bool = false,
         autoflowRounds: Int) {
        self.live = live
        self.wavwalk = wavwalk
        self.autoflowRounds = autoflowRounds
        // Profile QA args — processed BEFORE AppModel reads the stored
        // profile. resetprofile=1 clears it (first-run QA: onboarding shows);
        // autoprofile=1 stamps a sample profile (headless board/letterhead
        // screenshots — no taps available in simctl).
        let args = ProcessInfo.processInfo.arguments
        if args.contains("resetprofile=1") { BusinessProfile.clear() }
        if args.contains("autoprofile=1") {
            BusinessProfile.save(BusinessProfile(
                businessName: "Testflight Lawn Co",
                cityState: "Denver CO",
                licenseNumber: "44-1234",
                tradeKey: "landscape"
            ))
        }
        // autoflow (screenshot/CI automation) must never be trapped behind
        // onboarding — dam review #190: fresh sim + autoflow=1 has to reach
        // the scripted walk, not stall on OnboardingFlow with no taps available.
        _needsOnboarding = State(initialValue: BusinessProfile.current == nil && autoflowRounds == 0)
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
        if needsOnboarding {
            // First run: no business profile yet — the paperwork can't carry
            // the operator's name until this arc completes.
            OnboardingFlow {
                model.reloadProfile()
                withAnimation(.easeOut(duration: 0.3)) { needsOnboarding = false }
            }
            .transition(.opacity)
        } else {
            appFlow
        }
    }

    private var appFlow: some View {
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
        case components, onboarding, jobs, capture, document, vocab

        var title: String {
            switch self {
            case .components: return "COMPONENT KIT"
            case .onboarding: return "00 · ONBOARDING"
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
                case .onboarding: OnboardingFlow(onComplete: {})
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
