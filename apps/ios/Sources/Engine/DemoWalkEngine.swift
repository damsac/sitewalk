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

    /// Per-trade trigger phrases, index-aligned with `trade.captured`.
    private static let triggers: [String: [String]] = [
        "landscape": ["mulch", "boxwood", "zone two", "edge the beds", "twelve hundred"],
        "property": ["carpet", "blinds", "walls", "water heater", "balcony"],
        "inspection": ["shingles", "attic", "gfci", "furnace", "grading"],
    ]

    func begin(trade: TradeFixture) -> AsyncStream<WalkEvent> {
        continuation?.finish()
        self.trade = trade
        seenText = ""
        firedItems = []
        var cont: AsyncStream<WalkEvent>.Continuation!
        let stream = AsyncStream<WalkEvent> { cont = $0 }
        continuation = cont
        return stream
    }

    func append(transcript: String) {
        seenText += transcript.lowercased()
        let phrases = Self.triggers[trade.key] ?? []
        for (index, phrase) in phrases.enumerated() where !firedItems.contains(index) {
            if seenText.contains(phrase), index < trade.captured.count {
                firedItems.insert(index)
                continuation?.yield(.itemCaptured(trade.captured[index]))
            }
        }
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
