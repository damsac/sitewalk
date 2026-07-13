import SwiftUI
import Observation
import os

// One observable model drives the whole flow:
//   board → walking (pause/resume, photos) → building → notes → review (edit, fill gaps) → sent
// (Plan 13: `building` computes NOTES, not a document — `notes` is the
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
        case building
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

    /// One board row per walk finished THIS SESSION (profile mode replaces
    /// the fixture jobs list with this honest log). In-memory on purpose —
    /// walk history is a core concern; this is the interim surface.
    struct WalkRecord: Identifiable {
        let id = UUID()
        let time: String     // "9:41"
        let docNo: String
        let docKind: String  // "ESTIMATE" / "MOVE-OUT REPORT" / ...
        let sent: Bool       // false = discarded at review
    }
    private(set) var sessionWalks: [WalkRecord] = []

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
    /// True while a build-document tap is in flight — the notes screen
    /// disables the button so a double-tap can't burn two document numbers
    /// (D7: numbers mint per generate).
    var isBuildingDocument = false

    // Review state
    var document: DocumentModel?
    var editingRowID: UUID?
    var editText = ""
    var shareURL: URL?

    // Not fully `private`: read from AppModel+Photos.swift (a same-module
    // extension in a different file — Swift's `private` is file-scoped).
    private(set) var engine: WalkEngine
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
         voiceProcessing: Bool = false) {
        self.engine = engine ?? DemoWalkEngine()
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
    func startWalk() {
        if walkMode == .voice && !wavFixture {
            Task { [weak self] in
                guard let self else { return }
                if await AudioCaptureSource.requestPermissions() {
                    self.micDenied = false
                    self.beginWalk()
                } else {
                    self.micDenied = true
                }
            }
        } else {
            beginWalk()
        }
    }

    private func beginWalk() {
        pumpTask?.cancel()
        eventTask?.cancel()

        // begin() is throwing (review P1): the real engine's session start is
        // fallible across FFI. If it fails, the user must NOT enter the
        // walking flow — a dead session would run STT and silently drop every
        // append (capture loss). Stay on the board; walk state untouched.
        // sac: this deserves a visible error surface ("couldn't start the
        // walk — try again"); no error chrome exists in the app yet, so the
        // floor here is the log breadcrumb + not entering .walking. Yours to
        // design.
        let events: AsyncStream<WalkEvent>
        do {
            events = try engine.begin(trade: trade)
        } catch {
            Logger(subsystem: Bundle.main.bundleIdentifier ?? "sitewalk", category: "walk")
                .error("startWalk: engine.begin failed, staying on board: \(error, privacy: .public)")
            return
        }

        transcript = ""
        previewTail = ""
        items = []
        isPaused = false
        walkStart = Date()
        // Snapshot the session id NOW, while the engine's live session still
        // has one — see the doc comment on `currentSessionId` (Plan 11 D7).
        currentSessionId = engine.currentSessionId
        photos = []

        eventTask = Task { [weak self] in
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

        phase = .walking
        path = [.walking]
        if walkMode == .demo {
            startScriptedSource()
        } else {
            startAudioSource()
        }
    }

    /// Text/demo path: canned transcript → engine.append (unchanged).
    private func startScriptedSource() {
        let src = makeScriptedSource(trade)
        source = src
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
        let audio: any PCMAudioSource = wavFixture
            ? WavFileAudioSource(pushSamples: onSamples)   // mic-free fixture (D7)
            : AudioCaptureSource(pushSamples: onSamples, voiceProcessing: voiceProcessing) // live mic
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
        source?.stop()
        audioSource?.stop()
        // Navigate once, immediately, into the notes phase in a loading state.
        notes = nil
        notesLoading = true
        phase = .notes
        path = [.notes]
        Task {
            // Flush before finish (issue #155 / CANON: flush over speed —
            // the last words of a walk are often the price). `stop()` lets a
            // final speech result land (grace-bounded in SpeechSource); the
            // pump ends when the source's stream finishes, so awaiting it
            // guarantees every flushed chunk reached engine.append first.
            _ = await pumpTask?.value
            let notes = await engine.finish()
            self.notes = notes
            self.notesLoading = false
        }
    }

    /// Leaves the notes screen without building a document (e.g. the "not
    /// now" / back-to-board path, or the empty-walk UX). The session already
    /// reached `Processed` inside `finish()` — nothing to tombstone here,
    /// unlike `discardWalk()` (which cancels a still-live session). Just
    /// resets local UI state.
    func dismissNotes() {
        notes = nil
        documentBuildError = nil
        currentSessionId = nil
        phase = .board
        path = []
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
                self.documentBuildError = "Couldn’t build the \(DocKinds.label(for: kind).lowercased()) — tap to try again."
            }
        }
    }

    func buildPrimaryDocument() { buildDocument(kind: DocKinds.primaryKind(for: trade.key)) }

    /// Review → back to notes (the review screen's back arrow). Keeps the
    /// session and the built notes intact so the operator can build a different
    /// document or re-read; the last document stays in memory until the next
    /// build overwrites it. Pops just the review frame (board → notes).
    func backToNotes() {
        documentBuildError = nil
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

    // MARK: Review interactions

    func beginEdit(_ row: DocRowFixture) {
        editingRowID = row.id
        editText = row.amount.hasPrefix("$") ? String(row.amount.dropFirst()) : ""
    }

    func commitEdit() {
        guard let id = editingRowID, var doc = document,
              let index = doc.rows.firstIndex(where: { $0.id == id }) else {
            editingRowID = nil
            return
        }
        let cleaned = editText.replacingOccurrences(of: ",", with: "").trimmingCharacters(in: .whitespaces)
        if let value = Int(cleaned), value > 0 {
            let old = doc.rows[index]
            doc.rows[index] = DocRowFixture(
                title: old.title,
                sub: old.isGap ? "FILLED BY YOU" : old.sub,
                subWarn: false,
                hint: old.hint,
                qty: old.qty,
                amount: "$\(value)",
                isEdit: false,
                isGap: false
            )
            document = doc
        }
        editingRowID = nil
    }

    // MARK: Send

    func makePDF() {
        guard let doc = document else { return }
        shareURL = DocumentPDF.render(
            trade: trade, document: doc,
            biz: letterheadBiz, bizSub: letterheadSub, docDate: letterheadDate,
            branding: branding
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

    func completeSend() {
        if let index = jobs.firstIndex(where: { !$0.done }) {
            let old = jobs[index]
            jobs[index] = JobFixture(
                time: old.time, name: old.name, sub: old.sub,
                tag: TagFixture(kind: .green, label: "SENT"), done: true
            )
        }
        sessionWalks.append(WalkRecord(
            time: Self.clockNow(), docNo: trade.docNo, docKind: trade.docKind, sent: true
        ))
        shareURL = nil
        document = nil
        notes = nil
        phase = .board
        path = []
    }

    /// Abandon a reviewed document WITHOUT marking the job sent (issue #155:
    /// DISCARD previously routed through `completeSend()` and flipped the job
    /// to SENT). The persisted core artifact is untouched — only the app-side
    /// review state resets.
    func discardDocument() {
        sessionWalks.append(WalkRecord(
            time: Self.clockNow(), docNo: trade.docNo, docKind: trade.docKind, sent: false
        ))
        shareURL = nil
        document = nil
        notes = nil
        phase = .board
        path = []
    }
}
