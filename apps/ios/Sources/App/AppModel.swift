import SwiftUI
import Observation
import UIKit
import os

// One observable model drives the whole flow:
//   board → walking (pause/resume, photos) → notes → review (edit, fill gaps) → sent
// (Plan 13: DONE computes NOTES, not a document — `notes` is the
// primary result; `review` is reached deliberately via a build-document
// button from the notes screen, not automatically at DONE.)
// The engine behind it is injected; today that's DemoWalkEngine, tomorrow the
// FFI bridge. The UI never knows the difference.

@MainActor
@Observable
final class AppModel {

    enum Phase: Equatable {
        case board
        case walking
        case notes
        case review
    }

    // MARK: State

    var trade: TradeFixture = Fixtures.landscape
    var jobs: [JobFixture] = Fixtures.landscape.jobs
    var phase: Phase = .board
    var path: [Phase] = []

    /// The operator's business (nil until onboarding saves one). When set,
    /// the fixture business disappears: the board header carries the
    /// profile name, the trade comes from the profile (no switcher), and
    /// every letterhead is stamped with the operator — see letterheadBiz/
    /// letterheadSub/letterheadDate. App-side only for now (BusinessProfile).
    private(set) var profile: BusinessProfile?

    /// The operator's document branding (logo / accent / letterhead font /
    /// contact / footer). Loaded from persistence; the Letterhead Studio edits a
    /// copy and commits via `saveBranding`. App-side only (design doc §5 — the
    /// STYLE half). `.current` reads the stored record or falls back to stock.
    var branding: Branding = .current

    /// App-side document STRUCTURE basics (terms / signature) — the "B-basics"
    /// (design doc §8) rendered on every document ahead of dam's core schema
    /// seam (§7.2). Edited in the Letterhead Studio, committed via
    /// `saveDocumentLayout`. Separate from `branding` (style) on purpose.
    var documentLayout: DocumentLayout = .current

    /// One board row per finished walk. Plan 20: no longer this-session-only —
    /// `hydrateWalkLog()` fills it from `engine.listSessions()` at app open,
    /// so history survives relaunch; `completeSend`/`discardDocument` still
    /// append full-fidelity in-session records until the next hydrate.
    struct WalkRecord: Identifiable {
        let id = UUID()
        let time: String     // "9:41"
        let docNo: String
        let docKind: String  // "ESTIMATE" / "MOVE-OUT REPORT" / ...
        let sent: Bool       // false = discarded at review
        /// The core session id this row reopens (Plan 20 D5). "" only for
        /// legacy/demo rows with no session (reopen no-ops there).
        let sessionId: String
        /// Mirror of `NotesModel.queued` for the reopened-notes gating banner.
        let queued: Bool
        /// What actually happened to this walk, for the board label.
        ///
        /// `sent` alone was a two-state guess derived from `hasDocument`, so a
        /// walk that was finished but never turned into a document rendered as
        /// "DISCARDED" at 55% opacity — actively alarming, and wrong: NOTHING
        /// in WalkSummary records a discard. Now that a finished walk enters
        /// the log immediately (it used to appear only on send), that mislabel
        /// would have been the first thing an operator saw after every walk.
        enum Disposition {
            /// Processing hasn't finished — notes aren't readable yet.
            case saving
            /// Notes are saved. No document was built, which is a perfectly
            /// ordinary end state, not a failure.
            case saved
            /// A document exists for this walk.
            case documented
        }

        var disposition: Disposition {
            if queued { return .saving }
            return sent ? .documented : .saved
        }

        /// The board row's headline. Pure so it can be tested without a store.
        ///
        /// Prefers the walk's own summary — the one thing that answers "what
        /// was this?" months later. Falls back to a document number when there
        /// genuinely is one (in-session records keep the real minted number),
        /// then to a plain, honest label. Never blank: an untitled row is
        /// unreadable and looks broken.
        var title: String {
            let trimmed = AppModel.firstSentence(of: summary)
            if !trimmed.isEmpty { return trimmed }
            if !docNo.isEmpty { return docNo }
            return queued ? "Walk still processing" : "Walk notes"
        }

        /// The quiet second line. Item count is a FACT about the walk; the doc
        /// kind is advisory and would read as "an estimate exists" on a walk
        /// that never produced one (#221). Kind is shown only once a document
        /// really was built.
        var subtitle: String {
            let items = itemCount == 1 ? "1 item" : "\(itemCount) items"
            guard disposition == .documented, !docKind.isEmpty else { return items }
            return "\(items) · \(docKind.capitalized)"
        }

        /// Raw start time, epoch SECONDS (core `started_at`). Kept alongside
        /// the formatted `time` for two reasons that arrived independently:
        /// the TODAY list wants a clock ("9:41") while a job card spanning
        /// months needs a date, AND the loose list groups by calendar day
        /// (TODAY / EARLIER) rather than piling a multi-day history under one
        /// hardcoded "TODAY" (operator report 2026-07-27: "random walks").
        ///
        /// `UInt64` to match `WalkSummary.startedAt` at the FFI boundary —
        /// converted at the two comparison sites rather than stored lossy.
        let startedAt: UInt64

        /// The job this walk is filed under; nil = unfiled. Set AFTER the walk
        /// (R4: no pre-labeling), and re-settable — a walk filed to the wrong
        /// job months ago has to be movable.
        var jobId: String?

        /// The walk's narrative summary — what the board row actually says.
        ///
        /// The row used to lead with `docNo`, which is synthesized EMPTY for
        /// every stored walk (the number is minted per-build and isn't in the
        /// lightweight projection), so every hydrated row rendered with a blank
        /// title (#221). The summary is the honest answer to "what was this
        /// walk?" — and it is precisely the question an operator opening a
        /// months-old walk has.
        let summary: String

        /// How many lines the walk captured. A quiet, TRUE subtitle — unlike
        /// the doc kind, which is advisory and reads as a claim that an
        /// estimate exists when none was ever built.
        let itemCount: Int

        init(time: String, docNo: String, docKind: String, sent: Bool,
             sessionId: String, queued: Bool, jobId: String? = nil,
             startedAt: UInt64 = 0, summary: String = "", itemCount: Int = 0) {
            self.jobId = jobId
            self.summary = summary
            self.itemCount = itemCount
            self.time = time
            self.docNo = docNo
            self.docKind = docKind
            self.sent = sent
            self.sessionId = sessionId
            self.queued = queued
            self.startedAt = startedAt
        }

        /// Board hydration mapping (Plan 20 F7, pinned): `sent` reads a
        /// built-and-kept walk as "sent"; **`docNo` is synthesized empty** —
        /// the document number is minted per-build and is not in the
        /// lightweight projection, an ACCEPTED v1 fidelity loss (in-session
        /// records keep the real number until the next hydrate overwrites
        /// the log).
        init(_ summary: WalkSummary) {
            self.time = AppModel.clockTime(epochSeconds: summary.startedAt)
            self.docNo = ""
            self.docKind = DocKinds.label(for: summary.docKind)
            self.sent = summary.hasDocument
            self.sessionId = summary.id
            self.queued = summary.queued
            self.jobId = summary.jobId
            self.startedAt = summary.startedAt
            self.summary = summary.summary
            self.itemCount = Int(summary.itemCount)
        }
    }
    private(set) var sessionWalks: [WalkRecord] = []

    // MARK: Billing

    /// Jefe Pro entitlement. Owned by the model so every gate reads one answer;
    /// `GalleryApp` kicks off `start()` at launch.
    let entitlement = Entitlement()

    /// Drives the paywall sheet. Set only by `startWalk()` refusing, and by the
    /// explicit "upgrade" affordances — never by a background event, so the
    /// paywall can't appear over a walk in progress.
    var showPaywall = false

    /// Usage at the moment the gate refused, so the paywall can name the number
    /// instead of running a generic upsell. Nil until a refusal.
    var blockedUsage: (used: Int, limit: Int)?

    /// Free walks left at the moment the first-walk offer was raised, or nil
    /// when the paywall is not that offer. Kept separate from `blockedUsage` so
    /// the sheet can soften its copy and its dismiss button, and carried as a
    /// number so the copy is accurate for both the practice walk (nothing spent
    /// yet) and a real first walk (one spent) — without the model importing the
    /// view layer to say so.
    var paywallOfferFreeLeft: Int?

    /// Free walks left AFTER the one just started; nil for Pro. Read by the
    /// notes screen to nudge at the end of a walk rather than the start.
    var walksRemainingAfterThis: Int?

    // Walk state
    var transcript = ""
    /// Volatile greyed preview tail from the Rust STT pump (Plan 08 D4) — the
    /// un-finalized hypothesis. Never persisted; rendered greyed (nice-to-have).
    var previewTail = ""
    var items: [CapturedFixture] = []
    var isPaused = false
    var walkStart = Date()
    var pausedElapsed: TimeInterval = 0
    /// Most-recently-captured item's id, tracked explicitly from the event
    /// loop (Plan 07 D3/Task 10) — under whole-board `boardUpdated` replace,
    /// "array tail == most-recently-captured" is NOT load-bearing: a
    /// re-extraction mints new ids mid-swap and store ordering is insertion
    /// order, not mention order. `addPhoto()` pins by this id, never by
    /// array position. Which item a photo pins to is ultimately a core
    /// concern (HANDOFF open Q3, photo sync schema — Deferred 6); until that
    /// lands, "most-recently-captured id" is the honest interim rule.
    var lastCapturedID: UUID?

    // Vocabulary editor state (Plan 10). The list is the source of truth the
    // editor renders; `vocabularyError` carries a thrown FFI error for display.
    private(set) var vocabulary: [String] = []
    var vocabularyError: String?

    // Photo attachments (Plan 11). `photos` is the source of truth the review
    // gallery renders (loaded via `loadPhotos(sessionId:)`); `photoError`
    // carries a thrown FFI error for display.
    // sac: capture affordance placement, gallery layout/thumbnails, empty
    // state, and per-item attach gesture are yours — this is functional-plain.
    // (Not `private(set)`: mutated from AppModel+Photos.swift, a same-module
    // extension in a different file — Swift's `private` is file-scoped.)
    var photos: [PhotoModel] = []
    var photoError: String?
    /// In-flight guard for the app-open Failed-session retry (review #206
    /// should-fix): SwiftUI can re-run the launching `.task` on scene
    /// re-appearance, and two overlapping retry runs would each drive
    /// process() on the same Failed session — the second loses harmlessly at
    /// the state machine but burns a real duplicate LLM call (R9). One retry
    /// run per app process at a time. Not `private`: mutated from
    /// AppModel+Photos.swift (same-module extension, different file).
    var isRetryingFailedSessions = false
    /// Chains capture calls (PR #176 should-fix, AppModel+Photos.swift) so
    /// rapid taps run their off-main bytes-write + attach sequentially, in
    /// tap order, rather than interleaving. Not `private`: mutated from
    /// AppModel+Photos.swift, a same-module extension in a different file —
    /// Swift's `private` is file-scoped (same pattern as `photos` above).
    var photoCaptureChain: Task<Void, Never>?
    /// Snapshotted when a walk successfully begins and kept through review
    /// (Plan 11 D7): the real `MurmurEngine` drops its live `WalkSession` once
    /// `finish()` returns, so `engine.currentSessionId` alone would go nil
    /// exactly when review-time photo capture needs it. Engine-keyed CRUD
    /// (add/list/remove_photo) works on a `Processed` session too — there is
    /// no live `WalkSession` requirement, just this id.
    private(set) var currentSessionId: String?

    // Notes state (Plan 13 D1/D2): the primary finish() result. // sac: the
    // real notes screen (grouping, action-button set, transcript row) is
    // yours (docs/design/notes-mockup.html) — this is the plumbing +
    // plainest functional rendering (NotesView.swift).
    var notes: NotesModel?
    /// Set when a `buildDocument` tap fails (illegal kind, non-Processed
    /// session) — surfaced by the notes screen; the button stays available
    /// to retry. // sac: error chrome is yours; this is the plumbing.
    var documentBuildError: String?
    /// Set when an item edit/add/remove (Plan 16 CRUD) throws — surfaced on the
    /// notes screen; the edit sheet stays so the operator can retry.
    var notesEditError: String?
    /// Set when a board-row reopen tap fails (Plan 20 F4: a NotFound/
    /// tombstoned race must be a breadcrumb, never a silent dead tap).
    /// // sac: the reopenError chrome is yours; the floor is the log + this
    /// // sac: breadcrumb string surfaced near the walk log.
    var reopenError: String?
    /// How the current notes screen was reached — picks the queued-banner
    /// copy (Plan 20 F5): a FRESH offline finish can honestly promise
    /// "unlocks when you reconnect"; a REOPENED still-Failed walk cannot
    /// (the retry sweep may already have run and exhausted).
    enum NotesBannerReason { case liveFinish, reopened }
    var notesBannerReason: NotesBannerReason = .liveFinish
    /// Once-per-process guard for `hydrateWalkLog()` (Plan 20 F2, mirror of
    /// `isRetryingFailedSessions`): SwiftUI can re-fire the launching `.task`,
    /// and a re-hydrate must not re-run (in the demo it would race the
    /// in-memory log).
    var isHydratingWalkLog = false
    /// True while a build-document tap is in flight — the notes screen
    /// disables the button so a double-tap can't burn two document numbers
    /// (D7: numbers mint per generate).
    var isBuildingDocument = false

    // Review state
    var document: DocumentModel?
    var editingRowID: UUID?
    var editText = ""
    /// The line's description, edited alongside its amount. Isaac, 2026-08-09:
    /// "the user should be able to edit the name of any of the lines, and also
    /// delete or add a line should they choose." A transcript gets a word
    /// wrong roughly as often as it gets a number wrong, and until now only
    /// the number could be fixed on the paper.
    var editTitle = ""
    /// True while the row being edited was just added by ADD LINE and has
    /// never had a description. Dismissing without typing one must leave no
    /// blank line behind on the document.
    var editingRowIsNew = false
    var shareURL: URL?

    // Not fully `private`: read from AppModel+Photos.swift (a same-module
    // extension in a different file — Swift's `private` is file-scoped).
    private(set) var engine: WalkEngine
    /// The real engine, parked while a practice run borrows a throwaway
    /// `DemoWalkEngine` (F1, dam's #238 review): a practice walk must NOT touch
    /// the real store. Swapping the engine gives structural isolation — on
    /// real-core, `begin`/`append`/`finish`/`buildDocument` would otherwise all
    /// persist a phantom session+items+document (demo-only CI can't see it,
    /// since the demo engine never persists). Non-nil ⇔ a practice run is
    /// borrowing the demo engine; restored on every practice-exit path.
    private var suspendedEngine: WalkEngine?
    private var source: TranscriptSource?
    /// The live PCM source (Plan 08): used instead of `source` when
    /// `!scripted`. Produces PCM (not text) — STT is Rust-side whisper. Either
    /// the live mic (`AudioCaptureSource`) or a bundled fixture WAV
    /// (`WavFileAudioSource`, the mic-free `wavwalk=1` path, D7).
    private var audioSource: (any PCMAudioSource)?
    private var pumpTask: Task<Void, Never>?
    private var eventTask: Task<Void, Never>?

    /// The walk's input mode — a USER choice now, not a launch condition.
    /// `.voice` = mic → on-device whisper (the product). `.demo` = the canned
    /// scripted walk (kept for showing the moment; graduates into onboarding).
    /// Persisted across launches; launch args (`live=`, `demo=1`, autoflow)
    /// still force a mode for QA — forced modes lock the toggle and don't
    /// persist (D10: every existing launch arg keeps working).
    enum WalkMode: String { case voice, demo }
    var walkMode: WalkMode {
        didSet {
            guard !modeLocked else { return }
            UserDefaults.standard.set(walkMode.rawValue, forKey: Self.walkModeKey)
        }
    }
    let modeLocked: Bool
    private static let walkModeKey = "sitewalk.walkMode"
    /// Set when the user tries a voice walk with mic permission denied —
    /// BoardView surfaces it with an "open Settings" affordance.
    var micDenied = false
    /// Plan 20 D9: true from the START WALK paint until `begin` + wiring
    /// complete — WalkView shows "MIC STARTING…" while the (usually warm,
    /// occasionally cold-load) engine bring-up runs behind the painted screen.
    var micStarting = false
    /// The mic could not be opened — the session would not activate, or the
    /// input reported an invalid format. Surfaced on the walk screen; the walk
    /// is live but deaf, and the operator has to know that immediately.
    var micUnavailable = false
    /// Latch preventing a second walk from starting while one is being set up.
    /// See `startWalk()` — this is what stops two mic taps and the abort() they
    /// caused.
    private var isStartingWalk = false

    /// A scripted, unsaved "practice run" armed from onboarding's optional
    /// offer. While set, the next walk plays demo content regardless of the
    /// persisted mode (a first-timer needn't know what to say) AND never lands
    /// on the real board — cleared the moment the practice run leaves the flow.
    private(set) var isPracticeWalk = false

    /// When live, drive the STT path from a bundled fixture WAV instead of the
    /// mic (`wavwalk=1`, D7) — a mic-free way to exercise real whisper.
    private let wavFixture: Bool
    /// Voice-processing A/B knob (Plan 08 Task 10): enable Apple's on-device
    /// noise/echo suppression on the mic capture path. Sourced from the
    /// `voiceproc=1` launch arg; only affects the live-mic `AudioCaptureSource`
    /// (the WAV fixture already has clean PCM). Default off — the Task 12 SNR
    /// eval decides the production default.
    private let voiceProcessing: Bool
    /// Injection seam for the scripted text source (issue #155): tests and
    /// previews can substitute a source without touching the walk lifecycle.
    @ObservationIgnored
    var makeScriptedSource: (TradeFixture) -> TranscriptSource = { ScriptedSource(trade: $0) }

    init(engine: WalkEngine? = nil, forcedMode: WalkMode? = nil, wavFixture: Bool = false,
         voiceProcessing: Bool = false, practiceArmed: Bool = false) {
        self.engine = engine ?? DemoWalkEngine()
        self.isPracticeWalk = practiceArmed  // QA: practice=1 lands on a practice board
        if let forcedMode {
            self.walkMode = forcedMode
            self.modeLocked = true
        } else {
            self.walkMode = UserDefaults.standard.string(forKey: Self.walkModeKey)
                .flatMap(WalkMode.init(rawValue:)) ?? .voice
            self.modeLocked = false
        }
        self.wavFixture = wavFixture
        self.voiceProcessing = voiceProcessing
        self.profile = BusinessProfile.current
        if let trade = profile?.trade {
            self.trade = trade
            self.jobs = trade.jobs
        }
    }

    /// Re-read the persisted profile (after onboarding FINISH) and align the
    /// trade template with it.
    func reloadProfile() {
        profile = BusinessProfile.current
        if let trade = profile?.trade {
            self.trade = trade
            self.jobs = trade.jobs
        }
    }

    func toggleMode() {
        guard !modeLocked else { return }
        walkMode = walkMode == .voice ? .demo : .voice
        if walkMode == .demo { micDenied = false }
    }

    /// Arm the optional practice walk (onboarding's "try a practice walk"): a
    /// scripted dry run that is never saved. We deliberately do NOT touch
    /// `walkMode` — the persisted default stays whatever the operator will
    /// really use — and just land on the board so the START coach mark + the
    /// PRACTICE marker frame it.
    func armPracticeWalk() {
        isPracticeWalk = true
        // F1 (dam's #238 review): park the real engine and run the practice
        // walk against a throwaway DemoWalkEngine so the full lifecycle stays
        // off the real store. Idempotent — arm is only reached from onboarding,
        // but guard against a double-arm leaking the real engine reference.
        if suspendedEngine == nil {
            suspendedEngine = engine
            engine = DemoWalkEngine()
        }
        micDenied = false
        phase = .board
        path = []
    }

    /// Restore the real engine after a practice run. No-op outside practice
    /// (`suspendedEngine` is nil), so it's safe to call from every exit path.
    private func restoreEngineAfterPractice() {
        if let real = suspendedEngine {
            engine = real
            suspendedEngine = nil
        }
    }

    /// Exit an active practice run to the board WITHOUT logging it or flipping a
    /// job. Returns true if a practice run was active (the caller should stop).
    @discardableResult
    private func exitPracticeIfActive() -> Bool {
        guard isPracticeWalk else { return false }
        isPracticeWalk = false
        restoreEngineAfterPractice()
        shareURL = nil
        document = nil
        notes = nil
        phase = .board
        path = []
        return true
    }

    // MARK: Trade switching (validation strategy: same bones, swappable template)

    func switchTrade(_ newTrade: TradeFixture) {
        trade = newTrade
        jobs = newTrade.jobs
    }

    // MARK: Walk lifecycle

    /// Voice walks gate on mic permission BEFORE the session starts — a walk
    /// that can't hear must never begin (same posture as throwing begin()).
    /// Returns immediately when already authorized; first-ever tap shows the
    /// system prompt. Denied → `micDenied` surfaces on the board.
    ///
    /// The free-tier meter gates here too, and for the same reason: both are
    /// conditions that must be settled BEFORE the operator starts talking.
    /// Refusing at finish would destroy a recording they had already made.
    func startWalk() {
        // ONE walk at a time, enforced synchronously.
        //
        // The voice path defers `beginWalk()` behind `await
        // requestPermissions()`, so `phase` is still `.board` when a second tap
        // lands — a phase guard alone cannot catch it. Two taps therefore built
        // TWO `AudioCaptureSource`s, each with its own `AVAudioEngine`, each
        // installing a tap on the shared mic input. The second
        // `installTapOnBus` raises an ObjC exception, which is an abort():
        // SIGABRT on the first thing START WALK does (crash report, build 93).
        //
        // Cleared by `beginWalk` once the walk is actually live, and by every
        // failure path below, so a refused permission or a blocked meter does
        // not wedge the button.
        guard !isStartingWalk else { return }
        isStartingWalk = true

        // Two exemptions, both principled rather than convenient.
        //
        // Practice walks: they run on a throwaway engine, are never saved, and
        // are part of onboarding. Charging someone a walk to learn how the app
        // works — or blocking the tutorial at the limit — would be indefensible.
        //
        // Demo mode: the scripted DemoWalkEngine makes no model calls at all, so
        // it costs nothing, and the free tier exists to bound cost. It is also
        // dev-only (`demo=1`), and metering it would break the multi-round
        // autoflow QA runs at walk six.
        if !isPracticeWalk && walkMode != .demo {
            switch WalkAllowance.decide(
                isPro: entitlement.isPro,
                record: WalkMeter.load(),
                // No purchasable product means no way out of the limit — so the
                // limit does not apply. See WalkAllowance.decide.
                canSubscribe: entitlement.canSubscribe
            ) {
            case .blocked(let used, let limit):
                blockedUsage = (used: used, limit: limit)
                paywallOfferFreeLeft = nil
                showPaywall = true
                isStartingWalk = false
                return
            case .allowed(let remaining):
                // Surfaced on the notes screen after the walk, not now: a
                // countdown on the START button would make every walk feel
                // rationed. `nil` = Pro, never counted.
                walksRemainingAfterThis = remaining
            }
        }
        if walkMode == .voice && !wavFixture && !isPracticeWalk {
            Task { [weak self] in
                guard let self else { return }
                if await AudioCaptureSource.requestPermissions() {
                    self.micDenied = false
                    self.beginWalk()
                } else {
                    self.micDenied = true
                    self.isStartingWalk = false
                }
            }
        } else {
            beginWalk()
        }
    }

    /// Plan 20 D9: paint the walk screen FIRST, then run the (warm ⇒ cheap)
    /// `begin` + wiring on the next main-actor turn. The paint-first block
    /// contains ONLY `phase`/`path`/`micStarting` — NOTHING session-dependent
    /// moves into it (the D9 numbered invariant, F3): STT/audio must never
    /// wire onto a session `begin` didn't successfully open (Plan 07
    /// dead-walk — a dead session would pump and silently drop every append).
    /// Keep the display awake for the duration of a walk. Without this the phone
    /// auto-locks in a pocket mid-walk, iOS suspends/kills the app, and the
    /// in-progress walk is discarded (the whole "pocket it and keep walking"
    /// flow). Toggled off at every walk exit so the board sleeps normally.
    private func keepScreenAwake(_ on: Bool) {
        UIApplication.shared.isIdleTimerDisabled = on
    }

    private func beginWalk() {
        pumpTask?.cancel()
        eventTask?.cancel()

        // Paint-first block (D9): screen appears immediately; WalkView shows
        // "MIC STARTING…" until step 4 below clears it.
        micStarting = true
        micUnavailable = false
        isStartingWalk = false   // the walk is live; START WALK is armed again
        phase = .walking
        path = [.walking]
        keepScreenAwake(true)   // don't let the phone sleep + kill the walk

        Task { [weak self] in
            guard let self else { return }
            // Yield so SwiftUI renders the painted walk screen before the
            // begin work runs on this same main actor.
            await Task.yield()

            // (1) begin() is throwing (review P1): fallible across FFI. On a
            // throw, do NOT run steps 2–4 — revert to the board with the log
            // breadcrumb (the existing stay-on-board posture). F6 accepted
            // cosmetic: the screen briefly painted .walking and flashes back;
            // re-serializing the paint behind begin would forfeit the whole
            // D9 perceived-latency win. sac: visible error chrome is yours.
            let events: AsyncStream<WalkEvent>
            do {
                events = try self.engine.begin(trade: self.trade)
            } catch {
                Logger(subsystem: Bundle.main.bundleIdentifier ?? "sitewalk", category: "walk")
                    .error("startWalk: engine.begin failed, back to board: \(error, privacy: .public)")
                self.micStarting = false
                self.keepScreenAwake(false)
                self.phase = .board
                self.path = []
                return
            }

            // (2) Snapshot the session id STRICTLY AFTER begin returns Ok,
            // while the engine's live session still has one (Plan 11 D7).
            // Must not precede (1); must not sit in the paint-first block.
            self.currentSessionId = self.engine.currentSessionId

            // (3) Reset walk state + wire the event/audio tasks — all
            // session-dependent, so all strictly after the begin gate.
            self.transcript = ""
            self.previewTail = ""
            self.items = []
            self.isPaused = false
            self.walkStart = Date()
            self.photos = []

            self.eventTask = Task { [weak self] in
                guard let self else { return }
                for await event in events {
                    switch event {
                    case .boardUpdated(let items):
                        withAnimation(.easeOut(duration: 0.25)) { self.items = items }
                        // Track the newest by id, NOT array position (see
                        // `lastCapturedID` doc comment).
                        self.lastCapturedID = items.last?.id
                    case .transcriptCommitted(let text):
                        // The audio path's transcript originates in Rust (whisper).
                        self.transcript += text
                        self.previewTail = ""
                    case .transcriptPreview(let text):
                        self.previewTail = text
                    }
                }
            }

            // Practice runs play the scripted demo content regardless of the
            // persisted mode (a first-timer needn't know what to say).
            if self.walkMode == .demo || self.isPracticeWalk {
                self.startScriptedSource()
            } else {
                self.startAudioSource()
            }

            // (4) live.
            self.micStarting = false
        }
    }

    /// Text/demo path: canned transcript → engine.append (unchanged).
    private func startScriptedSource() {
        let src = makeScriptedSource(trade)
        source = src
        // STOP before dropping the reference. Assigning nil over a live
        // AudioCaptureSource orphans it with its mic tap still installed, and
        // the next real walk's `installTap` then aborts.
        audioSource?.stop()
        audioSource = nil
        pumpTask = Task { [weak self] in
            guard let self else { return }
            for await chunk in src.chunks {
                self.transcript += chunk
                self.engine.append(transcript: chunk)
            }
        }
        src.start()
    }

    /// Live path (Plan 08): PCM → engine.pushAudio; the transcript comes back
    /// via transcriptCommitted events (no src.chunks, so no pumpTask — the two
    /// paths never both feed the transcript). `wavFixture` picks the mic-free
    /// bundled WAV over the live mic (D7).
    private func startAudioSource() {
        source = nil
        let onSamples: @Sendable ([Float]) -> Void = { [weak self] samples in
            Task { @MainActor in self?.engine.pushAudio(samples) }
        }
        let audio: any PCMAudioSource
        if wavFixture {
            audio = WavFileAudioSource(pushSamples: onSamples)   // mic-free fixture (D7)
        } else {
            let mic = AudioCaptureSource(pushSamples: onSamples, voiceProcessing: voiceProcessing)
            // A walk whose mic never opened must SAY so. Sitting on a live
            // RECORDING screen capturing silence is worse than the crash it
            // replaced: the operator talks through a whole site and gets
            // nothing, with no clue why.
            mic.onUnavailable = { [weak self] in
                Task { @MainActor in
                    self?.micUnavailable = true
                    self?.micStarting = false
                }
            }
            audio = mic
        }
        // Same reasoning as `startScriptedSource`: never replace a live source
        // without stopping it first.
        audioSource?.stop()
        audioSource = audio
        audio.start()
    }

    func togglePause() {
        isPaused.toggle()
        if isPaused {
            source?.pause()
            audioSource?.pause()
        } else {
            source?.resume()
            audioSource?.resume()
        }
    }

    /// True while a walk is auto-paused because the app went to the background
    /// (distinct from a user's manual PAUSE) — so we auto-resume on return
    /// without overriding a deliberate pause.
    private(set) var pausedByBackground = false

    /// Pause/resume STT across app background transitions. Whisper runs its
    /// encode on the **Metal GPU**, and iOS forbids GPU work while an app is
    /// backgrounded — a compute that runs in the background hits `ggml_abort`
    /// (SIGABRT) and crashes the walk (field crash, build 56). Since the
    /// pocket-recording fix (#248) keeps the app alive in the background instead
    /// of suspending it, we must stop feeding whisper when we lose the
    /// foreground: pause on background, auto-resume on return. (The keep-screen-
    /// awake in #248 means a normal pocketed walk never backgrounds; this covers
    /// the manual power-button lock / incoming call / app-switch cases.)
    func handleBackgroundTransition(backgrounded: Bool) {
        guard phase == .walking else { return }
        if backgrounded {
            guard !isPaused else { return }   // don't fight a manual pause
            isPaused = true
            // Positively HALT the Rust STT pump first: pausing the audio source
            // only stops NEW PCM, but the pump can still decode already-buffered
            // windows on the Metal GPU — and a Metal decode submitted while
            // backgrounded is the ggml_abort/SIGABRT crash. pausePump() gates
            // the pump so no new decode starts while we're out of foreground
            // (the durable, core-side half of #253's mitigation).
            engine.pausePump()
            source?.pause()
            audioSource?.pause()
            pausedByBackground = true
        } else if pausedByBackground {
            pausedByBackground = false
            isPaused = false
            source?.resume()
            audioSource?.resume()
            engine.resumePump()
        }
    }

    /// Walk-time capture (Plan 11 D7 / sac design pass): one tap, zero
    /// confirm — the shot pins to the item being spoken (`lastCapturedID`,
    /// dam's D3 rule) or attaches session-level when the board is still
    /// empty. Core ids are canonical-lowercase UUIDv7; `UUID.uuidString`
    /// is uppercase, so lowercase across the seam. The chip bump is
    /// optimistic — the next `boardUpdated` carries the core's
    /// `photoCount` and self-corrects.
    ///
    /// PR #176 should-fix: `capturePhoto` now runs its bytes-write + FFI
    /// attach off the main actor and reports back via `onComplete` instead
    /// of finishing synchronously. `pinnedID` is snapshotted HERE, before
    /// the async work starts — matching the old code's implicit behavior
    /// (it read `lastCapturedID` right after a fully synchronous
    /// `capturePhoto` call, so it always saw the same value it captured
    /// with). Re-reading `lastCapturedID` from the completion closure
    /// instead would risk bumping the WRONG item if a `boardUpdated` lands
    /// while the attach is in flight.
    func addPhoto(_ data: Data) {
        let pinnedID = lastCapturedID
        capturePhoto(image: data, itemId: pinnedID?.uuidString.lowercased()) { [weak self] success in
            guard success, let self,
                  let id = pinnedID,
                  let idx = self.items.firstIndex(where: { $0.id == id }) else { return }
            self.items[idx].photos += 1
        }
    }

    func discardWalk() {
        keepScreenAwake(false)
        source?.abort()
        audioSource?.stop()
        pumpTask?.cancel()
        eventTask?.cancel()
        // Tell RUST to stop the pump + tombstone the session (Plan 08 Task 4):
        // without this the pump thread AND the Recording/item/artifact rows
        // leak (issue #3). Fire-and-forget off the main actor — the async Rust
        // cancel() spawn_blocking-joins the pump, so the UI never blocks. Reset
        // the Swift state synchronously below; the Rust teardown rides the Task.
        let engine = self.engine
        Task { await engine.cancel() }
        source = nil
        audioSource = nil
        transcript = ""
        previewTail = ""
        items = []
        isPaused = false
        currentSessionId = nil // the session was just tombstoned in Rust
        photos = []
        notes = nil
        documentBuildError = nil
        notesEditError = nil
        isPracticeWalk = false
        // Restore AFTER the local `engine` capture above cancelled the
        // throwaway demo session (F1); safe no-op for a real discard.
        restoreEngineAfterPractice()
        phase = .board
        path = []
    }

    /// Plan 13 D1: DONE ends the walk and computes NOTES — not a document.
    /// The walk's items + summary land on the notes screen immediately; a
    /// document is built later, deliberately, by a `buildPrimaryDocument()`
    /// tap (or a future per-kind button — sac's taxonomy).
    /// True while finish() is computing the notes — the notes screen shows a
    /// skeleton + top progress bar. dam's UX note: navigate ONCE (straight to
    /// the notes phase) and fill in place, rather than a separate building
    /// screen that then shifts to notes (the layout-shift he flagged).
    var notesLoading = false

    func finishWalk() {
        // Captured BEFORE the async work: a practice walk must never be metered,
        // and `isPracticeWalk` is cleared by the exit paths that may run first.
        let metered = !isPracticeWalk && walkMode != .demo
        keepScreenAwake(false)   // recording's done — let the phone sleep normally
        source?.stop()
        audioSource?.stop()
        // Navigate once, immediately, into the notes phase in a loading state.
        notes = nil
        notesLoading = true
        phase = .notes
        path = [.notes]
        // A fresh finish — the queued banner may honestly promise the
        // reconnect retry (Plan 20 F5; the reopen path sets `.reopened`).
        notesBannerReason = .liveFinish
        Task {
            // Flush before finish (issue #155 / CANON: flush over speed —
            // the last words of a walk are often the price). `stop()` lets a
            // final speech result land (grace-bounded by the source); the
            // pump ends when the source's stream finishes, so awaiting it
            // guarantees every flushed chunk reached engine.append first.
            _ = await pumpTask?.value
            let notes = await engine.finish()
            self.notes = notes
            self.notesLoading = false
            // Metered on OUTPUT, not on tapping START: the count moves only
            // once a walk has actually produced notes. Someone who starts a
            // walk, realizes they're at the wrong address, and discards it has
            // received nothing and must not be charged for it. The rest of the
            // rule — Pro, practice, demo, and the `canSubscribe` clause that
            // stops a whole TestFlight cohort burning its allowance against a
            // product nobody can buy — is `WalkAllowance.shouldCount`.
            if WalkAllowance.shouldCount(
                isPro: self.entitlement.isPro,
                canSubscribe: self.entitlement.canSubscribe,
                isMeteredWalk: metered
            ) {
                WalkMeter.recordFinishedWalk()
            }
            // A FINISHED walk is a walk, whether or not a document was ever
            // built from it. Previously a walk only entered the log via
            // `completeSend()` — so finishing and then just filing it left it
            // invisible on the board (Isaac's field report: "I didn't export
            // the notes or create a document... it should save regardless").
            // Core already persisted it at finish; the log simply never
            // re-read.
            self.refreshWalkLog()
            // R4's inference half, client-side: if the operator said the job's
            // name during the walk, file it there and SHOW that we did.
            self.autoFileFromTranscript()
        }
    }

    /// Files the just-finished walk under a job whose name the operator spoke.
    ///
    /// Isaac: "The AI should also auto file it if it hears the name of the site
    /// in the walk." It doesn't need the model to do it — the job names are
    /// already on the device and the transcript is right here, so this is a
    /// deterministic string match: no token cost, no latency, no chance of a
    /// hallucinated job.
    ///
    /// It auto-files rather than merely suggesting, which is what was asked
    /// for. The safety condition is that the result is always VISIBLE and
    /// one tap from changeable on the notes screen — a silent mis-file would
    /// bury the walk somewhere the operator never looks, which is the one
    /// outcome worse than not filing at all. Hence: match only on an
    /// unambiguous, whole-name hit, and never overwrite an existing filing.
    func autoFileFromTranscript() {
        guard let sessionId = currentSessionId,
              !isPracticeWalk,
              let match = Self.jobMatching(transcript: transcript, jobs: activeJobsForMatching())
        else { return }
        // Never clobber a deliberate choice.
        if let existing = sessionWalks.first(where: { $0.sessionId == sessionId }), existing.jobId != nil {
            return
        }
        try? setWalkJob(sessionId: sessionId, jobId: match.id)
        autoFiled = (sessionId: sessionId, jobName: match.name)
    }

    /// Set when a finish auto-filed, so the notes screen can say so rather
    /// than silently changing state under the operator.
    ///
    /// SESSION-SCOPED, not a bare name. A bare name never went stale on its
    /// own and the view's "does it match the current filing?" check let a
    /// false positive through: auto-file walk A to a job, then MANUALLY file
    /// walk B to that same job, and the notes screen would claim it heard the
    /// name in walk B. Claiming to have heard something we didn't is exactly
    /// the failure this message exists to prevent, so the id is carried and
    /// compared instead of hoping a reset fires everywhere.
    var autoFiled: (sessionId: String, jobName: String)?

    private func activeJobsForMatching() -> [JobModel] {
        ((try? engine.listJobs()) ?? []).filter { $0.status == .active }
    }

    /// The single job whose name the transcript clearly contains, if any.
    ///
    /// Normalizes both sides (case, punctuation, whitespace) so "117 Lexington"
    /// matches "117 lexington." mid-sentence. Requires a whole-name hit and
    /// UNIQUENESS — if two jobs both match, we don't guess; picking one would
    /// be a coin flip that hides the walk from the other. Very short names are
    /// skipped because a 2-3 character name matches almost any transcript by
    /// accident.
    /// `nonisolated` because it is pure — no state, no side effects. Binding a
    /// string comparison to the main actor would be noise, and it makes the
    /// matcher directly unit-testable off the main thread.
    nonisolated static func jobMatching(transcript: String, jobs: [JobModel]) -> JobModel? {
        func normalize(_ s: String) -> String {
            let folded = s.folding(options: [.diacriticInsensitive, .caseInsensitive], locale: .current)
            let cleaned = folded.map { $0.isLetter || $0.isNumber ? $0 : " " }
            return String(cleaned).split(separator: " ").joined(separator: " ")
        }
        let haystack = normalize(transcript)
        guard !haystack.isEmpty else { return nil }
        let hits = jobs.filter { job in
            let needle = normalize(job.name)
            // 4 chars is the floor: shorter names ("A", "12") collide with
            // ordinary speech and would file walks essentially at random.
            guard needle.count >= 4 else { return false }
            return haystack.contains(needle)
        }
        // Ambiguity means don't guess. One clear hit, or nothing.
        guard hits.count == 1 else { return nil }
        return hits.first
    }

    // MARK: Walk reopen (Plan 20 Half A) — read-only re-entry into NotesView.

    /// Hydrate the board walk log from the engine (Plan 20 D5/F2) so history
    /// survives relaunch. Called once from the app-open path — read-only
    /// (`listSessions` mutates nothing), safe alongside the sweeps (R4).
    /// TWO guards (F2):
    ///  1. once-per-process (`isHydratingWalkLog`) against `.task` re-fires;
    ///  2. overwrite only when the fetched list is NON-EMPTY or the engine is
    ///     real-core — `DemoWalkEngine.listSessions` returns `[]`
    ///     SUCCESSFULLY (a `??` fallback never fires), and that empty success
    ///     must not wipe the demo's in-memory log. Real-core's `[]` is a
    ///     legitimate "no sessions yet" and may clear a stale log.
    func hydrateWalkLog() {
        guard !isHydratingWalkLog else { return }
        isHydratingWalkLog = true
        refreshWalkLog()
    }

    /// Seeds a few finished walks for headless capture (`seedwalks=1`), in the
    /// established QA-hook style of `autoprofile` / `autobrand`.
    ///
    /// Two jobs it does, and both are real:
    ///
    /// 1. **App Store screenshots.** The board is the first screen a reviewer
    ///    and a buyer see, and `DemoWalkEngine.listSessions()` returns `[]` by
    ///    design — its log is in-memory — so an unattended run always captures
    ///    an empty board reading "NO WALKS YET".
    /// 2. **It makes the #221 row rendering visible at all.** Those labels are
    ///    a real-core path, so on-sim there was no way to look at a hydrated
    ///    row; the fix shipped unit-tested but unseen.
    ///
    /// Deliberately fake and deliberately obvious: a fixed date so captures are
    /// reproducible, and summaries written as a real walk would read.
    func seedDemoWalks(now: Date = Date()) {
        let day: UInt64 = 86_400
        let base = UInt64(now.timeIntervalSince1970)
        sessionWalks = [
            WalkRecord(
                time: "9:41", docNo: "", docKind: "ESTIMATE", sent: true,
                sessionId: "seed-1", queued: false, jobId: nil,
                startedAt: base - 3_600,
                summary: "1418 Alder Ct, scoped mulch and trim. "
                    + "The operator noted three yards of hardwood mulch, four boxwoods "
                    + "along the walkway, and a broken zone-2 irrigation head.",
                itemCount: 5
            ),
            WalkRecord(
                time: "8:05", docNo: "", docKind: "", sent: false,
                sessionId: "seed-2", queued: false, jobId: nil,
                startedAt: base - 5_400,
                summary: "No actionable session content was captured. The transcript "
                    + "contained only ambient noise with no discernible speech.",
                itemCount: 3
            ),
            WalkRecord(
                time: "16:20", docNo: "", docKind: "WORK ORDER", sent: true,
                sessionId: "seed-3", queued: false, jobId: nil,
                startedAt: base - (2 * day),
                summary: "Marston HOA irrigation check",
                itemCount: 6
            ),
        ]
    }

    /// Re-read the walk log NOW, bypassing the once-per-process latch.
    ///
    /// `isHydratingWalkLog` is set once and never cleared — deliberately, to
    /// stop `.task` re-fires from re-hydrating. But that made every LATER
    /// refresh a silent no-op, which is the bug Isaac hit: filing a walk under
    /// a job wrote through to core correctly and the board never showed it,
    /// because the only refresh path was latched off for the life of the
    /// process.
    ///
    /// Guard 2 is preserved: `DemoWalkEngine.listSessions()` returns `[]`
    /// SUCCESSFULLY, and that empty success must not wipe the demo's in-memory
    /// log. Real-core's `[]` is a legitimate "no sessions yet".
    func refreshWalkLog() {
        let fetched = (try? engine.listSessions()) ?? []
        let isRealCore = !(engine is DemoWalkEngine)
        if !fetched.isEmpty || isRealCore {
            sessionWalks = fetched.map(WalkRecord.init)
        }
    }

    /// One dated group of loose walks for the board's top list.
    struct WalkSection: Identifiable {
        /// The title doubles as a stable id — at most one TODAY and one
        /// EARLIER section exist at a time.
        var id: String { title }
        let title: String
        let walks: [WalkRecord]
    }

    /// Walks not yet filed under any job — the board's top list. A filed walk
    /// lives under its job card only (see `BoardView.walks(for:)`), so it must
    /// NOT also appear loose up top. Operator report 2026-07-27 ("random
    /// walks"): the old top list was unfiltered, so a filed walk showed twice
    /// (loose up top AND under its job), which read as clutter/duplication.
    var looseWalks: [WalkRecord] {
        sessionWalks.filter { $0.jobId == nil }
    }

    /// The board's top list, split into honest date groups so a multi-day
    /// history isn't piled under a single hardcoded "TODAY" (operator report
    /// 2026-07-27: days-old walks under "TODAY" read as "a bunch of random
    /// walks"). Newest-first order within each group is preserved from
    /// `sessionWalks`; empty groups are dropped.
    var looseWalkSections: [WalkSection] {
        Self.looseWalkSections(from: sessionWalks, now: Date(), calendar: .current)
    }

    /// The full board top-list pipeline: keep only unfiled walks, then split
    /// them into TODAY / EARLIER. Pure and `nonisolated` so the whole thing is
    /// unit-testable with a fixed `now`/`calendar` (no wall clock, no live
    /// `sessionWalks`).
    nonisolated static func looseWalkSections(
        from walks: [WalkRecord], now: Date, calendar: Calendar
    ) -> [WalkSection] {
        groupWalksByDay(walks.filter { $0.jobId == nil }, now: now, calendar: calendar)
    }

    /// Split walks into TODAY / EARLIER by calendar day. Pure and `nonisolated`
    /// so it's unit-testable with a fixed `now`/`calendar` (no wall clock).
    nonisolated static func groupWalksByDay(
        _ walks: [WalkRecord], now: Date, calendar: Calendar
    ) -> [WalkSection] {
        let startOfToday = calendar.startOfDay(for: now)
        var today: [WalkRecord] = []
        var earlier: [WalkRecord] = []
        for walk in walks {
            let day = Date(timeIntervalSince1970: TimeInterval(walk.startedAt))
            if day >= startOfToday { today.append(walk) } else { earlier.append(walk) }
        }
        var sections: [WalkSection] = []
        if !today.isEmpty { sections.append(WalkSection(title: "TODAY", walks: today)) }
        if !earlier.isEmpty { sections.append(WalkSection(title: "EARLIER", walks: earlier)) }
        return sections
    }

    /// Files a walk under a job (or unfiles it with nil), then refreshes.
    ///
    /// Updates the in-memory record BEFORE re-reading, because the demo engine's
    /// `listSessions()` returns `[]` by design (its log is in-memory) and
    /// `hydrateWalkLog` deliberately declines to let that empty result wipe the
    /// log. Without the optimistic update, filing would persist in the demo
    /// engine and never appear — which would make the whole feature undemoable.
    /// On real core the subsequent hydrate re-reads authoritative state, so the
    /// optimistic value is immediately replaced by the stored one.
    func setWalkJob(sessionId: String, jobId: String?) throws {
        try engine.setSessionJob(sessionId: sessionId, jobId: jobId)
        if let index = sessionWalks.firstIndex(where: { $0.sessionId == sessionId }) {
            sessionWalks[index].jobId = jobId
        }
        refreshWalkLog()
    }

    /// Reopen a finished walk from the board into the EXISTING NotesView
    /// (Plan 20 D5): `loadNotes` re-reads the same payload `finish()`
    /// returned, `currentSessionId` re-keys buildDocument/edits, and the nav
    /// path is `[.notes]` — back returns to the board root, never to a live
    /// walk. Read-only re-entry: no pump, no `.walking`, no resurrection.
    /// F4: a NotFound/tombstoned race (the row was deleted/swept between
    /// hydrate and tap) mirrors `buildDocument`'s catch — log + breadcrumb,
    /// board stays put, never a silent dead tap.
    func reopenWalk(sessionId: String) {
        guard phase == .board, !sessionId.isEmpty else { return }
        reopenError = nil
        Task {
            do {
                let loaded = try await engine.loadNotes(sessionId: sessionId)
                self.notes = loaded
                self.notesLoading = false
                self.notesBannerReason = .reopened
                self.currentSessionId = sessionId
                // Rehydrate the gallery for the reopened walk (jefe-2026-07-24):
                // its photos live in the store, not in the stale in-memory
                // `photos` array left over from whatever was on screen before.
                self.loadPhotos(sessionId: sessionId)
                self.phase = .notes
                self.path = [.notes]
            } catch {
                Logger(subsystem: Bundle.main.bundleIdentifier ?? "sitewalk", category: "walk")
                    .error("reopenWalk(\(sessionId, privacy: .public)) failed: \(error, privacy: .public)")
                self.reopenError = "Couldn’t reopen that walk. It may have been removed."
            }
        }
    }

    /// Leaves the notes screen without building a document (e.g. the "not
    /// now" / back-to-board path, or the empty-walk UX). The session already
    /// reached `Processed` inside `finish()` — nothing to tombstone here,
    /// unlike `discardWalk()` (which cancels a still-live session). Just
    /// resets local UI state.
    func dismissNotes() {
        let produced = notes
        isPracticeWalk = false
        restoreEngineAfterPractice()
        notes = nil
        documentBuildError = nil
        notesEditError = nil
        reviewKind = nil
        currentSessionId = nil
        notesBannerReason = .liveFinish
        phase = .board
        path = []
        offerProAfterFirstWalk(produced)
    }

    /// Which doc kind is currently building (for a per-button spinner); nil
    /// when idle. `isBuildingDocument` stays as the any-build flag.
    var buildingKind: String?

    /// The kind whose document is on the review screen — labels the review
    /// header ("ESTIMATE" / "INVOICE" …) so the back arrow reads clearly.
    var reviewKind: String?

    /// Build the finished document for an explicit `kind` (Plan 13 Stage-1 FFI
    /// `build_document`) and route to the existing ReviewView. Engine-keyed
    /// via `currentSessionId` (snapshotted at walk start, kept through review —
    /// works from history too). Each notes-screen action button passes its own
    /// legal kind; illegal-kind / non-Processed errors surface on the notes
    /// screen and leave the button available to retry, never crash.
    func buildDocument(kind: String) {
        guard let sessionId = currentSessionId, !isBuildingDocument else { return }
        documentBuildError = nil
        notesEditError = nil
        isBuildingDocument = true
        buildingKind = kind
        Task {
            defer { isBuildingDocument = false; buildingKind = nil }
            do {
                let doc = try await engine.buildDocument(sessionId: sessionId, kind: kind)
                self.document = doc
                self.reviewKind = kind
                self.phase = .review
                // Push review ONTO the notes screen (not replace) so it has a
                // real back to notes — the operator can pick a different document
                // or re-read. Was `[.review]`, which dropped notes and left
                // Send/Discard as the only exits (the reported dead-end).
                self.path = [.notes, .review]
            } catch {
                Logger(subsystem: Bundle.main.bundleIdentifier ?? "sitewalk", category: "document")
                    .error("buildDocument(\(kind, privacy: .public)) failed: \(error, privacy: .public)")
                // Carry the REAL reason. The old message said "couldn't build
                // the report" for ANY custom kind (DocKinds.label falls through
                // to "Report" for anything it doesn't hardcode), so a failure on
                // a custom document type named itself after the wrong document
                // and explained nothing. Isaac tapped a custom button and got
                // no usable signal at all.
                self.documentBuildError =
                    "Couldn’t build it. \(EngineErrorText.readable(error))"
            }
        }
    }

    /// Builds the template's lead document kind.
    ///
    /// `doc=<kind>` overrides it — a QA hook (parallel to `autoflow`/`autopdf`)
    /// so a headless run can reach any document type. Without it only the lead
    /// kind was reachable unattended, which is why #222 — the demo engine
    /// ignoring `kind` entirely — survived as long as it did: nothing that ran
    /// without hands ever built a second kind. Store screenshots need this too.
    func buildPrimaryDocument() {
        let arg = ProcessInfo.processInfo.arguments.first { $0.hasPrefix("doc=") }
        let kind = arg.map { String($0.dropFirst("doc=".count)) }
            ?? DocKinds.primaryKind(for: trade.key)
        buildDocument(kind: kind)
    }

    // MARK: Item edits (Plan 16 CRUD) — the walk is a first draft; the operator
    // fixes it here. These go through the CORE (not app-side pixels), so a
    // correction reaches every rebuilt document. The core stores lowercase
    // UUIDv7 ids and does a case-sensitive lookup, but Swift's `uuidString` is
    // uppercase — so the id is lowercased on the way out.

    /// Fix a captured line's text and/or quantity (`right`).
    func editItem(_ item: CapturedFixture, text: String, right: String) {
        guard let sessionId = currentSessionId else { return }
        notesEditError = nil
        do {
            _ = try engine.updateItem(
                sessionId: sessionId, itemId: item.id.uuidString.lowercased(),
                text: text, kind: nil, right: right
            )
            refreshNotes(sessionId: sessionId)
        } catch {
            itemEditFailed("save", error)
        }
    }

    /// Add a manual line. Appends last (Plan 16 UUIDv7 ordering); `kind:"note"`
    /// → the plain/ITEM tag.
    func addNoteItem(text: String, right: String) {
        guard let sessionId = currentSessionId else { return }
        notesEditError = nil
        do {
            _ = try engine.addItem(sessionId: sessionId, kind: "note", text: text, right: right)
            refreshNotes(sessionId: sessionId)
        } catch {
            itemEditFailed("add", error)
        }
    }

    /// Remove a line — a tombstone retraction (drops from every rebuilt document,
    /// distinct from `done`).
    func removeNoteItem(_ item: CapturedFixture) {
        guard let sessionId = currentSessionId else { return }
        notesEditError = nil
        do {
            try engine.removeItem(sessionId: sessionId, itemId: item.id.uuidString.lowercased())
            refreshNotes(sessionId: sessionId)
        } catch {
            itemEditFailed("remove", error)
        }
    }

    /// Plan 20 D4 (the Plan 16 clause-(b) contract, finally honored): after a
    /// successful mutation the notes screen RE-READS from the engine via
    /// `loadNotes` — the only sanctioned post-edit path — instead of patching
    /// `notes.items` in place (the echo-as-truth anti-pattern). A failed
    /// re-read keeps the current screen state and logs; the next edit or
    /// reopen re-reads again.
    private func refreshNotes(sessionId: String) {
        Task {
            do {
                self.notes = try await engine.loadNotes(sessionId: sessionId)
            } catch {
                Logger(subsystem: Bundle.main.bundleIdentifier ?? "sitewalk", category: "items")
                    .error("post-edit loadNotes failed: \(error, privacy: .public)")
            }
        }
    }

    private func itemEditFailed(_ what: String, _ error: Error) {
        notesEditError = "Couldn’t \(what) that line. Try again."
        Logger(subsystem: Bundle.main.bundleIdentifier ?? "sitewalk", category: "items")
            .error("item \(what, privacy: .public) failed: \(error, privacy: .public)")
    }

    /// Review → back to notes (the review screen's back arrow). Keeps the
    /// session and the built notes intact so the operator can build a different
    /// document or re-read; the last document stays in memory until the next
    /// build overwrites it. Pops just the review frame (board → notes).
    func backToNotes() {
        documentBuildError = nil
        notesEditError = nil
        reviewKind = nil
        phase = .notes
        path = [.notes]
    }

    // MARK: Vocabulary (Plan 10) — the write half of the vocabulary → STT
    // biasing loop. Defensive: a thrown FFI error becomes a logged breadcrumb +
    // an unchanged list, never a crash (the editor may show `vocabularyError`).

    private var vocabularyLogger: Logger {
        Logger(subsystem: Bundle.main.bundleIdentifier ?? "sitewalk", category: "vocabulary")
    }

    func loadVocabulary() {
        do {
            vocabulary = try engine.listVocabulary()
        } catch {
            vocabularyLogger.error("loadVocabulary failed: \(error, privacy: .public)")
            vocabularyError = "\(error)"
        }
    }

    func addVocabulary(_ term: String) {
        do {
            vocabulary = try engine.addVocabularyTerm(term)
            vocabularyError = nil
        } catch {
            // sac: how errors surface (full-at-100, empty) is a design call.
            vocabularyLogger.error("addVocabulary failed: \(error, privacy: .public)")
            vocabularyError = "\(error)"
        }
    }

    func removeVocabulary(_ term: String) {
        do {
            vocabulary = try engine.removeVocabularyTerm(term)
            vocabularyError = nil
        } catch {
            vocabularyLogger.error("removeVocabulary failed: \(error, privacy: .public)")
            vocabularyError = "\(error)"
        }
    }

    /// Plan 15: apply the vocab card's DONE — the confirmed chips go through
    /// `seedVocabulary` (one batch, idempotent per pack), then each free-form
    /// term through the existing `addVocabularyTerm` CRUD. The CRUD THROWS at
    /// Full/Empty/TooLong, so a naive loop would abort the whole batch on the
    /// first bad term — instead it is per-term catch-and-continue with a
    /// surfaced skipped count. Returns a confirmation line for the card
    /// ("Added N" / "Added N, skipped M"). // sac: confirmation copy is yours.
    func applyVocabSeed(pack: VocabPack, confirmedChips: [String], freeform: [String]) -> String {
        var addedCount = 0
        var skippedCount = 0
        do {
            let report = try engine.seedVocabulary(
                trade: pack.trade, version: pack.version, terms: confirmedChips
            )
            vocabulary = report.terms
            addedCount += Int(report.added)
            skippedCount += Int(report.skippedOverBudget + report.skippedFull)
        } catch {
            vocabularyLogger.error("seedVocabulary failed: \(error, privacy: .public)")
            skippedCount += confirmedChips.count
        }
        for term in freeform {
            do {
                vocabulary = try engine.addVocabularyTerm(term)
                addedCount += 1
            } catch {
                // Per-term catch-and-continue (Plan 15 Task 4): one over-cap or
                // bad term must not abort the rest of the batch.
                vocabularyLogger.error("seed free-form add failed: \(error, privacy: .public)")
                skippedCount += 1
            }
        }
        return skippedCount == 0
            ? "Added \(addedCount) terms"
            : "Added \(addedCount), skipped \(skippedCount)"
    }

    var elapsedLabel: String {
        let seconds = Int(Date().timeIntervalSince(walkStart))
        return String(format: "%02d:%02d", seconds / 60, seconds % 60)
    }

    // MARK: Profile-aware display (fixture values remain the no-profile
    // fallback so the demo/gallery QA path keeps working unchanged)

    private static func dateString(_ format: String) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = format
        return formatter.string(from: Date()).uppercased()
    }

    /// Board date stamp — the REAL day in profile mode ("WED — JUL 08"),
    /// the fixture's frozen day otherwise.
    var boardDateLabel: String {
        profile == nil ? trade.dateLabel : Self.dateString("EEE — MMM dd")
    }

    /// Board headline in profile mode — honest walk count, never fixture jobs.
    var sessionTitle: String {
        switch sessionWalks.count {
        case 0: return "Ready to walk"
        case 1: return "1 walk today"
        default: return "\(sessionWalks.count) walks today"
        }
    }

    var letterheadBiz: String { profile?.businessName ?? trade.biz }
    var letterheadSub: String { profile?.letterheadSub ?? trade.bizSub }
    /// Document date — real today in profile mode ("JUL 08 2026"); a
    /// profile-stamped letterhead with the fixture's frozen date would lie.
    var letterheadDate: String {
        profile == nil ? trade.docDate : Self.dateString("MMM dd yyyy")
    }

    private static func clockNow() -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "h:mm"
        return formatter.string(from: Date())
    }

    /// Board-log time from a core `started_at` (epoch seconds) — the hydrate
    /// path's counterpart of `clockNow()` (Plan 20 F7 mapping). `nonisolated`:
    /// pure value formatting, callable from `WalkRecord.init` (a nonisolated
    /// struct initializer).
    /// Date-aware label for a walk in a job card, where the list can span
    /// months. Today collapses to a clock time (a date would be noise for
    /// something that just happened); this year drops the year; anything older
    /// carries it.
    /// The first sentence of a walk summary, for a one-line board row.
    ///
    /// Isaac, on device 2026-07-29: *"Is there a way to make the walk
    /// description be more condensed? Max one sentence?"* The rows were showing
    /// things like *"Field session to discuss mulch work. Only the word 'mulch'
    /// was clearly audible in the recording, with no additional context provided
    /// about scope, timing, or constraints."* — three clauses of the model
    /// narrating its own difficulty, wrapping to two lines and crowding the
    /// board.
    ///
    /// The first sentence is almost always the one that says what the walk WAS;
    /// everything after it is elaboration that belongs on the notes screen, and
    /// the full summary is still there when the walk is opened. Nothing is lost,
    /// only deferred.
    ///
    /// A period alone is not a sentence end — `3 yd. of mulch` and `Alder Ct.`
    /// would both split wrongly. It has to be followed by whitespace and a
    /// capital (or end the string), and leave something long enough to be worth
    /// showing.
    nonisolated static func firstSentence(of summary: String, limit: Int = 56) -> String {
        let text = summary.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return "" }

        let sentence = Self.withoutLeadIn(Self.upToFirstTerminator(text))
        // Clamped: `prefix(_:)` traps on a negative length, so a caller passing
        // a bad limit would take the app down rather than render something ugly.
        // Found by fuzzing after a field crash report on build 93 — not the
        // cause of that one, but the same class of trap.
        let cap = max(0, limit)
        guard sentence.count > cap else { return sentence }

        // Still too long for one line: cut on a word boundary rather than
        // mid-word, so the ellipsis reads as "there's more" and not as damage.
        let head = sentence.prefix(cap)
        guard let lastSpace = head.lastIndex(of: " ") else {
            return String(head) + "…"
        }
        return sentence[sentence.startIndex..<lastSpace]
            .trimmingCharacters(in: .whitespaces) + "…"
    }

    /// Drops the boilerplate opener the summariser puts on almost every walk.
    ///
    /// Every summary in Isaac's on-device screenshots began the same way —
    /// "Field session to discuss mulch work", "Field session to procure
    /// materials", "A field session…". On a one-line row that prefix spends a
    /// third of the width before saying anything specific, and the operator
    /// already knows it was a field session; that is what the app is.
    ///
    /// A display-side trim, and knowingly a workaround: the real fix is the
    /// summariser not writing the phrase (filed for core). Kept deliberately
    /// short and anchored to the START of the string so it cannot eat content
    /// from the middle of a sentence.
    private nonisolated static func withoutLeadIn(_ sentence: String) -> String {
        let leadIns = [
            "Field session at ", "Field session to ", "Field session ",
            "A field session at ", "A field session to ", "A field session ",
            "This field session ", "The field session ",
            "Walk at ", "Site walk at ", "Site visit at ",
        ]
        for leadIn in leadIns where sentence.lowercased().hasPrefix(leadIn.lowercased()) {
            let trimmed = String(sentence.dropFirst(leadIn.count))
            // Never trim down to nothing, and never to a fragment too short to
            // mean anything — a summary that IS just the boilerplate keeps it.
            guard trimmed.count >= 12 else { return sentence }
            // Re-capitalise: the remainder was mid-sentence.
            return trimmed.prefix(1).uppercased() + trimmed.dropFirst()
        }
        return sentence
    }

    /// Everything up to the first real sentence terminator.
    private nonisolated static func upToFirstTerminator(_ text: String) -> String {
        /// Below this, a "sentence" is almost certainly an abbreviation we split
        /// on by mistake, so keep scanning.
        let minimumSentence = 16
        let characters = Array(text)
        for (index, character) in characters.enumerated() {
            guard character == "." || character == "!" || character == "?" else { continue }
            guard index + 1 >= minimumSentence else { continue }
            // End of string — the whole thing is one sentence.
            if index == characters.count - 1 {
                return String(characters[0...index])
            }
            // Needs whitespace then a capital to count as a break; that is what
            // keeps "3 yd. of mulch" and "Alder Ct. beds" intact.
            let next = characters[index + 1]
            guard next == " " || next == "\n" else { continue }
            let after = characters[(index + 2)...].first { $0 != " " }
            if let after, after.isUppercase || after.isNumber {
                return String(characters[0...index])
            }
        }
        return text
    }

    nonisolated static func walkDateLabel(epochSeconds: UInt64, now: Date = Date()) -> String {
        guard epochSeconds > 0 else { return "" }
        let date = Date(timeIntervalSince1970: TimeInterval(epochSeconds))
        let cal = Calendar.current
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        if cal.isDate(date, inSameDayAs: now) {
            formatter.dateFormat = "h:mm a"
        } else if cal.component(.year, from: date) == cal.component(.year, from: now) {
            formatter.dateFormat = "MMM d"
        } else {
            formatter.dateFormat = "MMM d yyyy"
        }
        return formatter.string(from: date).uppercased()
    }

    fileprivate nonisolated static func clockTime(epochSeconds: UInt64) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "h:mm"
        return formatter.string(from: Date(timeIntervalSince1970: TimeInterval(epochSeconds)))
    }

    // MARK: Review interactions

    /// Provenance for a line the operator added at review, kept off the walk's
    /// own vocabulary ("NOT HEARD", "FILLED BY YOU") because it is a different
    /// claim: this line was never spoken at all.
    static let addedLineNote = "ADDED BY YOU"

    func beginEdit(_ row: DocRowFixture) {
        editingRowID = row.id
        editTitle = row.title
        editingRowIsNew = false
        // Grouping separator stripped: the field is a decimal pad, which has
        // no comma key, so leaving one in gives the operator a value they
        // cannot retype.
        editText = row.amount.hasPrefix("$")
            ? row.amount.dropFirst().replacingOccurrences(of: ",", with: "")
            : ""
    }

    /// Appends a blank line and opens it for editing.
    ///
    /// The line is added to the document immediately rather than on commit so
    /// there is only ever one code path that writes a row — but it is marked
    /// `editingRowIsNew`, so a sheet dismissed without a description takes the
    /// blank row back out with it. An operator who taps ADD LINE and changes
    /// their mind should not have to then delete something.
    func addLine() {
        guard var doc = document else { return }
        let row = DocRowFixture(
            title: "",
            sub: Self.addedLineNote,
            subWarn: false,
            qty: "",
            amount: doc.pricesShown ? "——" : "",
            isGap: doc.pricesShown
        )
        doc.rows.append(row)
        document = doc
        editingRowID = row.id
        editTitle = ""
        editText = ""
        editingRowIsNew = true
    }

    /// Removes the line being edited. Deliberately not a swipe: this screen is
    /// read on a job site with gloves on, and a swipe that deletes a line off
    /// an estimate is too easy to do by accident while scrolling.
    func removeEditingLine() {
        guard let id = editingRowID, var doc = document else { return }
        doc.rows.removeAll { $0.id == id }
        document = doc
        editingRowID = nil
        editingRowIsNew = false
    }

    /// Writes the sheet back onto the line.
    ///
    /// Both fields are optional edits, and each one's EMPTY case is a real
    /// instruction rather than a no-op:
    /// - An emptied description on an existing line keeps the old one (there
    ///   is no such thing as an untitled line on a document); on a line just
    ///   added it means "never mind", and the line goes.
    /// - An emptied amount returns the line to an honest gap. Silently
    ///   keeping the old number would be the one failure this screen exists
    ///   to prevent — a price on a sent estimate that nobody agreed to.
    func commitEdit() {
        guard let id = editingRowID, var doc = document,
              let index = doc.rows.firstIndex(where: { $0.id == id }) else {
            editingRowID = nil
            editingRowIsNew = false
            return
        }
        let title = editTitle.trimmingCharacters(in: .whitespacesAndNewlines)
        if title.isEmpty && editingRowIsNew {
            removeEditingLine()
            return
        }
        let old = doc.rows[index]
        var row = old
        row.title = title.isEmpty ? old.title : title
        row.isEdit = false

        // A document that carries no money keeps its amount column empty
        // whatever is typed — a work order handed to a crew must not grow a
        // price (the same rule `DemoWalkEngine.buildDocument` documents), so
        // the sheet doesn't offer the field and this ignores it.
        if doc.pricesShown {
            let cleaned = editText
                .replacingOccurrences(of: ",", with: "")
                .replacingOccurrences(of: "$", with: "")
                .trimmingCharacters(in: .whitespaces)
            // A line the operator added themselves keeps saying so through
            // every edit. "FILLED BY YOU" and "NOT HEARD" are both claims
            // about what the WALK produced, and neither is true of a line the
            // walk never produced.
            let added = old.sub == Self.addedLineNote
            // Parsed as a decimal, not an Int: a line that already carries
            // cents ("$125.50") has to survive being opened and re-saved, and
            // an Int parse silently refused it.
            if let dollars = Double(cleaned), dollars > 0 {
                row.amount = DocumentModel.money(cents: Int((dollars * 100).rounded()))
                row.isGap = false
                row.subWarn = false
                if old.isGap && !added { row.sub = "FILLED BY YOU" }
            } else if cleaned.isEmpty {
                // Erased on purpose. Back to an honest gap — keeping the old
                // number would be the one failure this screen exists to
                // prevent, a price on a sent estimate nobody agreed to.
                row.amount = "——"
                row.isGap = true
                if !added {
                    row.sub = "NOT HEARD — TAP OR SAY IT"
                    row.subWarn = true
                }
            }
            // Anything else typed (letters, a stray symbol) leaves the amount
            // exactly as it was: we cannot read it, so we do not act on it.
        }

        // `itemId` rides along on the copy. The old code rebuilt the row field
        // by field and dropped it, which detached the row from the photos
        // taken against that item mid-walk (`ReviewView.photos(for:)` joins on
        // it) — so filling one gap quietly emptied that row's contact sheet.
        doc.rows[index] = row
        document = doc
        editingRowID = nil
        editingRowIsNew = false
    }

    // MARK: Send

    func makePDF() {
        guard let doc = document else { return }
        shareURL = DocumentPDF.render(
            trade: trade, document: doc,
            biz: letterheadBiz, bizSub: letterheadSub, docDate: letterheadDate,
            branding: branding, layout: documentLayout
        )
    }

    /// Persist branding edited in the Letterhead Studio and apply it live (the
    /// review sheet + every future PDF read `branding`).
    func saveBranding(_ updated: Branding) {
        branding = updated
        Branding.save(updated)
    }

    /// Persist business identity edited in the Letterhead Studio (name / city /
    /// license — the letterhead text set at onboarding, now editable anytime).
    /// `reloadProfile()` refreshes the board header + letterhead + trade.
    func saveProfile(_ updated: BusinessProfile) {
        BusinessProfile.save(updated)
        reloadProfile()
    }

    /// Persist the document structure basics (terms / signature) edited in the
    /// Letterhead Studio; the review sheet + every future PDF read them.
    func saveDocumentLayout(_ updated: DocumentLayout) {
        documentLayout = updated
        DocumentLayout.save(updated)
    }

    /// UserDefaults, not the keychain: re-showing this once after a reinstall is
    /// a shrug, and the keychain is reserved for things a reinstall must not
    /// reset (the walk meter, the install id).
    private static let firstWalkOfferKey = "jefe.offeredProAfterFirstWalk"

    /// Offer Pro once, the first time a walk has actually produced something and
    /// the operator is heading back to the board.
    ///
    /// **Every exit from a finished walk calls this** — closing the notes, or
    /// sending or discarding a document built from them, in a practice run or a
    /// real one. Isaac, 2026-08-08: *"after they finish their first walk in any
    /// way… if the user were to press the back arrow to go back to the home
    /// screen they should get the offer."*
    ///
    /// The earlier version fired only after the practice walk, which is an
    /// opt-in link under the primary button in onboarding — so the majority of
    /// operators, who tap START MY FIRST WALK, would never have seen it at all.
    ///
    /// The decision itself is `FirstWalkOffer` — a pure function so the three
    /// conditions are testable without a keychain, StoreKit or a walk. Catching
    /// the operator on the way OUT is the point: the offer must never interrupt
    /// the notes screen, which is the thing doing the selling.
    private func offerProAfterFirstWalk(_ produced: NotesModel?) {
        let defaults = UserDefaults.standard
        guard FirstWalkOffer.shouldOffer(
            isPro: entitlement.isPro,
            produced: produced,
            alreadyOffered: defaults.bool(forKey: Self.firstWalkOfferKey)
        ) else { return }
        defaults.set(true, forKey: Self.firstWalkOfferKey)

        let freeLeft = WalkAllowance.remaining(in: WalkMeter.load())
        Task { @MainActor in
            // Let the board's transition settle first; a sheet raised mid-
            // animation lands half-drawn.
            try? await Task.sleep(for: .milliseconds(700))
            guard !self.entitlement.isPro else { return }
            self.blockedUsage = nil
            self.paywallOfferFreeLeft = freeLeft
            self.showPaywall = true
        }
    }

    func completeSend() {
        let produced = notes
        // A practice run shows the whole loop (incl. the share sheet) but is
        // never recorded — no board log, no job flip.
        if exitPracticeIfActive() {
            offerProAfterFirstWalk(produced)
            return
        }
        if let index = jobs.firstIndex(where: { !$0.done }) {
            let old = jobs[index]
            jobs[index] = JobFixture(
                time: old.time, name: old.name, sub: old.sub,
                tag: TagFixture(kind: .green, label: "SENT"), done: true
            )
        }
        sessionWalks.append(WalkRecord(
            time: Self.clockNow(), docNo: trade.docNo, docKind: trade.docKind, sent: true,
            sessionId: currentSessionId ?? "", queued: notes?.queued ?? false,
            summary: notes?.summary ?? "", itemCount: notes?.items.count ?? 0
        ))
        shareURL = nil
        document = nil
        notes = nil
        phase = .board
        path = []
        offerProAfterFirstWalk(produced)
    }

    /// Abandon a reviewed document WITHOUT marking the job sent (issue #155:
    /// DISCARD previously routed through `completeSend()` and flipped the job
    /// to SENT). The persisted core artifact is untouched — only the app-side
    /// review state resets.
    func discardDocument() {
        let produced = notes
        if exitPracticeIfActive() {
            offerProAfterFirstWalk(produced)
            return
        }
        sessionWalks.append(WalkRecord(
            time: Self.clockNow(), docNo: trade.docNo, docKind: trade.docKind, sent: false,
            sessionId: currentSessionId ?? "", queued: notes?.queued ?? false,
            summary: notes?.summary ?? "", itemCount: notes?.items.count ?? 0
        ))
        shareURL = nil
        document = nil
        notes = nil
        phase = .board
        path = []
        offerProAfterFirstWalk(produced)
    }
}
