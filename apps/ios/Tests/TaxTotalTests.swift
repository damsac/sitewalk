import XCTest

@testable import SitewalkGallery

/// Tax is an AMOUNT the operator types, never a rate the app computes — so
/// nothing here can be wrong about someone's arithmetic. What these pin is the
/// part that CAN go wrong: whether the number reaches the total.
@MainActor
final class TaxTotalTests: XCTestCase {
    private func priced(tax: String?, amounts: [String]) -> DocumentModel {
        DocumentModel(
            rows: amounts.map {
                DocRowFixture(title: "Work", sub: "", qty: "", amount: $0, isGap: false)
            },
            totalKey: "TOTAL", staticTotal: DocumentModel.noTotal, note: "", send: "SEND",
            sections: tax.map {
                [DocSectionFixture(key: "tax", label: "TAX", fields: [DocFieldFixture(
                    key: "tax", label: "Tax", value: $0,
                    isGap: false, isOptional: true, isParagraph: false
                )])]
            } ?? []
        )
    }

    /// Absent tax changes nothing. Most operators will never set it, and the
    /// document they get must be the one they got yesterday.
    func testNoTaxLeavesTheDocumentExactlyAsItWas() {
        let doc = priced(tax: nil, amounts: ["$180", "$420"])
        XCTAssertEqual(doc.totalLines.count, 1)
        XCTAssertEqual(doc.totalLines[0].value, "$600")
    }

    /// The whole point: the total INCLUDES it. A tax printed beside a total
    /// that ignored it would look paid for and not be counted.
    func testTaxProducesSubtotalTaxAndATotalThatIncludesIt() {
        let doc = priced(tax: "$49.50", amounts: ["$180", "$420"])
        let lines = doc.totalLines
        XCTAssertEqual(lines.map(\.key), ["SUBTOTAL", "TAX", "TOTAL"])
        XCTAssertEqual(lines[0].value, "$600")
        XCTAssertEqual(lines[1].value, "$49.50")
        XCTAssertEqual(lines[2].value, "$649.50", "the total is subtotal plus tax")
        XCTAssertTrue(lines[2].strong)
    }

    /// A tax field the operator left blank is not a $0 tax line.
    func testABlankTaxFieldAddsNothing() {
        XCTAssertEqual(priced(tax: "", amounts: ["$100"]).totalLines.count, 1)
        XCTAssertEqual(priced(tax: "0", amounts: ["$100"]).totalLines.count, 1)
    }

    /// A move-out report has a deposit ledger and no tax; if both were ever
    /// present the deposit arithmetic wins, because that document's total is
    /// a balance rather than a sum.
    func testTheDepositLedgerIsNotDisplacedByTax() {
        var doc = priced(tax: "$10", amounts: ["$185"])
        doc.totalKey = "DEPOSIT DEDUCTION"
        doc.sections.append(DocSectionFixture(
            key: "deposit", label: "DEPOSIT",
            fields: [DocFieldFixture(
                key: "deposit_held", label: "Deposit held", value: "$500",
                isGap: false, isOptional: true, isParagraph: false
            )]
        ))
        XCTAssertEqual(doc.totalLines.map(\.key).last, "BALANCE RETURNED")
    }
}
