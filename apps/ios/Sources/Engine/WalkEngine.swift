import Foundation

// The seam between the UI and the extraction engine.
//
// This protocol deliberately mirrors murmur-core's session API so the FFI
// bridge is a thin adapter, not a redesign:
//
//   begin(trade:)      → store.start_session(job_id)
//   append(transcript:)→ store.append_transcript + LiveExtractor incremental pass
//   events             → items the live pass lands on the board
//   finish()           → end_and_record_session + SessionProcessor.process → NOTES
//   buildDocument()    → MurmurEngine::build_document(kind), on demand (Plan 13)
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

/// App-facing mirror of the Rust `PhotoRef` (Plan 11 D7) — a display-copy-free
/// projection. `filename` is a relative name under `<Documents>/photos/`;
/// resolving it to a real file URL is the capture/gallery view's job.
struct PhotoModel: Identifiable, Equatable {
    var id: String
    var sessionId: String
    var itemId: String?
    var filename: String
    var capturedAt: UInt64
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

/// Plan 14 D2-14: the three notes buckets. Mirrors the uniffi `NotesBucket`
/// enum — an exhaustive `switch` over this is safe because the FFI
/// conversion boundary (`crates/ffi/src/convert.rs::notes_entries`) already
/// drops any row whose bucket string isn't one of the three known variants
/// (R6: never fabricate/coerce a bucket).
enum NotesBucket {
    case scopeOfWork
    case constraints
    case conditionsAndIssues
}

/// Plan 14 D2-14: one comprehensive-notes coordination entry — the detail
/// behind a terse board item (client preferences, budget, site conditions,
/// deadlines). `label` is terse (mirrors a board label); `detail` is the
/// full spoken context. // sac: the grouped bucket sections + visuals are
/// your follow-up (NotesView.swift's kind-grouped rendering already covers
/// the terse board — bucket rendering is additive on top).
struct NotesEntryFixture: Identifiable {
    var id: String { label + detail }
    var bucket: NotesBucket
    var label: String
    var detail: String
}

/// Plan 13 D2/D3: `finish()`'s notes-first result — items + summary, NOT a
/// document. The document build moves to an explicit, later
/// `buildDocument(sessionId:kind:)` call from the notes screen's action row.
/// `docKind` is ADVISORY only (core's template default) — button wiring keys
/// off the client-known `TradeFixture.key`/template, never off this field
/// (Plan 13 D2). // sac: grouping items by `tag`/kind for the notes screen is
/// yours; this is the plumbing shape only.
struct NotesModel {
    var summary: String
    var items: [CapturedFixture]
    var docKind: String
    /// `true` when `finish()` degraded offline (D9) — no authoritative board
    /// exists yet, so build-document actions must stay disabled until a
    /// retry succeeds.
    var queued: Bool
    /// Plan 14: the comprehensive, bucketed coordination entries captured at
    /// summary time. Defaults to `[]` — additive, so every existing caller
    /// (including test fixtures) keeps compiling unchanged. // sac: unrendered
    /// until the bucket-section follow-up lands; carrying data is this task's
    /// only job.
    var notes: [NotesEntryFixture] = []
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

    /// End the session and return its NOTES (Plan 13 D1/D2) — items +
    /// summary, computed by the pipeline's existing extraction+summary pass.
    /// No document is built here anymore; that's the deliberate, on-demand
    /// `buildDocument(sessionId:kind:)` call below. Target: < 8 s, no spinner
    /// lies.
    func finish() async -> NotesModel

    /// Build the finished document for `kind` on demand (Plan 13 D1, Stage
    /// 1's `MurmurEngine::build_document`) — called from the notes screen's
    /// action row, NOT during `finish()`. Engine-keyed (not session-scoped):
    /// `finish()` already dropped its live session handle, so this call
    /// works from history/relaunch too. Burns a fresh document number per
    /// tap (D7: regenerate is explicit, never silently reused). Throwing:
    /// the real FFI call is fallible (a non-`Processed` session, an illegal
    /// `kind` for the template) — the caller (the notes screen) surfaces the
    /// error and leaves the button available to retry, never crashes.
    func buildDocument(sessionId: String, kind: String) async throws -> DocumentModel

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

    // Photo attachments (Plan 11). Bytes are the SHELL's responsibility: write
    // the file into <Documents>/photos/ FIRST, then call attachPhoto(...) with
    // its relative filename. Deletion is the reconciling sweep — see
    // sweepPhotoBytes() on AppModel. Throwing: the FFI methods are fallible
    // (missing session, bad item_id, persistence).
    //
    // `attachPhoto` is `async` (PR #176 should-fix): the real implementation's
    // FFI call takes the same `std::Mutex` store lock the Rust pump thread
    // contends for during live-extraction commits, so it can block for a
    // while. `async` lets `MurmurEngine` hop the actual call off the main
    // actor (`Task.detached`) while the app-facing call site stays a normal
    // `await` that never blocks the UI thread. `DemoWalkEngine` needs no
    // change — a synchronous function already satisfies an `async` protocol
    // requirement.
    func attachPhoto(
        sessionId: String, itemId: String?, filename: String, capturedAt: UInt64?
    ) async throws -> PhotoModel
    func listPhotos(sessionId: String) throws -> [PhotoModel]
    func removePhoto(photoId: String) throws
    func liveLivePhotoFilenames() throws -> [String]   // for the sweep

    /// App-open zombie sweep: a `Recording` session left behind by a crash or
    /// force-quit mid-walk can never resume (there is no live session for it
    /// after relaunch) — this transitions every such row to `Failed` so it
    /// stops sitting in `Recording` forever. Called once at app open,
    /// alongside `sweepPhotoBytes()` (see `AppRoot.body`'s `.task`).
    /// Transcript/items survive; `Failed -> Processed` remains the existing
    /// retry path. Returns the number swept (`0` on a clean relaunch).
    /// `DemoWalkEngine` no-ops (there is no Recording concept to leak in the
    /// scripted demo).
    func sweepZombieSessions() throws -> UInt64

    /// Retries every `Failed` session (offline drop, LLM error, or a
    /// zombie-swept crash-orphan — indistinguishable once they're `Failed`)
    /// once each, oldest first. This is what makes the offline banner
    /// ("SAVED OFFLINE — DOCUMENTS UNLOCK WHEN YOU RECONNECT") true: without
    /// it, nothing ever revisited a `Failed` session. `async` because it
    /// makes real LLM calls — callers must NOT await it inline on the
    /// app-open path (see `AppModel+Photos.runAppOpenSweeps`, which fires
    /// this from a separate detached `Task` AFTER the synchronous sweeps).
    /// Returns the count that reached `Processed`; a still-Failed session
    /// (still offline) is not an error, just uncounted. `DemoWalkEngine`
    /// no-ops (nothing is ever `Failed` in the scripted demo).
    func retryFailedSessions() async throws -> UInt32

    /// The active walk's session id, so the capture UI can call
    /// `attachPhoto(sessionId:...)` mid-walk (Plan 11 D7). `nil` when there is
    /// no live session (not walking, or the real engine has none yet).
    var currentSessionId: String? { get }
}
