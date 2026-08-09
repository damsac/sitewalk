import Foundation

// The formatting/mapping layer for `MurmurEngine` (D2), split out of
// MurmurEngine.swift to keep that file under the file-length lint. Core is
// display-copy-free (cents, unix seconds, integer doc number, label keys);
// this file formats currency/date/prefix and owns board-chrome/document-body
// lookups. Same `canImport(MurmurCoreFFI)` gate as MurmurEngine.swift — inert
// on the demo build.
#if canImport(MurmurCoreFFI)
import MurmurCoreFFI

extension MurmurEngine {
    // MARK: - Formatting layer (D2): core is display-copy-free; this is
    // where cents → "$285", doc_number → "EST-0047", job_date_unix →
    // "JUL 01 2026", and label keys → display copy happen.

    // nonisolated: pure value mapping, called from the Rust callback thread
    // (the direct-yield path in MurmurEngine.swift) — must not be
    // @MainActor-isolated.
    nonisolated static func board(_ item: FFIBoardItem) -> CapturedFixture {
        CapturedFixture(
            id: UUID(uuidString: item.id) ?? UUID(),
            tag: tag(for: item.kind),
            text: item.text,
            right: item.right,
            photos: Int(item.photoCount)
        )
    }

    nonisolated static func tag(for kind: String) -> TagFixture {
        switch kind {
        case "safety": return TagFixture(kind: .red, label: "SAFETY")
        case "price": return TagFixture(kind: .green, label: "PRICE")
        case "part": return TagFixture(kind: .yellow, label: "PART")
        case "decision": return TagFixture(kind: .plain, label: "DECISION")
        default: return TagFixture(kind: .plain, label: "ITEM")
        }
    }

    /// Cents → "$285" / "$125.50". Shared with the total (`DocumentModel.money`)
    /// so a line and the sum underneath it can never disagree about how money
    /// is written — and so a price with cents renders both digits instead of
    /// the "$125.5" a bare decimal formatter produces.
    static func amountString(_ cents: Int64?) -> String {
        guard let cents else { return "——" }
        return DocumentModel.money(cents: Int(cents))
    }

    private static let dateFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "MMM dd yyyy"
        formatter.locale = Locale(identifier: "en_US_POSIX")
        return formatter
    }()

    static func dateLabel(_ unixSeconds: UInt64) -> String {
        dateFormatter.string(from: Date(timeIntervalSince1970: TimeInterval(unixSeconds))).uppercased()
    }

    static func docNumberLabel(docKind: String, docNumber: UInt64) -> String {
        let prefix: String
        switch docKind {
        case "estimate": prefix = "EST"
        case "invoice": prefix = "INV"
        case "work_order": prefix = "WO"
        case "inspection": prefix = "IR"
        case "condition": prefix = "COND"
        case "move_out": prefix = "MO"
        default: prefix = "DOC"
        }
        return "\(prefix)-\(String(format: "%04d", docNumber))"
    }

    /// Per-`doc_kind` display copy the milestone doesn't yet source from
    /// core — letterhead/board chrome stays in `TradeFixture` (D2); this
    /// table is the document-body chrome only (total label, footer note,
    /// send button copy). // sac: the notes-screen button labels/taxonomy
    /// are yours (D8 open question) — this table only keeps the document
    /// body's OWN chrome (reached via the button, in the unchanged
    /// ReviewView) correct for every Plan 13 kind, so no template silently
    /// falls through to the old "estimate" default copy.
    static func totalLabel(_ key: String) -> String {
        switch key {
        case "deposit_deduction": return "DEPOSIT DEDUCTION"
        case "findings": return "FINDINGS"
        default: return "TOTAL"
        }
    }

    static func note(for docKind: String, queued: Bool) -> String {
        if queued {
            return "SAVED OFFLINE — WILL FINISH WHEN YOU RECONNECT"
        }
        switch docKind {
        case "inspection": return "FINDINGS MARKED — NOT YET ASSESSED"
        case "report", "condition", "move_out": return "DEDUCTIONS LEFT OPEN ARE MARKED — CONFIRM BEFORE SENDING"
        case "work_order": return "GAPS ARE MARKED — CONFIRM SCOPE BEFORE SENDING"
        default: return "GAPS ARE MARKED — TAP TO FILL BEFORE SENDING"
        }
    }

    static func sendLabel(for docKind: String) -> String {
        switch docKind {
        case "inspection": return "SEND REPORT"
        case "report", "condition", "move_out": return "SEND REPORT"
        case "invoice": return "SEND INVOICE"
        case "work_order": return "SEND WORK ORDER"
        default: return "SEND ESTIMATE"
        }
    }

    static func row(_ line: FFIDocLine) -> DocRowFixture {
        DocRowFixture(
            title: line.title,
            sub: line.isGap ? "NOT HEARD — TAP OR SAY IT" : line.detail,
            subWarn: line.isGap,
            hint: nil, // Deferred 4: price-book autofill hint
            qty: line.qty,
            // No amount and not a gap = this document has no money story for
            // this line, so the column stays EMPTY rather than showing "——".
            // A dash means "a number is missing here"; on a work order —
            // which must never carry prices — nothing is missing, and a
            // column of dashes read as a document that had failed to price
            // itself. (The demo engine has always stripped these; the real
            // one didn't, so the two engines disagreed on the same document.)
            amount: line.amountCents.map(amountString) ?? (line.isGap ? "——" : ""),
            isEdit: false, // Deferred 4: pre-filled-from-history affordance
            isGap: line.isGap,
            itemId: line.itemId
        )
    }

    static func document(_ payload: FFIDocumentPayload) -> DocumentModel {
        DocumentModel(
            rows: payload.lines.map(row),
            totalKey: totalLabel(payload.totalLabelKey),
            staticTotal: payload.staticTotalCents.map(amountString) ?? "——",
            note: note(for: payload.docKind, queued: payload.queued),
            send: sendLabel(for: payload.docKind),
            // Built-in kinds answer directly; a custom schema (Plan 19) is
            // priced iff core actually rendered money onto it — an amount or
            // a gap on any line. Guessing from the kind name alone would tell
            // an operator's own document type what it is allowed to be.
            pricesShown: DocKinds.isPricingKind(payload.docKind)
                || payload.lines.contains { $0.amountCents != nil || $0.isGap }
        )
    }

    static func emptyDocument() -> DocumentModel {
        DocumentModel(
            rows: [], totalKey: "TOTAL", staticTotal: "——", note: "", send: "SEND",
            pricesShown: false
        )
    }

    // MARK: - Notes mapping (Plan 13 D2/D3; Plan 14 D2-14 grows it with buckets)

    static func notes(_ payload: FFINotesPayload) -> NotesModel {
        NotesModel(
            summary: payload.summary,
            items: payload.items.map(Self.board),
            docKind: payload.docKind,
            queued: payload.queued,
            notes: payload.notes.map(Self.notesEntry)
        )
    }

    // Plan 14 D2-14: the FFI boundary already dropped any unknown-bucket row
    // (`crates/ffi/src/convert.rs::notes_entries`) — this `switch` over
    // `FFINotesBucket` is exhaustive and safe.
    static func notesEntry(_ entry: FFINotesEntry) -> NotesEntryFixture {
        let bucket: NotesBucket
        switch entry.bucket {
        case .scopeOfWork: bucket = .scopeOfWork
        case .constraints: bucket = .constraints
        case .conditionsAndIssues: bucket = .conditionsAndIssues
        }
        return NotesEntryFixture(bucket: bucket, label: entry.label, detail: entry.detail)
    }

    static func emptyNotes() -> NotesModel {
        NotesModel(summary: "", items: [], docKind: "report", queued: false)
    }

    // MARK: - Walk-log mapping (Plan 20 D2/D5)

    static func walkSummary(_ summary: FFIWalkSummary) -> WalkSummary {
        WalkSummary(
            id: summary.id,
            jobId: summary.jobId,
            docKind: summary.docKind,
            status: walkStatus(summary.status),
            summary: summary.summary,
            startedAt: summary.startedAt,
            itemCount: summary.itemCount,
            hasDocument: summary.hasDocument,
            queued: summary.queued
        )
    }

    static func walkStatus(_ status: FFIWalkStatus) -> WalkStatus {
        switch status {
        case .processing: return .processing
        case .processed: return .processed
        case .failed: return .failed
        }
    }
}
#endif
