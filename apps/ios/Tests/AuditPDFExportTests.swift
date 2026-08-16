import PDFKit
import XCTest

@testable import SitewalkGallery

/// Not a test — a generator. Renders one PDF per kind from the same walk so
/// the seven can be read side by side against the trade checklists (Pass B of
/// the paperwork audit). Skipped unless `AUDIT_PDFS=1`, so it never runs in CI.
@MainActor
final class AuditPDFExportTests: XCTestCase {
    func testExportOnePDFPerKind() throws {
        try XCTSkipUnless(
            ProcessInfo.processInfo.environment["AUDIT_PDFS"] == "1",
            "generator, not a check — set AUDIT_PDFS=1"
        )
        let out = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("audit-pdfs", isDirectory: true)
        try? FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)

        for spec in AuditFixtures.all {
            let url = try XCTUnwrap(
                DocumentPDF.render(
                    trade: Fixtures.all[0], document: spec.document,
                    biz: "Trimmers LLC", bizSub: "Landscape & property care",
                    docDate: "AUG 16 2026"
                )
            )
            let dest = out.appendingPathComponent("\(spec.name).pdf")
            try? FileManager.default.removeItem(at: dest)
            try FileManager.default.moveItem(at: url, to: dest)
            print("AUDIT_PDF \(dest.path)")
        }
    }
}

/// One walk — a landscape job with priced work, a crew, access notes and a
/// deposit — rendered as each kind, so differences between the documents are
/// differences in the DOCUMENTS rather than in the input.
enum AuditFixtures {
    struct Spec { let name: String; let document: DocumentModel }

    private static func field(_ k: String, _ l: String, _ v: String?, para: Bool = true)
        -> DocFieldFixture
    {
        DocFieldFixture(key: k, label: l, value: v, isGap: v == nil, isParagraph: para)
    }

    private static func rows(priced: Bool) -> [DocRowFixture] {
        [
            ("Weeding, garden beds", "$180"), ("Compost and mulch, garden beds", "$420"),
            ("Plants: 5 peppers, 6 tomatoes", "$95"), ("Weed eating, yard perimeter", "$140"),
            ("Blow off driveway", "$60"), ("Clean gutters", "$180"),
        ].map { title, amount in
            // NO quantity. The generator used to put "1" on every line, which
            // printed a column of stray 1s down all seven PDFs and read as an
            // app defect (Isaac, 2026-08-16). A real walk carries a quantity
            // only when one was spoken — "5 yards mulch" — so inventing one
            // here was the fixture lying about the product.
            DocRowFixture(
                title: title, sub: "", qty: "",
                amount: priced ? amount : "", isGap: false
            )
        }
    }

    private static func doc(
        _ kind: String, _ label: String, _ number: String,
        totalKey: String, priced: Bool, sections: [DocSectionFixture]
    ) -> DocumentModel {
        DocumentModel(
            rows: rows(priced: priced), totalKey: totalKey,
            staticTotal: DocumentModel.noTotal, note: "", send: "SEND",
            sections: sections, docNumber: number, docKindLabel: label,
            pricesShown: priced
        )
    }

    static var all: [Spec] {
        let scope = "We will weed the garden beds and yard perimeter, add compost and mulch, "
            + "and plant five pepper plants and six tomato plants. We will blow off the "
            + "driveway and clean the gutters. Compost and mulch go on after weeding."
        return [
            Spec(name: "1-estimate", document: doc(
                "estimate", "ESTIMATE", "EST-0001", totalKey: "TOTAL", priced: true,
                sections: [
                    DocSectionFixture(key: "client", label: "PREPARED FOR",
                        fields: [field("prepared_for", "Prepared for", "Mary Lou\n117 Lex St")]),
                    DocSectionFixture(key: "scope", label: "SCOPE OF WORK",
                        fields: [field("scope_summary", "Scope of work", scope)]),
                ])),
            Spec(name: "2-invoice", document: doc(
                "invoice", "INVOICE", "INV-0001", totalKey: "AMOUNT DUE", priced: true,
                sections: [
                    DocSectionFixture(key: "client", label: "PREPARED FOR",
                        fields: [field("prepared_for", "Prepared for", "Mary Lou\n117 Lex St")]),
                    DocSectionFixture(key: "work", label: "WORK PERFORMED",
                        fields: [field("work_summary", "Work performed",
                            scope.replacingOccurrences(of: "We will", with: "We"))]),
                ])),
            Spec(name: "3-work-order", document: doc(
                "work_order", "WORK ORDER", "WO-0001", totalKey: "TOTAL", priced: false,
                sections: [
                    DocSectionFixture(key: "assignment", label: "ASSIGNMENT", fields: [
                        field("crew", "Assigned to", "Jose, Michael", para: false),
                        field("schedule", "Scheduled", "Thursday first thing", para: false),
                    ]),
                    DocSectionFixture(key: "site", label: "SITE NOTES", fields: [
                        field("access", "Access", "Gate code 4412. Dog in back until 8 AM."),
                        field("safety", "Safety", "Irrigation heads sit shallow along the walkway."),
                    ]),
                    DocSectionFixture(key: "instructions", label: "INSTRUCTIONS",
                        fields: [field("instructions", "Instructions",
                            "Strip old bark before laying mulch. Jose takes mulch and bark, "
                            + "Michael takes edging. Work compost into the rose bed.")]),
                ])),
            Spec(name: "4-condition", document: doc(
                "condition", "CONDITION REPORT", "COND-0001", totalKey: "FINDINGS",
                priced: false, sections: [
                    DocSectionFixture(key: "summary", label: "SUMMARY",
                        fields: [field("summary", "Summary",
                            "117 Lex St, unit 2. Walked the yard and exterior at move-in. "
                            + "Beds overgrown, gutters full, driveway stained near the garage.")]),
                ])),
            Spec(name: "5-move-out", document: doc(
                "move_out", "MOVE-OUT REPORT", "MO-0001", totalKey: "DEPOSIT DEDUCTION",
                priced: true, sections: [
                    DocSectionFixture(key: "summary", label: "SUMMARY",
                        fields: [field("summary", "Summary",
                            "117 Lex St, unit 2, moved out Aug 12. Yard left overgrown "
                            + "beyond normal wear; gutters and driveway need attention.")]),
                    DocSectionFixture(key: "deposit", label: "DEPOSIT",
                        fields: [DocFieldFixture(key: "deposit_held", label: "Deposit held",
                            value: "$1,500", isGap: false, isOptional: true, isParagraph: false)]),
                ])),
            Spec(name: "6-inspection", document: doc(
                "inspection", "INSPECTION REPORT", "IR-0001", totalKey: "FINDINGS",
                priced: false, sections: [
                    DocSectionFixture(key: "summary", label: "SUMMARY",
                        fields: [field("summary", "Summary",
                            "Exterior and grounds inspection at 117 Lex St. Six findings, "
                            + "one safety related.")]),
                ])),
            Spec(name: "7-report", document: doc(
                "report", "REPORT", "DOC-0001", totalKey: "TOTAL", priced: false,
                sections: [
                    DocSectionFixture(key: "summary", label: "SUMMARY",
                        fields: [field("summary", "Summary",
                            "Site visit at 117 Lex St covering beds, perimeter, driveway "
                            + "and gutters.")]),
                ])),
        ]
    }
}
