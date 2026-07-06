import Foundation

// The real bridge adapter (Plan 07 Task 11): MurmurEngine: WalkEngine puts
// murmur-core behind sac's app via the `crates/ffi` UniFFI bridge. Formatting
// layer lives here (D2) — core emits display-copy-free structured data
// (cents, unix seconds, integer doc number, label keys); this file formats
// currency/date/prefix and owns letterhead/board-chrome lookups.
//
// WALL (Task 9, honestly reported — see docs/plans/2026-07-04-rust-core-07-ffi-bridge.md
// deviation notes / the FFI-bridge landing report): this project's Nix dev
// shell (flake.nix) provides only the HOST rustc/cargo — no rustup, no
// multi-target rust-overlay/fenix. `cargo build --target aarch64-apple-ios-sim`
// fails with E0463 ("can't find crate for `core`/`std`") because that
// target's std isn't installed and there's no way to add it from this shell.
// The `crates/ffi` crate itself was proven correct: built for the HOST
// target and run through `cargo run -p ffi --features uniffi-bindgen-cli
// --bin uniffi-bindgen -- generate --library target/release/libffi.dylib
// --language swift` successfully, producing the full expected Swift surface
// (MurmurEngine, WalkSession, EngineConfig, DocumentPayload, DocLine,
// BoardItem, WalkEvent, WalkEventListener). What's still missing is an
// iOS-linkable static lib/xcframework and the generated `MurmurCoreFFI`
// Swift package to link it — that needs the iOS cross-compilation toolchain
// (rustup + `rustup target add aarch64-apple-ios-sim x86_64-apple-ios-sim`,
// or a rust-overlay/fenix toolchain in flake.nix with those targets) added to
// this project's dev shell, which is out of this task's authorized scope
// (apps/ios only).
//
// This file is written to the real generated Swift API (field/method names
// verified against the host-built bindings above) so it activates the
// moment `import MurmurCoreFFI` resolves — no further edits should be
// needed once Task 9's toolchain gap is closed. Until then `canImport`
// keeps it inert so the app keeps building on DemoWalkEngine (D10).
#if canImport(MurmurCoreFFI)
import MurmurCoreFFI

// Not `private`: MurmurEngine's `init(config:)` takes FFIEngineConfig, so the
// typealias needs at least the same (internal) access as the initializer —
// a `private` alias here forces the initializer to be `fileprivate` too,
// which breaks construction from GalleryApp (Task 11 Step 3).
typealias FFIMurmurEngine = MurmurCoreFFI.MurmurEngine
typealias FFIWalkSession = MurmurCoreFFI.WalkSession
typealias FFIEngineConfig = MurmurCoreFFI.EngineConfig
private typealias FFIDocumentPayload = MurmurCoreFFI.DocumentPayload
private typealias FFIDocLine = MurmurCoreFFI.DocLine
private typealias FFIBoardItem = MurmurCoreFFI.BoardItem
private typealias FFIWalkEvent = MurmurCoreFFI.WalkEvent
private typealias FFIWalkEventListener = MurmurCoreFFI.WalkEventListener

/// Bridges a Rust callback (`WalkEventListener.onEvent`, invoked off-main)
/// into the app-facing `AsyncStream`. The closures yield DIRECTLY into the
/// stream's continuation from the callback thread (`Continuation.yield` is
/// thread-safe): yields from one thread are FIFO into the stream buffer, so
/// committed transcript chunks — all emitted by the single Rust pump thread —
/// render in order. (The previous per-event `Task { @MainActor }` hops were
/// NOT ordered: independent tasks can interleave, reordering chunks.) NOTE:
/// this guarantees per-thread order only, not a total order across different
/// Rust threads (board ticks arrive from the tokio pool) — which is exactly
/// what the transcript needs. Consumers still receive events on their own
/// actor: AppModel's event loop is a `@MainActor` task.
private final class BoardListener: FFIWalkEventListener {
    private let onBoardUpdated: @Sendable ([FFIBoardItem]) -> Void
    private let onTranscriptCommitted: @Sendable (String) -> Void
    private let onTranscriptPreview: @Sendable (String) -> Void
    init(
        onBoardUpdated: @escaping @Sendable ([FFIBoardItem]) -> Void,
        onTranscriptCommitted: @escaping @Sendable (String) -> Void,
        onTranscriptPreview: @escaping @Sendable (String) -> Void
    ) {
        self.onBoardUpdated = onBoardUpdated
        self.onTranscriptCommitted = onTranscriptCommitted
        self.onTranscriptPreview = onTranscriptPreview
    }
    func onEvent(event: FFIWalkEvent) {
        switch event {
        case .boardUpdated(let items):
            onBoardUpdated(items)
        case .transcriptCommitted(let text):
            onTranscriptCommitted(text)
        case .transcriptPreview(let text):
            onTranscriptPreview(text)
        }
    }
}

@MainActor
final class MurmurEngine: WalkEngine {
    private let engine: FFIMurmurEngine
    private var session: FFIWalkSession?
    private var continuation: AsyncStream<WalkEvent>.Continuation?
    /// The document built by the most recent `finish()` call. `session` is
    /// nil'd out once `finish()` has run (below) — a re-entrant `finish()`
    /// call (e.g. a double-tap racing the UI transition) has no session left
    /// to call into, so it returns this instead of a blank document. Rust's
    /// `WalkSession.finish()` is itself safe to call twice (it degrades
    /// rather than panicking — see crates/ffi/src/session.rs), but nothing
    /// on the Swift side should ever issue that second call in the first
    /// place once we already have the answer.
    private var lastDocument: DocumentModel?

    // Throwing: the Rust constructor is fallible across FFI now (opening the
    // store / starting the runtime can fail) — no panics across the boundary.
    // GalleryApp falls back to DemoWalkEngine when this throws (D10).
    init(config: FFIEngineConfig) throws {
        self.engine = try FFIMurmurEngine(config: config)
    }

    // BoardItem.id is a Rust-side string uuid; parsed once here and threaded
    // through to CapturedFixture.id (Fixtures.swift) so ids stay stable
    // across boardUpdated snapshots (Plan 07 Task 10/Self-Review).
    func begin(trade: TradeFixture) throws -> AsyncStream<WalkEvent> {
        // A second begin() cancels the first stream cleanly (Self-Review:
        // per-session stream lifetime) — finish the old continuation before
        // handing out a fresh one.
        continuation?.finish()
        continuation = nil
        lastDocument = nil
        // Tear down any surviving prior session DETERMINISTICALLY before the
        // fallible start (review finding 1c): just nil-ing it would strand the
        // Rust pump thread + whisper Metal context (and the Recording rows)
        // until the best-effort Drop safety net fires. `cancel()` stops the
        // pump and tombstones; it is idempotent, so this is safe even if the
        // session already finished. Fire-and-forget is acceptable — the new
        // session is independent of the old one's teardown. Nil-ing `session`
        // first also keeps the original invariant: if beginWalk throws below,
        // append/finish no-op on nil rather than touching the stale session.
        if let stale = session {
            session = nil
            Task { await stale.cancel() }
        }

        // beginWalk is fallible across FFI (store lock / session insert). The
        // failure PROPAGATES (review P1): returning a normal-looking stream
        // with no session would let the app enter the walking flow and
        // silently drop every append — capture loss, the worst failure for
        // this product. AppModel catches and stays on the board.
        let newSession = try engine.beginWalk(jobId: nil, template: trade.key) // template key = trade.key (D4)

        let (stream, cont) = AsyncStream<WalkEvent>.makeStream()
        continuation = cont
        // Yield DIRECTLY from the Rust callback thread (review finding 3):
        // per-event `Task { @MainActor }` hops are unordered and can render
        // committed transcript chunks out of order. `cont` is captured by
        // value — yielding into a finished continuation is a documented no-op,
        // which covers the post-finish()/post-begin() window without touching
        // `self.continuation` off the main actor. The mapping helpers are
        // `nonisolated` pure functions, safe off-main.
        newSession.setEventListener(listener: BoardListener(
            onBoardUpdated: { items in
                cont.yield(.boardUpdated(items.map(Self.board)))
            },
            onTranscriptCommitted: { text in
                cont.yield(.transcriptCommitted(text))
            },
            onTranscriptPreview: { text in
                cont.yield(.transcriptPreview(text))
            }
        ))
        session = newSession
        return stream
    }

    func append(transcript: String) {
        session?.appendTranscript(text: transcript)
    }

    // The audio path (Plan 08 D1/D2): hand mic PCM to the Rust pump. Cheap
    // enqueue; the transcript comes back via transcriptCommitted events.
    func pushAudio(_ samples: [Float]) {
        session?.pushAudio(samples: samples)
    }

    // DISCARD (Plan 08 Task 4): stop the pump + tombstone the session in Rust,
    // then drop our side. Async — the Rust cancel() joins the pump off the
    // async workers; AppModel calls this from a detached Task so the main actor
    // never blocks. Idempotent on the Rust side (safe after finish()).
    func cancel() async {
        continuation?.finish()
        continuation = nil
        await session?.cancel()
        session = nil
        lastDocument = nil
    }

    func finish() async -> DocumentModel {
        // Re-entrant call: `session` was already nil'd out by a prior
        // `finish()` (or none ever began). Harmless no-op — hand back
        // whatever we already built instead of calling into a session that
        // no longer exists on this side.
        guard let session else { return lastDocument ?? Self.emptyDocument() }

        continuation?.finish()
        continuation = nil
        let payload = await session.finish()
        // Drop the session now that it's finished — this is what makes the
        // guard above fire on any subsequent call, instead of issuing a
        // second `finish()` down through the FFI.
        self.session = nil
        let document = Self.document(payload)
        lastDocument = document
        return document
    }

    // MARK: - Vocabulary (Plan 10): forward to the FFI CRUD methods. Each is
    // throwing across the boundary (vocabulary full, empty term, poisoned lock,
    // persistence failure); the thrown error propagates to AppModel, which logs
    // a breadcrumb and leaves the list unchanged (never crashes). add/remove
    // return the resulting list so the editor updates in one round-trip.

    func listVocabulary() throws -> [String] {
        try engine.listVocabulary()
    }

    func addVocabularyTerm(_ term: String) throws -> [String] {
        try engine.addVocabularyTerm(term: term)
    }

    func removeVocabularyTerm(_ term: String) throws -> [String] {
        try engine.removeVocabularyTerm(term: term)
    }

    // MARK: - Photos (Plan 11): engine-keyed CRUD (not WalkSession-scoped —
    // photos are attachable during the walk AND at review time, when there is
    // no live WalkSession). `session` gives the active walk's id via the
    // `sessionId()` getter; `nil` when there is no live session.

    var currentSessionId: String? {
        session?.sessionId()
    }

    func attachPhoto(sessionId: String, itemId: String?, filename: String, capturedAt: UInt64?) throws -> PhotoModel {
        let ref = try engine.addPhoto(sessionId: sessionId, itemId: itemId, filename: filename, capturedAt: capturedAt)
        return Self.photo(ref)
    }

    func listPhotos(sessionId: String) throws -> [PhotoModel] {
        try engine.listPhotos(sessionId: sessionId).map(Self.photo)
    }

    func removePhoto(photoId: String) throws {
        try engine.removePhoto(photoId: photoId)
    }

    func liveLivePhotoFilenames() throws -> [String] {
        try engine.listLivePhotoFilenames()
    }

    private nonisolated static func photo(_ ref: MurmurCoreFFI.PhotoRef) -> PhotoModel {
        PhotoModel(
            id: ref.id,
            sessionId: ref.sessionId,
            itemId: ref.itemId,
            filename: ref.filename,
            capturedAt: ref.capturedAt
        )
    }

    // MARK: - Formatting layer (D2): core is display-copy-free; this is
    // where cents → "$285", doc_number → "EST-0047", job_date_unix →
    // "JUL 01 2026", and label keys → display copy happen.

    // nonisolated: pure value mapping, called from the Rust callback thread
    // (the direct-yield path above) — must not be @MainActor-isolated.
    private nonisolated static func board(_ item: FFIBoardItem) -> CapturedFixture {
        CapturedFixture(
            id: UUID(uuidString: item.id) ?? UUID(),
            tag: tag(for: item.kind),
            text: item.text,
            right: item.right,
            photos: Int(item.photoCount)
        )
    }

    private nonisolated static func tag(for kind: String) -> TagFixture {
        switch kind {
        case "safety": return TagFixture(kind: .red, label: "SAFETY")
        case "price": return TagFixture(kind: .green, label: "PRICE")
        case "part": return TagFixture(kind: .yellow, label: "PART")
        case "decision": return TagFixture(kind: .plain, label: "DECISION")
        default: return TagFixture(kind: .plain, label: "ITEM")
        }
    }

    private static let centsFormatter: NumberFormatter = {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        return formatter
    }()

    private static func amountString(_ cents: Int64?) -> String {
        guard let cents else { return "——" }
        let dollars = Double(cents) / 100.0
        return "$" + (centsFormatter.string(from: NSNumber(value: dollars)) ?? "\(dollars)")
    }

    private static let dateFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "MMM dd yyyy"
        formatter.locale = Locale(identifier: "en_US_POSIX")
        return formatter
    }()

    private static func dateLabel(_ unixSeconds: UInt64) -> String {
        dateFormatter.string(from: Date(timeIntervalSince1970: TimeInterval(unixSeconds))).uppercased()
    }

    private static func docNumberLabel(docKind: String, docNumber: UInt64) -> String {
        let prefix: String
        switch docKind {
        case "estimate": prefix = "EST"
        case "inspection": prefix = "IR"
        default: prefix = "MO"
        }
        return "\(prefix)-\(String(format: "%04d", docNumber))"
    }

    /// Per-`doc_kind` display copy the milestone doesn't yet source from
    /// core — letterhead/board chrome stays in `TradeFixture` (D2); this
    /// table is the document-body chrome only (total label, footer note,
    /// send button copy).
    private static func totalLabel(_ key: String) -> String {
        switch key {
        case "deposit_deduction": return "DEPOSIT DEDUCTION"
        case "findings": return "FINDINGS"
        default: return "TOTAL"
        }
    }

    private static func note(for docKind: String, queued: Bool) -> String {
        if queued {
            return "SAVED OFFLINE — WILL FINISH WHEN YOU RECONNECT"
        }
        switch docKind {
        case "inspection": return "FINDINGS MARKED — NOT YET ASSESSED"
        case "report": return "DEDUCTIONS LEFT OPEN ARE MARKED — CONFIRM BEFORE SENDING"
        default: return "GAPS ARE MARKED — TAP TO FILL BEFORE SENDING"
        }
    }

    private static func sendLabel(for docKind: String) -> String {
        switch docKind {
        case "inspection": return "SEND REPORT"
        case "report": return "SEND REPORT"
        default: return "SEND ESTIMATE"
        }
    }

    private static func row(_ line: FFIDocLine) -> DocRowFixture {
        DocRowFixture(
            title: line.title,
            sub: line.isGap ? "NOT HEARD — TAP OR SAY IT" : line.detail,
            subWarn: line.isGap,
            hint: nil, // Deferred 4: price-book autofill hint
            qty: line.qty,
            amount: amountString(line.amountCents),
            isEdit: false, // Deferred 4: pre-filled-from-history affordance
            isGap: line.isGap
        )
    }

    private static func document(_ payload: FFIDocumentPayload) -> DocumentModel {
        DocumentModel(
            rows: payload.lines.map(row),
            totalKey: totalLabel(payload.totalLabelKey),
            staticTotal: payload.staticTotalCents.map(amountString) ?? "——",
            note: note(for: payload.docKind, queued: payload.queued),
            send: sendLabel(for: payload.docKind)
        )
    }

    private static func emptyDocument() -> DocumentModel {
        DocumentModel(rows: [], totalKey: "TOTAL", staticTotal: "——", note: "", send: "SEND")
    }
}
#endif
