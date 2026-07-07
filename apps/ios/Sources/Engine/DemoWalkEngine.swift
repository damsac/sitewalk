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

    func begin(trade: TradeFixture) -> AsyncStream<WalkEvent> {
        continuation?.finish()
        self.trade = trade
        seenText = ""
        firedItems = []
        board = []
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

    func finish() async -> DocumentModel {
        continuation?.finish()
        continuation = nil
        // Simulate the document build beat (the real engine targets < 8 s).
        try? await Task.sleep(for: .seconds(1.6))
        return DocumentModel(
            rows: trade.rows,
            totalKey: trade.totalKey,
            staticTotal: trade.totalValue,
            note: trade.note,
            send: trade.send
        )
    }
}
