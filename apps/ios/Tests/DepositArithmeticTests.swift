import XCTest

@testable import SitewalkGallery

/// A move-out report exists to answer one question: what does the tenant get
/// back. Summing deductions answers half of it.
@MainActor
final class DepositArithmeticTests: XCTestCase {
    private func moveOut(deposit: String?, deductions: [String]) -> DocumentModel {
        DocumentModel(
            rows: deductions.map {
                DocRowFixture(title: "Damage", sub: "", qty: "1", amount: $0, isGap: false)
            },
            totalKey: "DEPOSIT DEDUCTION",
            staticTotal: DocumentModel.noTotal,
            note: "", send: "SEND",
            sections: deposit.map {
                [DocSectionFixture(
                    key: "deposit", label: "DEPOSIT",
                    fields: [DocFieldFixture(
                        key: "deposit_held", label: "Deposit held", value: $0,
                        isGap: false, isOptional: true, isParagraph: false
                    )]
                )]
            } ?? []
        )
    }

    func testWithoutADepositItIsStillJustTheDeductionTotal() {
        let doc = moveOut(deposit: nil, deductions: ["$185"])
        XCTAssertEqual(doc.totalLines.count, 1, "no deposit typed, no arithmetic invented")
        XCTAssertEqual(doc.totalLines[0].value, "$185")
    }

    func testADepositProducesTheBalanceReturned() {
        let doc = moveOut(deposit: "$500", deductions: ["$120", "$65"])
        let lines = doc.totalLines
        // Held, then taken, then left: the order the sentence is spoken in.
        XCTAssertEqual(lines.map(\.key), ["DEPOSIT HELD", "DEPOSIT DEDUCTION", "BALANCE RETURNED"])
        XCTAssertEqual(lines[0].value, "$500")
        XCTAssertEqual(lines[1].value, "$185")
        XCTAssertEqual(lines[2].value, "$315", "500 − 185")
        XCTAssertTrue(lines[2].strong, "the balance is the number the tenant acts on")
    }

    /// Damage above the deposit is a real outcome, and it is OWED rather than
    /// a negative refund — a "-$85 returned" is not a sentence anyone can act
    /// on, and it is the moment a tenant most needs the document to be plain.
    func testDeductionsAboveTheDepositReadAsOwed() {
        let doc = moveOut(deposit: "$200", deductions: ["$285"])
        let last = doc.totalLines.last!
        XCTAssertEqual(last.key, "BALANCE OWED")
        XCTAssertEqual(last.value, "$85")
    }

    /// Typed by a thumb on a job site: "$1,250.50", "1250.5", " 500 ".
    func testTheDepositIsParsedTheWayItGetsTyped() {
        XCTAssertEqual(DocumentModel.parseCents("$1,250.50"), 125_050)
        XCTAssertEqual(DocumentModel.parseCents(" 500 "), 50_000)
        XCTAssertNil(DocumentModel.parseCents(""), "blank is not zero")
        XCTAssertNil(DocumentModel.parseCents("none"), "words are not money")
    }
}
