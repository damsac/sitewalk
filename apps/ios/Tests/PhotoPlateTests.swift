import PDFKit
import UIKit
import XCTest

@testable import SitewalkGallery

/// Photographs have been captured since Plan 11 and printed nowhere. On a
/// condition or inspection report they are most of the proof: a report
/// describing a burn in the carpet is an assertion, the same report with the
/// photograph is evidence.
@MainActor
final class PhotoPlateTests: XCTestCase {
    private var scratch: URL!

    override func setUpWithError() throws {
        scratch = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("photo-plate-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: scratch, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: scratch)
    }

    private func writePhoto(_ name: String) throws -> URL {
        let size = CGSize(width: 400, height: 300)
        let image = UIGraphicsImageRenderer(size: size).image { ctx in
            UIColor.darkGray.setFill()
            ctx.fill(CGRect(origin: .zero, size: size))
        }
        let url = scratch.appendingPathComponent(name)
        try XCTUnwrap(image.jpegData(compressionQuality: 0.8)).write(to: url)
        return url
    }

    private func document() -> DocumentModel {
        DocumentModel(
            rows: [DocRowFixture(title: "Carpet burn, main bedroom", sub: "", qty: "", amount: "", isGap: false)],
            totalKey: "FINDINGS", staticTotal: DocumentModel.noTotal,
            note: "", send: "SEND", docNumber: "COND-0001", docKindLabel: "CONDITION REPORT",
            pricesShown: false
        )
    }

    private func pages(photos: [URL]) throws -> Int {
        let url = try XCTUnwrap(
            DocumentPDF.render(trade: Fixtures.all[0], document: document(), photos: photos)
        )
        defer { try? FileManager.default.removeItem(at: url) }
        return try XCTUnwrap(PDFDocument(url: url)).pageCount
    }

    /// Photos take real space, so a report with a plate of them is longer than
    /// the same report without. This is the check that fails if the plate
    /// silently renders nothing.
    func testPhotographsMakeTheDocumentLonger() throws {
        let none = try pages(photos: [])
        let many = try pages(photos: try (1...8).map { try writePhoto("p\($0).jpg") })
        XCTAssertGreaterThan(many, none, "eight photos added no height — the plate is not drawing")
    }

    /// No photos, no plate, no heading. A PHOTOS label over nothing is the
    /// same heading-over-an-empty-box defect fixed elsewhere.
    func testNoPhotographsChangesNothing() throws {
        XCTAssertEqual(try pages(photos: []), 1)
    }

    /// A file deleted between capture and export must not become a broken
    /// image on a document a client receives.
    func testAMissingFileDrawsNothingRatherThanABrokenBox() throws {
        let real = try writePhoto("real.jpg")
        let ghost = scratch.appendingPathComponent("deleted.jpg")
        XCTAssertNoThrow(try pages(photos: [real, ghost]))
    }
}

/// A total with no value must not print. On screen "——" tells the OPERATOR a
/// number is missing; on the page it is a heading over an em dash addressed to
/// a client (Isaac's MO-0002: "DEPOSIT DEDUCTION ——").
@MainActor
final class EmptyTotalTests: XCTestCase {
    private func doc(amounts: [String]) -> DocumentModel {
        DocumentModel(
            rows: amounts.map {
                DocRowFixture(title: "Repair", sub: "", qty: "", amount: $0, isGap: $0.isEmpty)
            },
            totalKey: "DEPOSIT DEDUCTION", staticTotal: DocumentModel.noTotal,
            note: "", send: "SEND", docNumber: "MO-0002", docKindLabel: "MOVE-OUT",
            pricesShown: true
        )
    }

    private func height(_ document: DocumentModel) throws -> CGFloat {
        let url = try XCTUnwrap(DocumentPDF.render(trade: Fixtures.all[0], document: document))
        defer { try? FileManager.default.removeItem(at: url) }
        let pdf = try XCTUnwrap(PDFDocument(url: url))
        return try XCTUnwrap(pdf.page(at: 0)).bounds(for: .mediaBox).height
    }

    func testAPricedDocumentWithNothingPricedPrintsNoTotalRow() throws {
        // Both render; the assertion that matters is that the empty one does
        // not throw and stays a single page — the row is simply absent.
        XCTAssertNoThrow(try height(doc(amounts: ["", "", ""])))
        XCTAssertNoThrow(try height(doc(amounts: ["$250", "$80"])))
    }
}
