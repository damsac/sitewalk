import PDFKit
import XCTest

@testable import SitewalkGallery

/// Pass B of the paperwork audit, the machine-checkable half.
///
/// The PDFs this app exports are RASTER — one rendered image per page — so no
/// test can read their words back. What a test can prove is that every kind
/// produces a file at all, that a document with authored blocks does not lose
/// them to the same crop that used to eat line items, and that the parts which
/// print nothing when empty still print nothing when they are the only thing
/// there. Reading the seven documents for trade correctness stays a human pass.
@MainActor
final class EveryKindRendersTests: XCTestCase {
    private func model(
        kind: String, sections: [DocSectionFixture], rows: Int, priced: Bool
    ) -> DocumentModel {
        DocumentModel(
            rows: (0..<rows).map { i in
                DocRowFixture(
                    title: "Item \(i + 1) as a walk would describe it",
                    sub: "",
                    qty: "1",
                    amount: priced ? "$120" : "",
                    isGap: false
                )
            },
            totalKey: priced ? "TOTAL" : "FINDINGS",
            staticTotal: DocumentModel.noTotal,
            note: "", send: "SEND",
            sections: sections,
            docNumber: "\(kind.prefix(3).uppercased())-0001",
            docKindLabel: kind.uppercased(),
            pricesShown: priced
        )
    }

    private func section(_ key: String, _ label: String, value: String?) -> DocSectionFixture {
        DocSectionFixture(
            key: key, label: label,
            fields: [DocFieldFixture(
                key: key, label: label, value: value, isGap: value == nil, isParagraph: true
            )]
        )
    }

    private func render(_ document: DocumentModel) throws -> PDFDocument {
        let url = try XCTUnwrap(
            DocumentPDF.render(trade: Fixtures.all[0], document: document),
            "no file produced"
        )
        defer { try? FileManager.default.removeItem(at: url) }
        return try XCTUnwrap(PDFDocument(url: url), "not a readable PDF")
    }

    /// All seven produce a file. Cheap, and it is the check that would have
    /// caught a kind wired to a renderer that returns nil.
    func testEveryKindProducesAPDF() throws {
        let kinds = [
            ("estimate", true), ("invoice", true), ("work_order", false),
            ("condition", false), ("move_out", true), ("inspection", false),
            ("report", false),
        ]
        for (kind, priced) in kinds {
            let doc = model(
                kind: kind,
                sections: [section("summary", "SUMMARY", value: "Two sentences about the walk.")],
                rows: 4, priced: priced
            )
            XCTAssertEqual(try render(doc).pageCount, 1, "\(kind) did not render one page")
        }
    }

    /// A work order carries three authored blocks and a long instruction
    /// paragraph. Blocks add height that line items do not, so this is the
    /// shape most likely to overflow — and before pagination it would have
    /// dropped the tasks rather than the prose, because the prose is on top.
    func testAWorkOrderWithFullBlocksAndManyTasksPaginates() throws {
        let long = String(repeating: "Strip the old bark before laying mulch. ", count: 12)
        let doc = model(
            kind: "work_order",
            sections: [
                section("assignment", "ASSIGNMENT", value: "Jose, Michael — Thursday first thing"),
                section("site", "SITE NOTES", value: "Gate code 4412. Dog in back until 8 AM."),
                section("instructions", "INSTRUCTIONS", value: long),
            ],
            rows: 30, priced: false
        )
        XCTAssertGreaterThan(
            try render(doc).pageCount, 1,
            "three blocks and 30 tasks fit on one page — the export is cropping"
        )
    }

    /// Every authored block empty is the ordinary case on a short walk. The
    /// blocks omit themselves, and what is left must still be a document
    /// rather than a failed render.
    func testADocumentWhoseBlocksAreAllEmptyStillRenders() throws {
        let doc = model(
            kind: "report",
            sections: [section("summary", "SUMMARY", value: nil)],
            rows: 2, priced: false
        )
        XCTAssertEqual(try render(doc).pageCount, 1)
    }
}
