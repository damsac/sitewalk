import XCTest

@testable import SitewalkGallery

/// Gates on editing a document's lines at review time — rename, add, remove,
/// and the amount rules that come with them.
///
/// Isaac, 2026-08-09: *"the user should be able to edit the name of any of the
/// lines, and also delete or add a line should they choose."* Review used to
/// offer exactly one edit, SET AMOUNT, which quietly said the only thing that
/// could be wrong on a document was a price. Speech-to-text mishears a word
/// about as often as it mishears a number, and the description is the part the
/// client actually reads.
///
/// These are model-level on purpose: the rules that will be wrong are the empty
/// cases (an emptied description, an emptied amount, a line added and
/// abandoned) and the one-way doors (money on a document that must not carry
/// it). None of those need a screen to be exercised, and all of them are
/// invisible on one.
@MainActor
final class DocumentLineEditTests: XCTestCase {
    private func model(pricesShown: Bool = true, rows: [DocRowFixture]? = nil) -> AppModel {
        let model = AppModel(engine: DemoWalkEngine(), forcedMode: .demo)
        model.document = DocumentModel(
            rows: rows ?? [
                DocRowFixture(title: "5 yards mulch", sub: "", qty: "", amount: "$200"),
                DocRowFixture(
                    title: "redo all garden beds", sub: "NOT HEARD — TAP OR SAY IT",
                    subWarn: true, qty: "", amount: "——", isGap: true, itemId: "item-2"
                ),
            ],
            totalKey: "TOTAL",
            staticTotal: "——",
            note: "",
            send: "SEND ESTIMATE",
            pricesShown: pricesShown
        )
        return model
    }

    // MARK: Renaming

    func testALineCanBeRenamed() {
        let model = model()
        let row = model.document!.rows[0]
        model.beginEdit(row)
        XCTAssertEqual(model.editTitle, "5 yards mulch", "the sheet opens on the current words")
        XCTAssertEqual(model.editText, "200", "and the current amount, without its $")

        model.editTitle = "5 yards dark mulch, installed"
        model.commitEdit()

        XCTAssertEqual(model.document?.rows[0].title, "5 yards dark mulch, installed")
        XCTAssertEqual(model.document?.rows[0].amount, "$200", "renaming left the money alone")
    }

    func testAnEmptiedDescriptionKeepsTheOldOne() {
        let model = model()
        model.beginEdit(model.document!.rows[0])
        model.editTitle = "   "
        model.commitEdit()

        XCTAssertEqual(
            model.document?.rows[0].title, "5 yards mulch",
            "there is no such thing as an untitled line on a document"
        )
        XCTAssertEqual(model.document?.rows.count, 2, "and nothing was removed")
    }

    func testRenamingKeepsTheItemIdThePhotosJoinOn() {
        let model = model()
        model.beginEdit(model.document!.rows[1])
        model.editTitle = "rebuild all garden beds"
        model.editText = "500"
        model.commitEdit()

        XCTAssertEqual(
            model.document?.rows[1].itemId, "item-2",
            "dropping it detaches the row from the photos taken against that item"
        )
    }

    // MARK: Amounts

    func testFillingAGapMarksItFilledAndClearsTheWarning() {
        let model = model()
        model.beginEdit(model.document!.rows[1])
        model.editText = "1,250"
        model.commitEdit()

        let row = model.document!.rows[1]
        XCTAssertEqual(row.amount, "$1,250", "typed money is written the way core writes it")
        XCTAssertFalse(row.isGap)
        XCTAssertFalse(row.subWarn)
        XCTAssertEqual(row.sub, "FILLED BY YOU")
        XCTAssertEqual(model.document?.gapCount, 0)
    }

    func testAnEmptiedAmountGoesBackToAGapRatherThanKeepingTheOldPrice() {
        let model = model()
        model.beginEdit(model.document!.rows[0])
        model.editText = ""
        model.commitEdit()

        let row = model.document!.rows[0]
        XCTAssertEqual(row.amount, "——", "a price nobody agreed to is the failure this prevents")
        XCTAssertTrue(row.isGap)
        XCTAssertTrue(row.subWarn)
        XCTAssertEqual(model.document?.gapCount, 2)
    }

    func testUnreadableTextLeavesTheAmountExactlyAsItWas() {
        let model = model()
        model.beginEdit(model.document!.rows[0])
        model.editText = "about two hundred"
        model.commitEdit()

        XCTAssertEqual(
            model.document?.rows[0].amount, "$200",
            "we cannot read it, so we do not act on it — never zero, never a guess"
        )
        XCTAssertFalse(model.document!.rows[0].isGap)
    }

    func testADocumentThatCarriesNoMoneyNeverGrowsAPrice() {
        let model = model(
            pricesShown: false,
            rows: [DocRowFixture(title: "haul the old bark away", sub: "", qty: "", amount: "")]
        )
        model.beginEdit(model.document!.rows[0])
        model.editTitle = "haul the old bark away"
        model.editText = "400"
        model.commitEdit()

        XCTAssertEqual(
            model.document?.rows[0].amount, "",
            "a work order handed to a crew with prices on it is the wrong document"
        )
        XCTAssertFalse(model.document!.rows[0].isGap)
    }

    // MARK: Adding and removing

    func testAddLineAppendsAnOpenLineAndOpensIt() {
        let model = model()
        model.addLine()

        XCTAssertEqual(model.document?.rows.count, 3)
        XCTAssertTrue(model.editingRowIsNew)
        XCTAssertEqual(model.editingRowID, model.document?.rows.last?.id)
        XCTAssertEqual(model.editTitle, "")

        model.editTitle = "haul-off and disposal"
        model.editText = "150"
        model.commitEdit()

        let added = model.document!.rows[2]
        XCTAssertEqual(added.title, "haul-off and disposal")
        XCTAssertEqual(added.amount, "$150")
        XCTAssertEqual(added.sub, "ADDED BY YOU", "the document says where the line came from")
        XCTAssertFalse(model.editingRowIsNew)
    }

    func testANewLinePricedLaterStartsAsAnHonestGap() {
        let model = model()
        model.addLine()
        model.editTitle = "haul-off"
        model.commitEdit()

        let added = model.document!.rows[2]
        XCTAssertEqual(added.amount, "——")
        XCTAssertTrue(added.isGap, "an unpriced line is a gap, whoever added it")
    }

    func testANewLineAbandonedWithoutWordsLeavesNothingBehind() {
        let model = model()
        model.addLine()
        // The sheet is dismissed by dragging it down, which commits.
        model.commitEdit()

        XCTAssertEqual(model.document?.rows.count, 2, "no blank line on the estimate")
        XCTAssertNil(model.editingRowID)
        XCTAssertFalse(model.editingRowIsNew)
    }

    func testAnExistingLineCanBeRemoved() {
        let model = model()
        model.beginEdit(model.document!.rows[0])
        model.removeEditingLine()

        XCTAssertEqual(model.document?.rows.count, 1)
        XCTAssertEqual(model.document?.rows.first?.title, "redo all garden beds")
        XCTAssertNil(model.editingRowID, "the sheet closes with it")
    }

    // MARK: Money

    func testALineWithCentsIsCountedInTheTotalAndKeepsBothDigits() {
        // The defect: every amount was parsed as an Int, so "$125.50" failed
        // to parse and was silently dropped from the total printed directly
        // beneath it. Spoken prices landing on lines verbatim made it likelier.
        let model = model(rows: [
            DocRowFixture(title: "stone", sub: "", qty: "", amount: "$125.50"),
            DocRowFixture(title: "labor", sub: "", qty: "", amount: "$500"),
        ])
        XCTAssertEqual(model.document?.totalValue, "$625.50")
    }

    func testWholeDollarsCarryNoDecimals() {
        XCTAssertEqual(DocumentModel.money(cents: 28500), "$285")
        XCTAssertEqual(DocumentModel.money(cents: 121000), "$1,210")
        XCTAssertEqual(DocumentModel.money(cents: 12550), "$125.50", "not $125.5")
        XCTAssertEqual(DocumentModel.money(cents: 12505), "$125.05")
    }

    func testAnAmountWithCentsSurvivesBeingOpenedAndSavedAgain() {
        let model = model(rows: [
            DocRowFixture(title: "stone", sub: "", qty: "", amount: "$125.50")
        ])
        model.beginEdit(model.document!.rows[0])
        XCTAssertEqual(model.editText, "125.50")
        model.commitEdit()
        XCTAssertEqual(model.document?.rows[0].amount, "$125.50")
    }

    func testTheTotalFollowsEveryEdit() {
        let model = model()
        XCTAssertEqual(model.document?.totalValue, "$200")

        model.beginEdit(model.document!.rows[1])
        model.editText = "500"
        model.commitEdit()
        XCTAssertEqual(model.document?.totalValue, "$700")

        model.beginEdit(model.document!.rows[0])
        model.removeEditingLine()
        XCTAssertEqual(model.document?.totalValue, "$500")
    }
}
