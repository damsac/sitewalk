import Foundation

// Header honesty (issue #155): this file is TWO things that should split when
// convenient. (1) The five struct types below (TagFixture, JobFixture,
// CapturedFixture, DocRowFixture, TradeFixture) are the app's INTERIM DOMAIN
// MODEL — the WalkEngine seam speaks them, and the real bridge maps core rows
// into them. (2) The `Fixtures` enum is canned demo/design data mirroring
// docs/design/mockup.html. Types are load-bearing; the canned data is not.

enum TagKind { case red, yellow, green, plain }

struct TagFixture: Hashable {
    let kind: TagKind
    let label: String
}

struct JobFixture: Identifiable {
    let id = UUID()
    let time: String
    let name: String
    let sub: String
    let tag: TagFixture
    var done: Bool = false
}

struct CapturedFixture: Identifiable {
    let id: UUID
    let tag: TagFixture
    let text: String
    let right: String
    var photos: Int = 0

    // Explicit init (not the implicit memberwise one) so callers that don't
    // care about identity keep getting a fresh UUID (fixtures, DemoWalkEngine
    // snapshots), while MurmurEngine's real-core adapter can thread the
    // core-assigned BoardItem.id through — ids must be stable across
    // `boardUpdated` snapshots for ForEach/lastCapturedID (Plan 07 Task 10).
    init(id: UUID = UUID(), tag: TagFixture, text: String, right: String, photos: Int = 0) {
        self.id = id
        self.tag = tag
        self.text = text
        self.right = right
        self.photos = photos
    }
}

/// Marks the app writes onto a document to talk to the OPERATOR — never to
/// whoever receives it.
///
/// "NOT HEARD — TAP OR SAY IT" is an instruction to the person holding the
/// phone. "ADDED BY YOU" and "FILLED BY YOU" are provenance, useful while
/// reviewing and faintly damaging on a client's copy: they advertise which
/// parts of the estimate the software got wrong. Isaac, 2026-08-09: *"when
/// the user hits send... little tag lines like 'added by you' are not
/// included. Overall these should be clean and professional looking."*
///
/// They live here, in one place, because the failure mode is silent: a new
/// marker added at a call site would simply start printing on customers'
/// paperwork, and nobody would find out from the app. `DocumentPDF` strips
/// everything in `all`, so a marker that is not registered here is a bug that
/// ships. Add new ones HERE, not as string literals.
enum OperatorNote {
    static let added = "ADDED BY YOU"
    static let filled = "FILLED BY YOU"
    static let gap = "NOT HEARD — TAP OR SAY IT"

    static let all: Set<String> = [added, filled, gap]

    /// True for any operator-facing mark, including the demo fixtures' older
    /// comma-spelled variants ("NOT HEARD, TAP OR SAY IT").
    static func isOperatorFacing(_ text: String) -> Bool {
        let upper = text.uppercased()
        return all.contains(text) || upper.hasPrefix("NOT HEARD") || upper.hasPrefix("NOT ACCESSED")
            || upper.hasSuffix("BY YOU")
    }
}

struct DocRowFixture: Identifiable {
    let id = UUID()
    /// `var` so review can rename a line in place (#…, Isaac 2026-08-09).
    /// Speech-to-text mishears a word about as often as it mishears a number,
    /// and the description is what the client actually reads.
    var title: String
    var sub: String
    var subWarn: Bool = false
    var hint: String? = nil
    /// `var` so the print path can blank a gap's "——" — a dash is warning
    /// styling for the operator, not information for the client.
    var qty: String
    /// `var` so a non-pricing document can blank it without rebuilding the row
    /// field by field (#222) — a work order must not carry prices.
    var amount: String
    var isEdit: Bool = false
    var isGap: Bool = false
    /// The core item this row was built from (Plan 12). `nil` for demo/
    /// fixture rows (no core ids), total/rollup lines, or rows built before
    /// Plan 12 landed. // sac: a demo could wire a stub id to preview grouping.
    var itemId: String?
    /// Who is doing this line — work orders only, where the money column is
    /// empty by design and that space belongs to the crew.
    var assignee: String?
}

struct TradeFixture {
    /// `var`, not `let`: an operator whose trade ships no fixture content
    /// borrows landscape's demo copy but KEEPS THEIR OWN KEY, because this key
    /// is what scopes their document schemas and tags their walk template. See
    /// `BusinessProfile.trade`.
    var key: String
    let dateLabel: String
    let countTitle: String
    let biz: String
    let bizCaps: String
    let bizSub: String
    let boardMeta: String
    let openLabel: String
    let jobs: [JobFixture]
    let site: String
    let transcript: String
    let capturedCount: String
    let captured: [CapturedFixture]
    let docKind: String
    let docNo: String
    let docDate: String
    let rows: [DocRowFixture]
    let totalKey: String
    let totalValue: String
    let note: String
    let send: String
}

/// Plan 13 D8 — the Swift mirror of core's `doc_kinds_for_template`/
/// `is_pricing_kind` (`crates/murmur-core/src/pipeline/mod.rs`): the legal
/// `kind` vocabulary per template, in priority order, and which kinds need a
/// price. Button WIRING (which kind an action calls `buildDocument` with)
/// keys off `TradeFixture.key` via this table, never off the FFI payload's
/// advisory `doc_kind` (D2). Which button *leads*, its label, and the full
/// per-trade button-set content are sac's (`docs/design/notes-mockup.html`);
/// this table only guarantees the wiring is correct.
enum DocKinds {
    /// Every trade quotes, bills, dispatches work and writes things up, so the
    /// general four are available to all of them — including a trade the app
    /// has never heard of. Specialists get their extras on top.
    ///
    /// This mirrors the seeded schemas after 2026-08-08: Estimate, Invoice,
    /// Work Order and Report carry a NULL `trade_key` (universal) in core, while
    /// Condition, Move-Out and Inspection stay scoped. This function is only the
    /// FALLBACK for when no schemas load at all — core's resolver is the real
    /// authority — but it must agree with it or the fallback would offer
    /// something the build then refuses.
    static let generalKinds = ["estimate", "invoice", "work_order", "report"]

    static func legalKinds(for templateKey: String) -> [String] {
        switch templateKey {
        case "property":   return generalKinds + ["condition", "move_out"]
        case "inspection": return generalKinds + ["inspection"]
        default:           return generalKinds
        }
    }

    /// The template's default/lead kind — `legalKinds(for:).first`. The one
    /// build-document button Task 7 wires per template calls this.
    static func primaryKind(for templateKey: String) -> String {
        legalKinds(for: templateKey).first ?? "report"
    }

    /// Notes-screen action-button copy per kind: the button title and a small
    /// stamp beneath it (the mockup's "ESTIMATE / QUOTE"). sac-owned taxonomy.
    static func label(for kind: String) -> String {
        switch kind {
        case "estimate": return "Estimate"
        case "invoice": return "Invoice"
        case "work_order": return "Work Order"
        case "condition": return "Condition"
        case "move_out": return "Move-Out"
        case "inspection": return "Inspection"
        default: return "Report"
        }
    }

    static func stamp(for kind: String) -> String {
        switch kind {
        case "estimate": return "QUOTE"
        case "invoice": return "BILL"
        case "work_order": return "CREW"
        case "condition": return "REPORT"
        case "move_out": return "DEPOSIT"
        // NOT "TREC REPORT". TREC is the Texas Real Estate Commission's
        // mandatory inspection form, which a licensed Texas inspector is
        // required to deliver on — and this app produces a findings list, not
        // that form. There is no TREC section mapping anywhere in the
        // codebase. An inspector reading that stamp would reasonably expect
        // the form and discover otherwise after the walk, which is the worst
        // possible moment.
        case "inspection": return "FINDINGS"
        default: return "EXPORT"
        }
    }

    /// Mirrors core's `is_pricing_kind`. `move_out` joined on 2026-08-09: the
    /// deduction total IS the move-out report, and the unpriced version could
    /// not do the one job it has.
    static func isPricingKind(_ kind: String) -> Bool {
        kind == "estimate" || kind == "invoice" || kind == "move_out"
    }

    /// Mirrors core's `total_shape` label key → the demo's display copy, so a
    /// demo invoice says AMOUNT DUE exactly like the real one.
    static func totalLabel(for kind: String) -> String {
        switch kind {
        case "invoice": return "AMOUNT DUE"
        case "move_out": return "DEPOSIT DEDUCTION"
        case "inspection": return "FINDINGS"
        default: return isPricingKind(kind) ? "TOTAL" : "ITEMS"
        }
    }
}

enum Fixtures {

    static let landscape = TradeFixture(
        key: "landscape",
        dateLabel: "TUE · JUL 01",
        countTitle: "4 sites today",
        biz: "Ridgeline Landscape Co.",
        bizCaps: "RIDGELINE LANDSCAPE CO.",
        bizSub: "DENVER CO · LIC 44-0781",
        boardMeta: "CREW A · TRUCK 02",
        openLabel: "3 OPEN",
        jobs: [
            JobFixture(time: "8:00", name: "1418 Alder Ct", sub: "Estimate walk · new client", tag: TagFixture(kind: .green, label: "SENT"), done: true),
            JobFixture(time: "9:30", name: "Hollis Residence", sub: "Spring cleanup · estimate", tag: TagFixture(kind: .plain, label: "NEXT")),
            JobFixture(time: "11:00", name: "Marston HOA", sub: "Irrigation check · zone map", tag: TagFixture(kind: .yellow, label: "F/U")),
            JobFixture(time: "1:30", name: "Beckwith Rental", sub: "Mulch + edging · quote", tag: TagFixture(kind: .plain, label: "EST")),
        ],
        site: "1418 ALDER CT",
        transcript: "front beds need mulch, call it three yards… trim the four boxwoods along the walk… zone two head is broken, replace it… edge the beds while we\u{2019}re in there… quote the whole thing around twelve hundred",
        capturedCount: "5 ITEMS",
        captured: [
            CapturedFixture(tag: TagFixture(kind: .plain, label: "ITEM"), text: "BARK MULCH, FRONT BEDS", right: "3 CU YD"),
            CapturedFixture(tag: TagFixture(kind: .plain, label: "ITEM"), text: "TRIM BOXWOOD", right: "× 4"),
            CapturedFixture(tag: TagFixture(kind: .yellow, label: "PART"), text: "IRRIG. HEAD, ZONE 2", right: "REPLACE", photos: 1),
            CapturedFixture(tag: TagFixture(kind: .plain, label: "ITEM"), text: "BED EDGING, FRONT", right: "~60 LF"),
            CapturedFixture(tag: TagFixture(kind: .green, label: "PRICE"), text: "TARGET TOTAL", right: "$1,200"),
        ],
        docKind: "ESTIMATE",
        docNo: "EST-0047",
        docDate: "JUL 01 2026",
        rows: [
            DocRowFixture(title: "Premium bark mulch, front beds", sub: "Delivered and installed", qty: "3 CU YD", amount: "$285"),
            DocRowFixture(title: "Boxwood trim, walkway line", sub: "Shaped, clippings hauled", qty: "× 4", amount: "$140"),
            DocRowFixture(title: "Irrigation head, zone 2", sub: "Parts and labor", hint: "↺ LAST 3: $110 · $120 · $125", qty: "× 1", amount: "$120", isEdit: true),
            DocRowFixture(title: "Bed edging, front beds", sub: "Spade edge, re-cut", qty: "60 LF", amount: "$310"),
            DocRowFixture(title: "Haul & disposal", sub: "NOT HEARD, TAP OR SAY IT", subWarn: true, qty: "× 1", amount: "——", isGap: true),
            DocRowFixture(title: "Crew labor", sub: "Two-man crew · half day", qty: "4 HR", amount: "$355"),
        ],
        totalKey: "TOTAL",
        totalValue: "$1,210",
        note: "1 GAP LEFT: \u{201C}haul and disposal, ninety-five\u{201D} fills it. Never guessed for you.",
        send: "SEND ESTIMATE"
    )

    static let property = TradeFixture(
        key: "property",
        dateLabel: "TUE · JUL 01",
        countTitle: "5 units today",
        biz: "Corbett Property Group",
        bizCaps: "CORBETT PROPERTY GROUP",
        bizSub: "PORTLAND OR · 214 UNITS",
        boardMeta: "MOVE-OUTS · Q3 TURNS",
        openLabel: "4 OPEN",
        jobs: [
            JobFixture(time: "8:30", name: "Unit 204, Gaslight", sub: "Move-out walkthrough", tag: TagFixture(kind: .green, label: "SENT"), done: true),
            JobFixture(time: "9:00", name: "Unit 117, Gaslight", sub: "Move-out walkthrough", tag: TagFixture(kind: .plain, label: "NEXT")),
            JobFixture(time: "10:30", name: "48 Fremont St", sub: "Annual condition", tag: TagFixture(kind: .plain, label: "ANN")),
            JobFixture(time: "1:00", name: "Unit 09, Kern Bldg", sub: "Move-in baseline", tag: TagFixture(kind: .yellow, label: "F/U")),
        ],
        site: "UNIT 117 · GASLIGHT",
        transcript: "main bedroom carpet is stained near the window, deduct cleaning… kitchen blinds missing two slats… walls are normal wear throughout… water heater tag reads twenty-nineteen, note it… balcony door drags — maintenance ticket",
        capturedCount: "5 ITEMS",
        captured: [
            CapturedFixture(tag: TagFixture(kind: .red, label: "DEDUCT"), text: "CARPET STAIN, BR 1", right: "CLEAN", photos: 2),
            CapturedFixture(tag: TagFixture(kind: .red, label: "DEDUCT"), text: "BLINDS, KITCHEN", right: "2 SLATS", photos: 1),
            CapturedFixture(tag: TagFixture(kind: .green, label: "OK"), text: "WALLS, NORMAL WEAR", right: "ALL RMS"),
            CapturedFixture(tag: TagFixture(kind: .plain, label: "NOTE"), text: "WATER HEATER, 2019", right: "LOGGED"),
            CapturedFixture(tag: TagFixture(kind: .yellow, label: "MAINT"), text: "BALCONY DOOR DRAGS", right: "TICKET"),
        ],
        docKind: "MOVE-OUT REPORT",
        docNo: "MO-0112",
        docDate: "JUL 01 2026",
        rows: [
            DocRowFixture(title: "Carpet, bedroom 1", sub: "Stain near the window · photo ×2", hint: "↺ SCHEDULE: CARPET CLEAN $140", qty: "DEDUCT", amount: "$140", isEdit: true),
            DocRowFixture(title: "Blinds, kitchen", sub: "Two slats missing · photo ×1", qty: "DEDUCT", amount: "$45"),
            DocRowFixture(title: "Walls, all rooms", sub: "Normal wear and tear", qty: "OK", amount: "—"),
            DocRowFixture(title: "Water heater", sub: "Mfg 2019 · serial logged", qty: "NOTE", amount: "—"),
            DocRowFixture(title: "Garage remote", sub: "NOT HEARD, RETURNED? SAY IT", subWarn: true, qty: "DEDUCT", amount: "——", isGap: true),
        ],
        totalKey: "DEPOSIT DEDUCTION",
        totalValue: "$185",
        note: "PHOTOS PIN TO THE LINE YOU\u{2019}RE SPEAKING ABOUT: SAY \u{201C}PHOTO\u{201D} OR TAP",
        send: "SEND REPORT"
    )

    static let inspection = TradeFixture(
        key: "inspection",
        dateLabel: "TUE · JUL 01",
        countTitle: "2 inspections today",
        biz: "TrueLine Home Inspection",
        bizCaps: "TRUELINE HOME INSPECTION",
        bizSub: "AUSTIN TX · TREC 24119",
        boardMeta: "PRE-PURCHASE · 2 BOOKED",
        openLabel: "1 OPEN",
        jobs: [
            JobFixture(time: "8:30", name: "212 Garfield Ave", sub: "Pre-purchase · 1954 SFR", tag: TagFixture(kind: .green, label: "SENT"), done: true),
            JobFixture(time: "12:00", name: "77 Larkspur Ln", sub: "Pre-purchase · 2001 SFR", tag: TagFixture(kind: .plain, label: "NEXT")),
            JobFixture(time: "—", name: "Report follow-up", sub: "Buyer Q, 212 Garfield", tag: TagFixture(kind: .yellow, label: "F/U")),
            JobFixture(time: "—", name: "Thu hold", sub: "4-point · insurer req.", tag: TagFixture(kind: .plain, label: "HOLD")),
        ],
        site: "77 LARKSPUR LN",
        transcript: "roof: three lifted shingles on the south slope… attic ventilation adequate… hall bath GFCI won\u{2019}t trip — safety item… furnace filter overdue, maintenance… grading slopes to the foundation, northeast corner",
        capturedCount: "5 ITEMS",
        captured: [
            CapturedFixture(tag: TagFixture(kind: .yellow, label: "REPAIR"), text: "ROOF, LIFTED SHINGLES ×3", right: "S SLOPE", photos: 3),
            CapturedFixture(tag: TagFixture(kind: .green, label: "OK"), text: "ATTIC VENTILATION", right: "ADEQ."),
            CapturedFixture(tag: TagFixture(kind: .red, label: "SAFETY"), text: "GFCI HALL BATH, NO TRIP", right: "ELEC"),
            CapturedFixture(tag: TagFixture(kind: .yellow, label: "MAINT"), text: "FURNACE FILTER OVERDUE", right: "HVAC"),
            CapturedFixture(tag: TagFixture(kind: .yellow, label: "REPAIR"), text: "GRADING AT FOUNDATION", right: "NE COR", photos: 1),
        ],
        docKind: "INSPECTION",
        docNo: "IR-0389",
        docDate: "JUL 01 2026",
        rows: [
            DocRowFixture(title: "Roof covering, south slope", sub: "Three lifted shingles · photo ×3", qty: "REPAIR", amount: "§ 2.1"),
            DocRowFixture(title: "GFCI, hall bathroom", sub: "Fails to trip on test", hint: "↺ AUTO-FILED FROM YOUR LAST 12 REPORTS", qty: "SAFETY", amount: "§ 6.4", isEdit: true),
            DocRowFixture(title: "Attic ventilation", sub: "Ridge and soffit, adequate", qty: "OK", amount: "§ 3.2"),
            DocRowFixture(title: "Furnace filter", sub: "Replacement overdue", qty: "MAINT", amount: "§ 5.1"),
            DocRowFixture(title: "Water heater TPR valve", sub: "NOT ACCESSED, VERIFY OR EXCLUDE", subWarn: true, qty: "——", amount: "§ 5.3", isGap: true),
        ],
        totalKey: "FINDINGS",
        totalValue: "1 SAFETY · 3 REPAIR",
        // Was "FINDINGS FILE INTO TREC SECTIONS AUTOMATICALLY" — see
        // `DocKinds.stamp`: there is no TREC mapping in the product, and this
        // demo is what an inspector decides on.
        note: "FINDINGS ARE GROUPED BY SEVERITY: SAFETY FIRST",
        send: "SEND REPORT"
    )

    static let all: [TradeFixture] = [landscape, property, inspection]
}
