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
    @Environment(\.scenePhase) private var scenePhase
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
        // resetcoach=1 re-arms the first-run coach marks (parallel to
        // resetprofile) so they can be captured/verified on a warm sim.
        if args.contains("resetcoach=1") {
            for key in CoachMarks.allKeys { UserDefaults.standard.removeObject(forKey: key) }
        }
        if args.contains("autoprofile=1") {
            BusinessProfile.save(BusinessProfile(
                businessName: "Testflight Lawn Co",
                cityState: "Denver CO",
                licenseNumber: "44-1234",
                tradeKey: "landscape"
            ))
        }
        // practice=1: land straight on a practice board — stamps a profile (so
        // onboarding is skipped) and arms the scripted, unsaved practice run.
        let practiceArg = args.contains("practice=1")
        if practiceArg {
            BusinessProfile.save(BusinessProfile(
                businessName: "Testflight Lawn Co", cityState: "Denver CO",
                licenseNumber: "44-1234", tradeKey: "landscape"
            ))
        }
        // meter=N seeds the free-tier walk meter for the CURRENT month (parallel
        // to autoprofile). The blocked-at-the-limit paywall is otherwise
        // reachable only by finishing five real walks, which no headless
        // screenshot run and no quick manual check can do. `meter=0` clears it.
        if let arg = args.first(where: { $0.hasPrefix("meter=") }),
           let count = Int(arg.dropFirst("meter=".count)) {
            WalkMeter.save(WalkAllowance.Record(
                month: WalkAllowance.monthKey(for: Date()), count: max(0, count)
            ))
        }
        // QA hooks for the Letterhead Studio (parallel to autoprofile): stamp a
        // sample branding so headless screenshots exercise a customized letterhead.
        if args.contains("resetbrand=1") { Branding.save(.default); DocumentLayout.save(.default) }
        if args.contains("autobrand=1") {
            Branding.save(Branding(
                presetKey: "field", accentHex: 0x3E6B35, fontKey: "sans",
                phone: "(303) 555-0147", email: "quotes@aldercourt.co",
                website: "aldercourt.co", showWatermark: true
            ))
            DocumentLayout.save(DocumentLayout(
                termsText: "50% deposit due to schedule · balance on completion · quote valid 30 days.",
                showSignature: true
            ))
        }
        // autoflow (screenshot/CI automation) must never be trapped behind
        // onboarding — dam review #190: fresh sim + autoflow=1 has to reach
        // the scripted walk, not stall on OnboardingFlow with no taps available.
        // Same rule for Plan 15's vocab-seed card: it pops on the first notes
        // screen and needs a SKIP/DONE tap, so mark it shown for autoflow runs.
        if autoflowRounds > 0 {
            UserDefaults.standard.set(true, forKey: NotesView.vocabCardShownKey)
            // Coach marks are non-blocking, but keep the scripted screenshots
            // clean by marking them shown for autoflow runs.
            for key in CoachMarks.allKeys { UserDefaults.standard.set(true, forKey: key) }
        }
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
                voiceProcessing: voiceProcessing,
                practiceArmed: practiceArg
            )
        )
    }

    var body: some View {
        if needsOnboarding {
            // First run: no business profile yet — the paperwork can't carry
            // the operator's name until this arc completes.
            OnboardingFlow { startPractice in
                model.reloadProfile()
                withAnimation(.easeOut(duration: 0.3)) { needsOnboarding = false }
                // Land on the board armed for a scripted, unsaved practice run;
                // the START coach mark + demo content carry them through it.
                if startPractice { model.armPracticeWalk() }
            }
            .transition(.opacity)
        } else {
            appFlow
                // Whisper's Metal GPU encode aborts if it runs while the app is
                // backgrounded (SIGABRT via ggml_abort) — pause STT the moment we
                // leave the foreground, resume when we're back. `.active` is the
                // only safe-for-GPU state; `.inactive`/`.background` both stop it.
                .onChange(of: scenePhase) { _, phase in
                    model.handleBackgroundTransition(backgrounded: phase != .active)
                }
        }
    }

    private var appFlow: some View {
        NavigationStack(path: Bindable(model).path) {
            BoardView(model: model)
                .navigationDestination(for: AppModel.Phase.self) { phase in
                    switch phase {
                    case .walking:
                        WalkView(model: model)
                    case .notes:
                        NotesView(model: model)
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
            // seedwalks=1: fake finished walks for App Store capture and for
            // eyeballing the #221 row labels, which are otherwise a real-core-
            // only path. AFTER runAppOpenSweeps so the hydrate latch has
            // already fired and can't overwrite the seed.
            if ProcessInfo.processInfo.arguments.contains("seedwalks=1") {
                model.seedDemoWalks()
            }
            // Entitlement + product load, off the critical path. Detached from
            // this .task on purpose: it awaits the network, and the INVARIANT
            // above forbids a suspension point ahead of runAppOpenSweeps().
            // `isPro` starts false and is corrected when this lands — the
            // free-tier gate reads it, so the worst case is a subscriber
            // briefly seeing the free count on a cold launch, never a lapsed
            // user briefly getting Pro.
            Task { await model.entitlement.start() }
            // paywall=1 raises the paywall on launch. simctl can't tap, so this
            // is the only way to verify the sheet headlessly; pair with meter=5
            // for the refused-at-the-limit copy.
            if ProcessInfo.processInfo.arguments.contains("paywall=1") {
                model.blockedUsage = ProcessInfo.processInfo.arguments.contains("meter=5")
                    ? (used: WalkAllowance.freeMonthlyLimit, limit: WalkAllowance.freeMonthlyLimit)
                    : nil
                model.showPaywall = true
            }
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
                // Plan 13: finishWalk() now lands on the notes screen, not
                // review — the document is built deliberately via the one
                // wired button (Task 7). Drive it here so autoflow still
                // reaches ReviewView for the existing PDF/send screenshot
                // hooks below.
                if model.phase == .notes {
                    model.buildPrimaryDocument()
                    try? await Task.sleep(for: .seconds(2))
                }
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
        case components, onboarding, jobs, capture, document, vocab, structure

        var title: String {
            switch self {
            case .components: return "COMPONENT KIT"
            case .onboarding: return "00 · ONBOARDING"
            case .jobs: return "01 · JOBS BOARD"
            case .capture: return "02 · CAPTURE"
            case .document: return "04 · DOCUMENT REVIEW"
            case .vocab: return "05 · FIELD VOCABULARY"
            case .structure: return "06 · DOCUMENT BUILDER"
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

    /// ONE model for the whole gallery, created once.
    ///
    /// The vocab and structure destinations used to construct `AppModel()`
    /// inline in the `navigationDestination` builder. SwiftUI re-evaluates that
    /// builder freely, so every evaluation minted a fresh model — which was
    /// merely wasteful until `AppModel` gained an `Entitlement` (#277), and each
    /// one spawned a `Transaction.updates` listener. From then on `screen=vocab`
    /// and `screen=structure` rendered BLANK (#289): the whole design gallery,
    /// the only headless route to any screen behind a tap, was dead.
    ///
    /// Hoisting to `@State` is the fix and is also just correct SwiftUI —
    /// building an observable model inside a ViewBuilder was always wrong,
    /// StoreKit only made it visible.
    @State private var galleryModel = AppModel()

    var body: some View {
        NavigationStack(path: $path) {
            VStack(alignment: .leading, spacing: 0) {
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 10) {
                        Rectangle().fill(Theme.C.orangeDeep).frame(width: 13, height: 13)
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
                case .onboarding: OnboardingFlow(onComplete: { _ in })
                case .jobs: JobsBoardScreen(trade: Fixtures.landscape)
                case .capture: CaptureScreen(trade: Fixtures.landscape)
                case .document: DocumentReviewScreen(trade: Fixtures.landscape)
                case .vocab: VocabularyView(model: galleryModel)
            // The Document Builder is the surface most in need of design
            // iteration without a Rust build — the demo engine seeds it
            // with built-ins, so `screen=structure` is a full editor.
            case .structure: DocumentBuilderView(model: galleryModel)
                }
            }
        }
        .tint(Theme.C.ink)
    }
}
