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
    /// A whole-board snapshot, delivered once per live pass (batched by
    /// construction — Plan 07 D3). The live→authoritative swap at finish is
    /// just the terminal snapshot this carries; SwiftUI's `ForEach(id:)`
    /// computes the visual diff from the assigned array.
    case boardUpdated([CapturedFixture])
    /// Newly FINALIZED transcript text from the Rust STT pump (Plan 08 D4).
    /// The UI appends it to the visible transcript. For the audio path the
    /// transcript now ORIGINATES in Rust (whisper), not from `src.chunks`.
    case transcriptCommitted(String)
    /// The volatile, un-finalized preview tail (Plan 08 D4). Rendered greyed;
    /// never persisted, never extracted. Nice-to-have for a live feel.
    case transcriptPreview(String)
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
    /// Start a session for a trade and return THAT SESSION's event stream.
    /// Streams are per-session: consumers cancel freely at session end, and
    /// the next begin() hands out a fresh stream. Events arrive on main.
    ///
    /// Throwing: the real engine's session start is fallible (store insert
    /// across FFI). A dead session must surface HERE — if begin were
    /// non-throwing, the app would enter the walking flow, STT would run, and
    /// every append would silently drop: an hour of speech lost is the worst
    /// possible failure for this product. DemoWalkEngine conforms without
    /// throwing (a non-throwing implementation satisfies a throws requirement).
    func begin(trade: TradeFixture) throws -> AsyncStream<WalkEvent>

    /// Feed newly transcribed text. Called repeatedly during the walk
    /// (the scripted/text path).
    func append(transcript: String)

    /// Feed mic PCM (16 kHz mono f32) for the Rust STT path — the parallel to
    /// `append(transcript:)` for the audio path (Plan 08 D1/D2). A cheap
    /// enqueue; the transcript arrives back via `WalkEvent.transcriptCommitted`.
    /// `DemoWalkEngine` no-ops it (the scripted demo needs no audio).
    func pushAudio(_ samples: [Float])

    /// End the session and build the document. Target: < 8 s, no spinner lies.
    func finish() async -> DocumentModel

    /// DISCARD the session (Plan 08 Task 4): stop the STT pump and tombstone
    /// the session in Rust. Async because the Rust `cancel()` `spawn_blocking`-
    /// joins the pump (a decode can be in flight) — callers run it from a
    /// detached `Task` so the main actor never blocks. `DemoWalkEngine` no-ops.
    func cancel() async

    // Vocabulary → STT biasing loop, write half (Plan 10). These mutate the
    // `Memory` "vocabulary" section the engine reads at `begin_walk` to bias
    // whisper. Throwing: the real FFI methods are fallible across the boundary
    // (vocabulary full, empty term, a poisoned lock, a persistence failure) —
    // the editor surfaces the error and leaves its list unchanged rather than
    // crashing. add/remove return the RESULTING list so the editor updates in
    // one round-trip. `DemoWalkEngine` conforms with an in-memory list.
    func listVocabulary() throws -> [String]
    func addVocabularyTerm(_ term: String) throws -> [String]
    func removeVocabularyTerm(_ term: String) throws -> [String]
}
