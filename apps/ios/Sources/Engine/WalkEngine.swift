import Foundation

// The seam between the UI and the extraction engine.
//
// This protocol deliberately mirrors murmur-core's session API so the FFI
// bridge is a thin adapter, not a redesign:
//
//   begin(trade:)      → store.start_session(job_id)
//   append(transcript:)→ store.append_transcript + LiveExtractor incremental pass
//   events             → items the live pass lands on the board
//   finish()           → end_and_record_session + SessionProcessor.process → artifact
//
// The UI owns speech-to-text (see TranscriptSource) and only ever sends text
// down. The engine owns extraction and never receives audio.

enum WalkEvent {
    /// A structured item landed on the live board.
    case itemCaptured(CapturedFixture)
}

struct DocumentModel {
    var rows: [DocRowFixture]
    var totalKey: String
    var staticTotal: String   // used when rows carry no $ amounts (e.g. inspection)
    var note: String
    var send: String

    var gapCount: Int { rows.filter(\.isGap).count }

    /// Sum of $-parseable amounts; falls back to the template total.
    var totalValue: String {
        let sum = rows.compactMap { row -> Int? in
            guard row.amount.hasPrefix("$") else { return nil }
            return Int(row.amount.dropFirst().replacingOccurrences(of: ",", with: ""))
        }.reduce(0, +)
        guard sum > 0 else { return staticTotal }
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        return "$" + (formatter.string(from: NSNumber(value: sum)) ?? "\(sum)")
    }
}

@MainActor
protocol WalkEngine: AnyObject {
    /// Item events from live extraction, delivered on the main actor.
    var events: AsyncStream<WalkEvent> { get }

    /// Start a session for a trade (template key).
    func begin(trade: TradeFixture)

    /// Feed newly transcribed text. Called repeatedly during the walk.
    func append(transcript: String)

    /// End the session and build the document. Target: < 8 s, no spinner lies.
    func finish() async -> DocumentModel
}
