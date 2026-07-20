import Foundation

// Stand-in engine so the UI is fully drivable before the FFI bridge exists.
// Keyword-matches the transcript against the trade's known items — enough to
// make the live board tick convincingly in demos. The real implementation
// replaces this class and nothing else (see docs/HANDOFF-ios-ffi.md).

@MainActor
final class DemoWalkEngine: WalkEngine {

    private var continuation: AsyncStream<WalkEvent>.Continuation?

    private var trade: TradeFixture = Fixtures.landscape
    private var seenText = ""
    private var firedItems: Set<Int> = []
    /// Cumulative matched items, in match order. `BoardItem.id` must be
    /// stable across snapshots (ForEach/lastCapturedID rely on it), so this
    /// keeps one fixture array and yields it — it never rebuilds fixtures
    /// per event (Task 10).
    private var board: [CapturedFixture] = []

    /// In-memory vocabulary so the editor is usable with no backend (Plan 10
    /// D8). Seeded with a couple of demo terms. Mirrors the Rust semantics
    /// loosely — normalize + case-insensitive dedup + a 100-term cap — enough
    /// to drive the editor; the real semantics live in `harness::Memory`.
    private var vocabulary: [String] = ["french drain", "ledger board"]
    private static let maxVocabularyTerms = 100

    // MARK: Photos (Plan 11) — in-memory demo conformance, no real files.
    // The demo has no backend, so a fixed session id stands in for a real
    // `session_id`; captures attach a bundled placeholder image name.
    private let demoSessionId = "demo-session"
    private var demoPhotos: [PhotoModel] = []
    var currentSessionId: String? { demoSessionId }

    /// Per-trade trigger phrases, index-aligned with `trade.captured`.
    private static let triggers: [String: [String]] = [
        "landscape": ["mulch", "boxwood", "zone two", "edge the beds", "twelve hundred"],
        "property": ["carpet", "blinds", "walls", "water heater", "balcony"],
        "inspection": ["shingles", "attic", "gfci", "furnace", "grading"]
    ]

    // MARK: Item CRUD (Plan 16) — in-memory demo conformance.
    // The Processed-only gate (D3-16) is NEW demo state: the demo had no
    // session-status concept before this seam, so it introduces the minimum
    // status enum needed to make the gate exercisable — begin() -> .recording,
    // finish() -> .processed. This is NOT the real-core status machine (no
    // AwaitingProcessing/Failed): just enough that an edit control wired
    // before DONE fails here the same way it fails on the real engine.
    enum DemoSessionStatus { case recording, processed }
    private var sessionStatus: [String: DemoSessionStatus] = [:]
    /// Demo-side kind bookkeeping (CapturedFixture carries a display tag,
    /// not a core kind) so update/add can validate and re-tag.
    private var itemKinds: [UUID: String] = [:]
    /// // sac: keep in sync with murmur-core VALID_ITEM_KINDS
    /// (crates/murmur-core/src/domain.rs) — the one shared allowlist.
    private static let validItemKinds = ["todo", "decision", "note", "safety", "part", "price"]

    enum DemoEditError: Error { case rejected(String) }

    func begin(trade: TradeFixture) -> AsyncStream<WalkEvent> {
        continuation?.finish()
        self.trade = trade
        seenText = ""
        firedItems = []
        board = []
        itemKinds = [:]
        sessionStatus[demoSessionId] = .recording
        var cont: AsyncStream<WalkEvent>.Continuation!
        let stream = AsyncStream<WalkEvent> { cont = $0 }
        continuation = cont
        return stream
    }

    func append(transcript: String) {
        seenText += transcript.lowercased()
        let phrases = Self.triggers[trade.key] ?? []
        var changed = false
        for (index, phrase) in phrases.enumerated() where !firedItems.contains(index) {
            if seenText.contains(phrase), index < trade.captured.count {
                firedItems.insert(index)
                board.append(trade.captured[index])
                changed = true
            }
        }
        if changed {
            continuation?.yield(.boardUpdated(board))
        }
    }

    // The scripted demo drives the board from TEXT (append), never audio.
    func pushAudio(_ samples: [Float]) {}

    // No Rust session to tear down in the demo path.
    func cancel() async {}

    // MARK: Vocabulary (Plan 10) — in-memory demo conformance.

    private func normalize(_ term: String) -> String {
        term.split(whereSeparator: \.isWhitespace).joined(separator: " ")
    }

    func listVocabulary() -> [String] { vocabulary }

    func addVocabularyTerm(_ term: String) -> [String] {
        let normalized = normalize(term)
        guard !normalized.isEmpty else { return vocabulary }
        // Case-insensitive dedup (keep first-seen casing); cap; reject silently
        // in the demo (the real engine throws — the editor surfaces that).
        if vocabulary.contains(where: { $0.caseInsensitiveCompare(normalized) == .orderedSame }) {
            return vocabulary
        }
        if vocabulary.count < Self.maxVocabularyTerms {
            vocabulary.append(normalized)
        }
        return vocabulary
    }

    func removeVocabularyTerm(_ term: String) -> [String] {
        let normalized = normalize(term)
        vocabulary.removeAll { $0.caseInsensitiveCompare(normalized) == .orderedSame }
        return vocabulary
    }

    /// Applied seed-pack markers, keyed "trade:version" — the demo mirror of
    /// the Rust `_seeds` memory section (Plan 15 D4-15).
    private var seededPacks: Set<String> = []
    /// Per-pass seed budget. // keep in sync with ffi SEED_MAX (60)
    private static let seedMax = 60

    /// In-memory mirror of the Rust `seed_vocabulary` semantics (Plan 15) —
    /// normalize + case-insensitive dedup + a 60-term per-pass budget + the
    /// 100-cap backstop + a "trade:version" idempotency marker — enough to
    /// drive the vocab card with no backend. The REAL semantics (and the
    /// WE-A..WE-E worked examples) live in crates/ffi/src/vocabulary.rs.
    func seedVocabulary(trade: String, version: UInt32, terms: [String]) -> SeedReport {
        let key = "\(trade):\(version)"
        guard !seededPacks.contains(key) else {
            return SeedReport(added: 0, duplicates: 0, skippedOverBudget: 0,
                              skippedFull: 0, alreadySeeded: true, terms: vocabulary)
        }
        var added: UInt32 = 0, duplicates: UInt32 = 0
        var skippedOverBudget: UInt32 = 0, skippedFull: UInt32 = 0
        for raw in terms {
            if added == UInt32(Self.seedMax) { skippedOverBudget += 1; continue }
            let normalized = normalize(raw)
            guard !normalized.isEmpty else { continue } // pre-gated by VocabPackTests
            if vocabulary.contains(where: { $0.caseInsensitiveCompare(normalized) == .orderedSame }) {
                duplicates += 1
                continue
            }
            if vocabulary.count >= Self.maxVocabularyTerms { skippedFull += 1; continue }
            vocabulary.append(normalized)
            added += 1
        }
        seededPacks.insert(key) // applied even when partial (D4-15)
        return SeedReport(added: added, duplicates: duplicates,
                          skippedOverBudget: skippedOverBudget, skippedFull: skippedFull,
                          alreadySeeded: false, terms: vocabulary)
    }

    // MARK: Photos (Plan 11) — in-memory demo conformance.

    func attachPhoto(sessionId: String, itemId: String?, filename: String, capturedAt: UInt64?) -> PhotoModel {
        let photo = PhotoModel(
            id: UUID().uuidString,
            sessionId: sessionId,
            itemId: itemId,
            filename: filename,
            capturedAt: capturedAt ?? UInt64(Date().timeIntervalSince1970)
        )
        demoPhotos.append(photo)
        return photo
    }

    func listPhotos(sessionId: String) -> [PhotoModel] {
        demoPhotos.filter { $0.sessionId == sessionId }
    }

    func removePhoto(photoId: String) {
        demoPhotos.removeAll { $0.id == photoId }
    }

    // No real files in the demo — nothing to sweep.
    func liveLivePhotoFilenames() -> [String] {
        demoPhotos.map(\.filename)
    }

    // No Recording rows to leak in the scripted demo (no crash-orphaned
    // sessions are possible here) — nothing to sweep.
    func sweepZombieSessions() -> UInt64 {
        0
    }

    // Nothing ever reaches Failed in the scripted demo — nothing to retry.
    func retryFailedSessions() -> UInt32 {
        0
    }

    // Plan 13: finish() = scripted NOTES (items + a canned summary line), no
    // document. `buildDocument(kind:)` below returns the canned DocumentModel
    // this used to return directly — the demo mirrors the real engine's
    // notes-first shape so DONE -> Notes -> button -> ReviewView is drivable
    // with zero backend.
    func finish() async -> NotesModel {
        continuation?.finish()
        continuation = nil
        // Plan 16: the demo session reaches its terminal state — edits are
        // allowed from here on (the D3-16 gate mirror).
        sessionStatus[demoSessionId] = .processed
        // Simulate the notes-compute beat (the real engine targets < 8 s).
        try? await Task.sleep(for: .seconds(0.8))
        let itemWord = board.count == 1 ? "item" : "items"
        let notes = NotesModel(
            summary: "Walked \(trade.site.capitalized(with: nil)) — \(board.count) \(itemWord) captured.",
            items: board,
            docKind: DocKinds.primaryKind(for: trade.key),
            queued: false,
            notes: Self.sampleNotes
        )
        lastNotes = notes
        return notes
    }

    // MARK: Walk-reopen read seam (Plan 20) — no-op history in the demo.

    /// The last finished walk's notes, kept so the reopen/fresh-read paths
    /// compile and behave in the demo (Plan 20 demo posture).
    private var lastNotes: NotesModel?

    /// The demo keeps its walk log in-memory (`AppModel.sessionWalks`) —
    /// `listSessions` stays a `[]` no-op stub. `hydrateWalkLog`'s F2 guard is
    /// what keeps this empty-success from clobbering the demo log.
    func listSessions() -> [WalkSummary] { [] }

    /// The last demo notes with the CURRENT board (so a post-edit fresh read
    /// reflects the edit rather than reverting it), or empty notes if no walk
    /// has finished. A harmless no-op path — never throws, never resurrects.
    func loadNotes(sessionId: String) -> NotesModel {
        guard var notes = lastNotes else {
            return NotesModel(summary: "", items: [], docKind: "report", queued: false)
        }
        notes.items = board
        return notes
    }

    // Plan 14 Task 6: scripted sample buckets (WE-A shape) so the demo build
    // exercises `NotesModel.notes` with zero backend. // sac: bucket
    // rendering is your follow-up — this data is inert until then.
    private static let sampleNotes: [NotesEntryFixture] = [
        NotesEntryFixture(
            bucket: .scopeOfWork,
            label: "Mulch — front beds",
            detail: "Darker mulch than last year; the old mulch faded."
        ),
        NotesEntryFixture(
            bucket: .constraints,
            label: "Budget",
            detail: "Keep the whole job under $1,200."
        ),
        NotesEntryFixture(
            bucket: .conditionsAndIssues,
            label: "Zone-2 irrigation head broken",
            detail: "Replace — parts + labor."
        )
    ]

    // MARK: Item CRUD (Plan 16) — mirrors the Rust semantics loosely:
    // partial-apply update, kind validated against the shared six-kind
    // allowlist, empty text rejected, Processed-only, add appends, remove
    // drops. Enough to exercise the edit UI with no backend; the REAL
    // semantics (and the WE-A..WE-D worked examples) live in
    // crates/ffi/src/items.rs.

    private func requireEditable(_ sessionId: String) throws {
        guard sessionId == demoSessionId, sessionStatus[sessionId] == .processed else {
            throw DemoEditError.rejected("cannot edit items on a non-processed session")
        }
    }

    /// Demo copy of MurmurEngineFormatting.tag(for:) — that one is behind
    /// the canImport(MurmurCoreFFI) gate, inert on the demo build.
    private static func tag(for kind: String) -> TagFixture {
        switch kind {
        case "safety": return TagFixture(kind: .red, label: "SAFETY")
        case "price": return TagFixture(kind: .green, label: "PRICE")
        case "part": return TagFixture(kind: .yellow, label: "PART")
        case "decision": return TagFixture(kind: .plain, label: "DECISION")
        default: return TagFixture(kind: .plain, label: "ITEM")
        }
    }

    func updateItem(
        sessionId: String, itemId: String, text: String?, kind: String?, right: String?
    ) throws -> CapturedFixture {
        try requireEditable(sessionId)
        guard let uuid = UUID(uuidString: itemId),
              let index = board.firstIndex(where: { $0.id == uuid }) else {
            throw DemoEditError.rejected("no such item: \(itemId)")
        }
        if let text, text.trimmingCharacters(in: .whitespaces).isEmpty {
            throw DemoEditError.rejected("item text is empty")
        }
        if let kind {
            guard Self.validItemKinds.contains(kind) else {
                throw DemoEditError.rejected("invalid kind '\(kind)'")
            }
            itemKinds[uuid] = kind
        }
        let old = board[index]
        let updated = CapturedFixture(
            id: uuid,
            tag: kind.map(Self.tag(for:)) ?? old.tag,
            text: text ?? old.text,
            right: right ?? old.right,
            photos: old.photos
        )
        board[index] = updated
        return updated
    }

    func addItem(
        sessionId: String, kind: String, text: String, right: String
    ) throws -> CapturedFixture {
        try requireEditable(sessionId)
        guard Self.validItemKinds.contains(kind) else {
            throw DemoEditError.rejected("invalid kind '\(kind)'")
        }
        guard !text.trimmingCharacters(in: .whitespaces).isEmpty else {
            throw DemoEditError.rejected("item text is empty")
        }
        let fixture = CapturedFixture(tag: Self.tag(for: kind), text: text, right: right)
        itemKinds[fixture.id] = kind
        board.append(fixture)   // append — mirrors the UUIDv7 ordering rule
        return fixture
    }

    func removeItem(sessionId: String, itemId: String) throws {
        try requireEditable(sessionId)
        guard let uuid = UUID(uuidString: itemId),
              let index = board.firstIndex(where: { $0.id == uuid }) else {
            throw DemoEditError.rejected("no such item: \(itemId)")
        }
        itemKinds.removeValue(forKey: uuid)
        board.remove(at: index)
    }

    // Plan 13 D1: the on-demand build, engine-keyed. The demo has no real
    // per-kind rendering — it returns the trade's canned document rows
    // regardless of `kind` (Worked Ex B shape), so the button -> ReviewView
    // wiring is exercised end to end without a backend.
    func buildDocument(sessionId: String, kind: String) async throws -> DocumentModel {
        try? await Task.sleep(for: .seconds(0.8))
        return DocumentModel(
            rows: trade.rows,
            totalKey: trade.totalKey,
            staticTotal: trade.totalValue,
            note: trade.note,
            send: trade.send
        )
    }
}
