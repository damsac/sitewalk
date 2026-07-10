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

struct DocRowFixture: Identifiable {
    let id = UUID()
    let title: String
    let sub: String
    var subWarn: Bool = false
    var hint: String? = nil
    let qty: String
    let amount: String
    var isEdit: Bool = false
    var isGap: Bool = false
    /// The core item this row was built from (Plan 12). `nil` for demo/
    /// fixture rows (no core ids), total/rollup lines, or rows built before
    /// Plan 12 landed. // sac: a demo could wire a stub id to preview grouping.
    var itemId: String?
}

struct TradeFixture {
    let key: String
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
    static func legalKinds(for templateKey: String) -> [String] {
        switch templateKey {
        case "landscape": return ["estimate", "invoice", "work_order"]
        case "property": return ["condition", "move_out"]
        case "inspection": return ["inspection"]
        default: return ["report"]
        }
    }

    /// The template's default/lead kind — `legalKinds(for:).first`. The one
    /// build-document button Task 7 wires per template calls this.
    static func primaryKind(for templateKey: String) -> String {
        legalKinds(for: templateKey).first ?? "report"
    }

    static func isPricingKind(_ kind: String) -> Bool {
        kind == "estimate" || kind == "invoice"
    }
}

enum Fixtures {

    static let landscape = TradeFixture(
        key: "landscape",
        dateLabel: "TUE — JUL 01",
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
            CapturedFixture(tag: TagFixture(kind: .plain, label: "ITEM"), text: "BARK MULCH — FRONT BEDS", right: "3 CU YD"),
            CapturedFixture(tag: TagFixture(kind: .plain, label: "ITEM"), text: "TRIM BOXWOOD", right: "× 4"),
            CapturedFixture(tag: TagFixture(kind: .yellow, label: "PART"), text: "IRRIG. HEAD — ZONE 2", right: "REPLACE", photos: 1),
            CapturedFixture(tag: TagFixture(kind: .plain, label: "ITEM"), text: "BED EDGING — FRONT", right: "~60 LF"),
            CapturedFixture(tag: TagFixture(kind: .green, label: "PRICE"), text: "TARGET TOTAL", right: "$1,200"),
        ],
        docKind: "ESTIMATE",
        docNo: "EST-0047",
        docDate: "JUL 01 2026",
        rows: [
            DocRowFixture(title: "Premium bark mulch — front beds", sub: "DELIVERED + INSTALLED", qty: "3 CU YD", amount: "$285"),
            DocRowFixture(title: "Boxwood trim, walkway line", sub: "SHAPE + HAUL CLIPPINGS", qty: "× 4", amount: "$140"),
            DocRowFixture(title: "Irrigation head — zone 2", sub: "PARTS + LABOR", hint: "↺ LAST 3: $110 · $120 · $125", qty: "× 1", amount: "$120", isEdit: true),
            DocRowFixture(title: "Bed edging, front beds", sub: "SPADE EDGE, RE-CUT", qty: "60 LF", amount: "$310"),
            DocRowFixture(title: "Haul & disposal", sub: "NOT HEARD — TAP OR SAY IT", subWarn: true, qty: "× 1", amount: "——", isGap: true),
            DocRowFixture(title: "Crew labor", sub: "2-MAN CREW · HALF DAY", qty: "4 HR", amount: "$355"),
        ],
        totalKey: "TOTAL",
        totalValue: "$1,210",
        note: "1 GAP LEFT — \u{201C}haul and disposal, ninety-five\u{201D} fills it. Never guessed for you.",
        send: "SEND ESTIMATE"
    )

    static let property = TradeFixture(
        key: "property",
        dateLabel: "TUE — JUL 01",
        countTitle: "5 units today",
        biz: "Corbett Property Group",
        bizCaps: "CORBETT PROPERTY GROUP",
        bizSub: "PORTLAND OR · 214 UNITS",
        boardMeta: "MOVE-OUTS · Q3 TURNS",
        openLabel: "4 OPEN",
        jobs: [
            JobFixture(time: "8:30", name: "Unit 204 — Gaslight", sub: "Move-out walkthrough", tag: TagFixture(kind: .green, label: "SENT"), done: true),
            JobFixture(time: "9:00", name: "Unit 117 — Gaslight", sub: "Move-out walkthrough", tag: TagFixture(kind: .plain, label: "NEXT")),
            JobFixture(time: "10:30", name: "48 Fremont St", sub: "Annual condition", tag: TagFixture(kind: .plain, label: "ANN")),
            JobFixture(time: "1:00", name: "Unit 09 — Kern Bldg", sub: "Move-in baseline", tag: TagFixture(kind: .yellow, label: "F/U")),
        ],
        site: "UNIT 117 · GASLIGHT",
        transcript: "main bedroom carpet is stained near the window, deduct cleaning… kitchen blinds missing two slats… walls are normal wear throughout… water heater tag reads twenty-nineteen, note it… balcony door drags — maintenance ticket",
        capturedCount: "5 ITEMS",
        captured: [
            CapturedFixture(tag: TagFixture(kind: .red, label: "DEDUCT"), text: "CARPET STAIN — BR 1", right: "CLEAN", photos: 2),
            CapturedFixture(tag: TagFixture(kind: .red, label: "DEDUCT"), text: "BLINDS, KITCHEN", right: "2 SLATS", photos: 1),
            CapturedFixture(tag: TagFixture(kind: .green, label: "OK"), text: "WALLS — NORMAL WEAR", right: "ALL RMS"),
            CapturedFixture(tag: TagFixture(kind: .plain, label: "NOTE"), text: "WATER HEATER — 2019", right: "LOGGED"),
            CapturedFixture(tag: TagFixture(kind: .yellow, label: "MAINT"), text: "BALCONY DOOR DRAGS", right: "TICKET"),
        ],
        docKind: "MOVE-OUT REPORT",
        docNo: "MO-0112",
        docDate: "JUL 01 2026",
        rows: [
            DocRowFixture(title: "Carpet — bedroom 1", sub: "STAIN NEAR WINDOW · PHOTO ×2", hint: "↺ SCHEDULE: CARPET CLEAN $140", qty: "DEDUCT", amount: "$140", isEdit: true),
            DocRowFixture(title: "Blinds — kitchen", sub: "2 SLATS MISSING · PHOTO ×1", qty: "DEDUCT", amount: "$45"),
            DocRowFixture(title: "Walls — all rooms", sub: "NORMAL WEAR AND TEAR", qty: "OK", amount: "—"),
            DocRowFixture(title: "Water heater", sub: "MFG 2019 · SERIAL LOGGED", qty: "NOTE", amount: "—"),
            DocRowFixture(title: "Garage remote", sub: "NOT HEARD — RETURNED? SAY IT", subWarn: true, qty: "DEDUCT", amount: "——", isGap: true),
        ],
        totalKey: "DEPOSIT DEDUCTION",
        totalValue: "$185",
        note: "PHOTOS PIN TO THE LINE YOU\u{2019}RE SPEAKING ABOUT — SAY \u{201C}PHOTO\u{201D} OR TAP",
        send: "SEND REPORT"
    )

    static let inspection = TradeFixture(
        key: "inspection",
        dateLabel: "TUE — JUL 01",
        countTitle: "2 inspections today",
        biz: "TrueLine Home Inspection",
        bizCaps: "TRUELINE HOME INSPECTION",
        bizSub: "AUSTIN TX · TREC 24119",
        boardMeta: "PRE-PURCHASE · 2 BOOKED",
        openLabel: "1 OPEN",
        jobs: [
            JobFixture(time: "8:30", name: "212 Garfield Ave", sub: "Pre-purchase · 1954 SFR", tag: TagFixture(kind: .green, label: "SENT"), done: true),
            JobFixture(time: "12:00", name: "77 Larkspur Ln", sub: "Pre-purchase · 2001 SFR", tag: TagFixture(kind: .plain, label: "NEXT")),
            JobFixture(time: "—", name: "Report follow-up", sub: "Buyer Q — 212 Garfield", tag: TagFixture(kind: .yellow, label: "F/U")),
            JobFixture(time: "—", name: "Thu hold", sub: "4-point · insurer req.", tag: TagFixture(kind: .plain, label: "HOLD")),
        ],
        site: "77 LARKSPUR LN",
        transcript: "roof: three lifted shingles on the south slope… attic ventilation adequate… hall bath GFCI won\u{2019}t trip — safety item… furnace filter overdue, maintenance… grading slopes to the foundation, northeast corner",
        capturedCount: "5 ITEMS",
        captured: [
            CapturedFixture(tag: TagFixture(kind: .yellow, label: "REPAIR"), text: "ROOF — LIFTED SHINGLES ×3", right: "S SLOPE", photos: 3),
            CapturedFixture(tag: TagFixture(kind: .green, label: "OK"), text: "ATTIC VENTILATION", right: "ADEQ."),
            CapturedFixture(tag: TagFixture(kind: .red, label: "SAFETY"), text: "GFCI HALL BATH — NO TRIP", right: "ELEC"),
            CapturedFixture(tag: TagFixture(kind: .yellow, label: "MAINT"), text: "FURNACE FILTER OVERDUE", right: "HVAC"),
            CapturedFixture(tag: TagFixture(kind: .yellow, label: "REPAIR"), text: "GRADING AT FOUNDATION", right: "NE COR", photos: 1),
        ],
        docKind: "INSPECTION",
        docNo: "IR-0389",
        docDate: "JUL 01 2026",
        rows: [
            DocRowFixture(title: "Roof covering — south slope", sub: "3 LIFTED SHINGLES · PHOTO ×3", qty: "REPAIR", amount: "§ 2.1"),
            DocRowFixture(title: "GFCI — hall bathroom", sub: "FAILS TO TRIP ON TEST", hint: "↺ AUTO-FILED FROM YOUR LAST 12 REPORTS", qty: "SAFETY", amount: "§ 6.4", isEdit: true),
            DocRowFixture(title: "Attic ventilation", sub: "RIDGE + SOFFIT, ADEQUATE", qty: "OK", amount: "§ 3.2"),
            DocRowFixture(title: "Furnace filter", sub: "REPLACEMENT OVERDUE", qty: "MAINT", amount: "§ 5.1"),
            DocRowFixture(title: "Water heater TPR valve", sub: "NOT ACCESSED — VERIFY OR EXCLUDE", subWarn: true, qty: "——", amount: "§ 5.3", isGap: true),
        ],
        totalKey: "FINDINGS",
        totalValue: "1 SAFETY · 3 REPAIR",
        note: "FINDINGS FILE INTO TREC SECTIONS AUTOMATICALLY — REORDER BY DRAG",
        send: "SEND REPORT"
    )

    static let all: [TradeFixture] = [landscape, property, inspection]
}
