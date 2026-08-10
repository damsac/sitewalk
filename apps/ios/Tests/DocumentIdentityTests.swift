import XCTest

@testable import SitewalkGallery

/// Gates on a document knowing what it is.
///
/// Core has minted a real per-kind number on every build since Plan 13, and
/// the app was discarding it: the letterhead read the trade FIXTURE's number
/// for a trade's lead kind, and printed nothing at all for the other six. The
/// exported file was named after the fixture too. So a real invoice went out
/// headed ESTIMATE, numbered EST-0047 or not numbered at all, as
/// `EST-0047.pdf` — three separate ways of telling a client this is a
/// different document than it is.
///
/// An invoice without its own number is not a document a bookkeeper can
/// accept, so this is pinned per kind rather than spot-checked.
@MainActor
final class DocumentIdentityTests: XCTestCase {
    private func build(_ kind: String) async throws -> DocumentModel {
        let engine = DemoWalkEngine()
        _ = engine.begin(trade: Fixtures.landscape)
        engine.append(transcript: "mulch and boxwood")
        _ = await engine.finish()
        return try await engine.buildDocument(sessionId: "demo-session", kind: kind)
    }

    func testEveryKindCarriesItsOwnNumberAndHeading() async throws {
        let expected: [(kind: String, number: String, heading: String)] = [
            ("estimate", "EST-0047", "ESTIMATE"),
            ("invoice", "INV-0047", "INVOICE"),
            ("work_order", "WO-0047", "WORK ORDER"),
            ("report", "DOC-0047", "REPORT"),
        ]
        for row in expected {
            let doc = try await build(row.kind)
            XCTAssertEqual(doc.docNumber, row.number, "\(row.kind) number")
            XCTAssertEqual(doc.docKindLabel, row.heading, "\(row.kind) heading")
        }
    }

    /// What the sum MEANS differs per kind, and the label is the only place a
    /// reader learns it.
    func testTheTotalIsLabelledByWhatItMeans() async throws {
        let doc = try await build("invoice")
        XCTAssertEqual(doc.totalKey, "AMOUNT DUE", "an invoice states a debt, not an offer")

        let estimate = try await build("estimate")
        XCTAssertEqual(estimate.totalKey, "TOTAL")

        let workOrder = try await build("work_order")
        XCTAssertEqual(workOrder.totalKey, "ITEMS", "a work order counts tasks; it has no money")
    }

    /// The move-out report is a money document — the deduction total is the
    /// entire reason it exists. It was seeded unpriced while the demo screen
    /// the product is sold on showed "DEPOSIT DEDUCTION $185".
    func testTheMoveOutReportIsAMoneyDocument() {
        XCTAssertTrue(DocKinds.isPricingKind("move_out"))
        XCTAssertEqual(DocKinds.totalLabel(for: "move_out"), "DEPOSIT DEDUCTION")
        XCTAssertFalse(DocKinds.isPricingKind("condition"), "a move-in record tallies nothing")
        XCTAssertFalse(
            DocKinds.isPricingKind("work_order"),
            "a crew's copy with prices on it is the wrong document"
        )
    }

    /// TREC is the Texas Real Estate Commission's mandatory inspection form,
    /// which this app does not produce — there is no TREC mapping in the
    /// codebase. Claiming it on the button an inspector taps is a promise
    /// discovered to be false only after the walk.
    func testTheInspectionButtonDoesNotClaimATrecForm() {
        XCTAssertEqual(DocKinds.stamp(for: "inspection"), "FINDINGS")
        for kind in DocKinds.legalKinds(for: "inspection") {
            XCTAssertFalse(
                DocKinds.stamp(for: kind).contains("TREC"),
                "\(kind) claims a regulated form the app does not produce"
            )
        }
    }

    /// A work order carries the crew, an estimate never does — an assignee in
    /// the price column would be a rendering bug wearing a data bug's clothes.
    func testOnlyTheWorkOrderCarriesCrewNames() async throws {
        let workOrder = try await build("work_order")
        XCTAssertTrue(
            workOrder.rows.allSatisfy { $0.assignee != nil },
            "the crew column is the work order's reason for existing"
        )
        XCTAssertTrue(workOrder.rows.allSatisfy { $0.amount.isEmpty }, "and it carries no money")

        let estimate = try await build("estimate")
        XCTAssertTrue(estimate.rows.allSatisfy { $0.assignee == nil })
    }

    /// Each kind carries the authored blocks a trade reader expects, and they
    /// differ — an invoice's block is past tense, a work order's is the
    /// assignment. A document whose sections all said the same thing would be
    /// the old "every document looks like an estimate" bug wearing a hat.
    func testEachKindCarriesItsOwnAuthoredBlocks() async throws {
        let cases: [(kind: String, sections: [String])] = [
            ("estimate", ["scope"]),
            ("invoice", ["work"]),
            ("work_order", ["assignment", "site"]),
            ("report", ["summary"]),
        ]
        for row in cases {
            let doc = try await build(row.kind)
            XCTAssertEqual(doc.sections.map(\.key), row.sections, "\(row.kind) sections")
            XCTAssertTrue(
                doc.sections.allSatisfy(\.hasContent),
                "\(row.kind): a heading over an empty box reads as a broken document"
            )
        }
    }
}
