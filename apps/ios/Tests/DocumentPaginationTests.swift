import PDFKit
import XCTest

@testable import SitewalkGallery

/// The export used to be one page, always.
///
/// `DocumentPDF.render` pinned the content to a 612x792 frame and called
/// `beginPage()` once, so a document taller than a page was not scaled or
/// fitted — it was cropped. Silently: no error, no warning, and nothing in the
/// suite looking. A twenty-line estimate is an ordinary walk, and the lines
/// that fell off the bottom were lines a client was being charged for.
///
/// These tests exist because that defect is invisible from every direction the
/// app normally looks. The unit tests passed. The screen was fine — the review
/// screen scrolls. Only the exported artifact was wrong, and only past a length
/// nobody happened to hit while checking.
@MainActor
final class DocumentPaginationTests: XCTestCase {
    private func document(rows: Int) -> DocumentModel {
        DocumentModel(
            rows: (0..<rows).map { i in
                DocRowFixture(
                    title: "Line item number \(i + 1), described the way a walk describes it",
                    sub: "A second line of detail, because composed rows carry one",
                    qty: "1",
                    amount: "$120",
                    isGap: false
                )
            },
            totalKey: "TOTAL",
            staticTotal: "——",
            note: "",
            send: "SEND",
            docNumber: "EST-9001",
            docKindLabel: "ESTIMATE"
        )
    }

    private func pageCount(rows: Int) throws -> Int {
        let url = try XCTUnwrap(
            DocumentPDF.render(trade: Fixtures.all[0], document: document(rows: rows)),
            "the renderer returned no file"
        )
        defer { try? FileManager.default.removeItem(at: url) }
        let pdf = try XCTUnwrap(PDFDocument(url: url), "the file is not a readable PDF")
        return pdf.pageCount
    }

    /// The common case is unchanged: a short document is still one page, and a
    /// fix that turned every estimate into two would be its own defect.
    func testAShortDocumentIsStillOnePage() throws {
        XCTAssertEqual(try pageCount(rows: 3), 1)
    }

    /// The defect, pinned. Forty priced lines cannot fit on one US-Letter page
    /// at this type size, so a single page here means content was dropped.
    func testALongDocumentRunsOntoMorePages() throws {
        let pages = try pageCount(rows: 40)
        XCTAssertGreaterThan(
            pages, 1,
            "40 lines came back as \(pages) page(s) — the export is cropping again"
        )
    }

    /// Length and page count move together. Without this, "more than one page"
    /// could be satisfied by a fixed two-page export that crops at line 30.
    func testPageCountGrowsWithTheDocument() throws {
        let short = try pageCount(rows: 10)
        let long = try pageCount(rows: 60)
        XCTAssertGreaterThan(
            long, short,
            "60 lines produced no more pages than 10 — length is not reaching the export"
        )
    }
}
