import XCTest

@testable import SitewalkGallery

/// Gates on `WalkRecord.title` / `.subtitle` — what a board row says a walk was.
///
/// Issue #221. The row led with `docNo`, which `WalkRecord(_ summary:)`
/// synthesizes EMPTY for every stored walk (the number is minted per-build and
/// isn't in the lightweight projection), over a `docKind` subtitle. So a
/// hydrated board rendered rows with blank titles under a label claiming an
/// "Estimate" that had never been built.
///
/// The rule these pin: **say what the walk was, never claim a document that
/// doesn't exist, and never render blank.**
final class WalkRowLabelTests: XCTestCase {
    private func walk(
        summary: String = "", docNo: String = "", docKind: String = "",
        itemCount: Int = 0, sent: Bool = false, queued: Bool = false
    ) -> AppModel.WalkRecord {
        AppModel.WalkRecord(
            time: "9:41", docNo: docNo, docKind: docKind, sent: sent,
            sessionId: "s1", queued: queued, summary: summary, itemCount: itemCount
        )
    }

    // MARK: Title

    func testSummaryIsTheHeadline() {
        let record = walk(summary: "Walked 117 Lexington — front beds need mulch.", itemCount: 4)
        XCTAssertEqual(record.title, "Walked 117 Lexington — front beds need mulch.")
    }

    func testFallsBackToTheDocumentNumberWhenThereIsARealOne() {
        // In-session records keep the real minted number until the next hydrate.
        XCTAssertEqual(walk(docNo: "EST-0047").title, "EST-0047")
    }

    func testNeverRendersBlank() {
        // The actual #221 symptom: every stored walk had docNo == "" and, before
        // summary was threaded through, nothing else to show.
        XCTAssertFalse(walk().title.isEmpty)
        XCTAssertEqual(walk().title, "Walk notes")
    }

    func testAStillProcessingWalkSaysSo() {
        XCTAssertEqual(walk(queued: true).title, "Walk still processing")
    }

    func testWhitespaceOnlySummaryIsTreatedAsAbsent() {
        XCTAssertEqual(walk(summary: "   \n ", docNo: "EST-0047").title, "EST-0047")
    }

    // MARK: Subtitle

    func testSubtitleCountsItems() {
        XCTAssertEqual(walk(itemCount: 5).subtitle, "5 items")
    }

    func testSubtitleSingularizesOneItem() {
        XCTAssertEqual(walk(itemCount: 1).subtitle, "1 item")
    }

    func testDocKindIsHiddenUntilADocumentActuallyExists() {
        // The other half of #221. A walk that was finished and never turned
        // into a document must not advertise "Estimate" — `docKind` is
        // advisory, and showing it reads as a claim.
        let notDocumented = walk(docKind: "ESTIMATE", itemCount: 3, sent: false)
        XCTAssertEqual(notDocumented.subtitle, "3 items")
    }

    func testDocKindAppearsOnceTheDocumentWasBuilt() {
        let documented = walk(docKind: "ESTIMATE", itemCount: 3, sent: true)
        XCTAssertEqual(documented.subtitle, "3 items · Estimate")
    }

    func testAQueuedWalkDoesNotClaimADocumentEvenIfSentIsSet() {
        // `queued` wins in `disposition`, so processing never shows a kind.
        let queued = walk(docKind: "ESTIMATE", itemCount: 2, sent: true, queued: true)
        XCTAssertEqual(queued.subtitle, "2 items")
    }
}
