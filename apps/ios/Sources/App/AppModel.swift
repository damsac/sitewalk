import SwiftUI
import Observation
import os

// One observable model drives the whole flow:
//   board → walking (pause/resume, photos) → building → review (edit, fill gaps) → sent
// The engine behind it is injected; today that's DemoWalkEngine, tomorrow the
// FFI bridge. The UI never knows the difference.

@MainActor
@Observable
final class AppModel {

    enum Phase: Equatable {
        case board
        case walking
        case building
        case review
    }

    // MARK: State

    var trade: TradeFixture = Fixtures.landscape
    var jobs: [JobFixture] = Fixtures.landscape.jobs
    var phase: Phase = .board
    var path: [Phase] = []

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
    /// Snapshotted when a walk successfully begins and kept through review
    /// (Plan 11 D7): the real `MurmurEngine` drops its live `WalkSession` once
    /// `finish()` returns, so `engine.currentSessionId` alone would go nil
    /// exactly when review-time photo capture needs it. Engine-keyed CRUD
    /// (add/list/remove_photo) works on a `Processed` session too — there is
    /// no live `WalkSession` requirement, just this id.
    private(set) var currentSessionId: String?

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
    private let scripted: Bool
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

    init(engine: WalkEngine? = nil, scripted: Bool = true, wavFixture: Bool = false,
         voiceProcessing: Bool = false) {
        self.engine = engine ?? DemoWalkEngine()
        self.scripted = scripted
        self.wavFixture = wavFixture
        self.voiceProcessing = voiceProcessing
    }

    // MARK: Trade switching (validation strategy: same bones, swappable template)

    func switchTrade(_ newTrade: TradeFixture) {
        trade = newTrade
        jobs = newTrade.jobs
    }

    // MARK: Walk lifecycle

    func startWalk() {
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
        if scripted {
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
    func addPhoto(_ data: Data) {
        capturePhoto(image: data, itemId: lastCapturedID?.uuidString.lowercased())
        guard photoError == nil,
              let id = lastCapturedID,
              let idx = items.firstIndex(where: { $0.id == id }) else { return }
        items[idx].photos += 1
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
        phase = .board
        path = []
    }

    func finishWalk() {
        source?.stop()
        audioSource?.stop()
        phase = .building
        path = [.building]
        Task {
            // Flush before finish (issue #155 / CANON: flush over speed —
            // the last words of a walk are often the price). `stop()` lets a
            // final speech result land (grace-bounded in SpeechSource); the
            // pump ends when the source's stream finishes, so awaiting it
            // guarantees every flushed chunk reached engine.append first.
            _ = await pumpTask?.value
            let doc = await engine.finish()
            self.document = doc
            self.phase = .review
            self.path = [.review]
        }
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
        shareURL = DocumentPDF.render(trade: trade, document: doc)
    }

    func completeSend() {
        if let index = jobs.firstIndex(where: { !$0.done }) {
            let old = jobs[index]
            jobs[index] = JobFixture(
                time: old.time, name: old.name, sub: old.sub,
                tag: TagFixture(kind: .green, label: "SENT"), done: true
            )
        }
        shareURL = nil
        document = nil
        phase = .board
        path = []
    }

    /// Abandon a reviewed document WITHOUT marking the job sent (issue #155:
    /// DISCARD previously routed through `completeSend()` and flipped the job
    /// to SENT). The persisted core artifact is untouched — only the app-side
    /// review state resets.
    func discardDocument() {
        shareURL = nil
        document = nil
        phase = .board
        path = []
    }
}
