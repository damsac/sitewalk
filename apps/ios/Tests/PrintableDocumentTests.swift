import XCTest

@testable import SitewalkGallery

/// Gates on what leaves the phone.
///
/// The review screen and the exported PDF render the same document from the
/// same model, and that is the point — "the PDF is pixel-identical to the
/// preview" is the promise the whole review screen rests on. But a handful of
/// marks on that screen are the app talking to the OPERATOR: gap prompts,
/// provenance, the price-book hint, the +N GAP badge. Isaac, 2026-08-09:
/// *"when the user hits send... little tag lines like 'added by you' are not
/// included. Overall these should be clean and professional looking."*
///
/// The failure mode is silent and outward-facing: nobody finds out from the
/// app that a client received "ADDED BY YOU" on their estimate. So the marks
/// live in one enum and these tests pin the filter over it — including a
/// sweep that fails if a NEW marker is ever added without being registered.
final class PrintableDocumentTests: XCTestCase {
    private func printable(_ row: DocRowFixture) -> DocRowFixture {
        DocumentPDF.printableRow(row)
    }

    // MARK: Provenance never prints

    func testProvenanceAndPromptsAreStripped() {
        for note in OperatorNote.all {
            let row = DocRowFixture(
                title: "5 yards mulch", sub: note, subWarn: note == OperatorNote.gap,
                qty: "3 CU YD", amount: "$200"
            )
            XCTAssertEqual(printable(row).sub, "", "\(note) reached the client's copy")
            XCTAssertFalse(printable(row).subWarn)
        }
    }

    /// The sweep: every operator-facing string the app can put in a `sub` has
    /// to be recognized. A new one added at a call site without registering
    /// it in `OperatorNote` would otherwise print on customers' paperwork.
    func testEveryKnownOperatorMarkerIsRecognized() {
        let marks = [
            OperatorNote.added, OperatorNote.filled, OperatorNote.gap,
            "NOT HEARD, TAP OR SAY IT",           // demo fixtures' older spelling
            "NOT HEARD, RETURNED? SAY IT",        // move-out fixture
            "NOT ACCESSED, VERIFY OR EXCLUDE",    // inspection fixture
        ]
        for mark in marks {
            XCTAssertTrue(
                OperatorNote.isOperatorFacing(mark),
                "\(mark) is not recognized as operator-facing — it will print"
            )
        }
    }

    func testTheComposedDetailLineSurvives() {
        // The one thing on that line written FOR the reader.
        let row = DocRowFixture(
            title: "Mulch the front beds",
            sub: "Strip the old bark first. Watch the irrigation heads.",
            qty: "", amount: ""
        )
        XCTAssertEqual(
            printable(row).sub, "Strip the old bark first. Watch the irrigation heads.",
            "the directive IS the work order"
        )
    }

    // MARK: The pricing leak

    func testThePriceBookHintNeverPrints() {
        // "LAST 3: $110 · $120 · $125" is what this operator charged their
        // last three customers. On a quote, that is a pricing leak.
        let row = DocRowFixture(
            title: "Irrigation head", sub: "", hint: "↺ LAST 3: $110 · $120 · $125",
            qty: "× 1", amount: "$120"
        )
        XCTAssertNil(printable(row).hint)
    }

    // MARK: Gaps

    func testAnUnfilledAmountPrintsBlankNotAWarningDash() {
        let row = DocRowFixture(
            title: "Haul & disposal", sub: OperatorNote.gap, subWarn: true,
            qty: "——", amount: "——", isGap: true
        )
        let out = printable(row)
        XCTAssertEqual(out.amount, "", "a yellow dashed —— is warning styling, not information")
        XCTAssertEqual(out.qty, "")
        XCTAssertFalse(out.isGap, "and it renders in plain ink, not in the gap treatment")
    }

    func testARealAmountOnAGapLineIsKept() {
        // An inspection's amount column carries section refs ("§ 5.3"), not
        // money — a gap there must not blank a value that was actually set.
        let row = DocRowFixture(
            title: "Water heater TPR valve", sub: "NOT ACCESSED, VERIFY OR EXCLUDE",
            subWarn: true, qty: "——", amount: "§ 5.3", isGap: true
        )
        XCTAssertEqual(printable(row).amount, "§ 5.3")
    }

    // MARK: What the reader is owed

    func testTheCrewNameSurvivesOnAWorkOrder() {
        let row = DocRowFixture(
            title: "Rebuild the fence panel", sub: "Match the existing pickets.",
            qty: "", amount: "", assignee: "Michael"
        )
        XCTAssertEqual(printable(row).assignee, "Michael", "the crew names ARE the work order")
    }

    func testUnwrittenSectionsAndFieldsAreDroppedFromThePrintedCopy() {
        let section = DocSectionFixture(
            key: "site",
            label: "SITE NOTES",
            fields: [
                DocFieldFixture(
                    key: "access", label: "Access", value: "Gate code 4412.",
                    isGap: false, isParagraph: true
                ),
                DocFieldFixture(
                    key: "safety", label: "Safety", value: nil, isGap: true, isParagraph: true
                ),
            ]
        )
        let out = DocumentPDF.printableSection(section)
        XCTAssertEqual(out.fields.count, 1, "a gap is a message to the operator")
        XCTAssertEqual(out.fields.first?.key, "access")
        XCTAssertTrue(section.hasContent)

        let allGaps = DocSectionFixture(
            key: "assignment",
            label: "ASSIGNMENT",
            fields: [DocFieldFixture(
                key: "crew", label: "Assigned to", value: nil, isGap: true, isParagraph: false
            )]
        )
        XCTAssertFalse(
            allGaps.hasContent,
            "a heading over an empty box reads as a broken document, so the block is omitted"
        )
    }
}
