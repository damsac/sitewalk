import XCTest

@testable import SitewalkGallery

/// Gates on `AppModel.firstSentence` — what a board row shows of a walk summary.
///
/// Isaac, on device 2026-07-29: *"Is there a way to make the walk description be
/// more condensed? Max one sentence?"* Rows were carrying things like *"Field
/// session to discuss mulch work. Only the word 'mulch' was clearly audible in
/// the recording, with no additional context provided about scope, timing, or
/// constraints."*
///
/// The risk in a fix like this is over-eager splitting: a period is not a
/// sentence end. `3 yd. of mulch` and `Alder Ct. beds` are both real things an
/// operator says, and truncating either to three words would be worse than the
/// verbosity. Most of these pin the cases where it must NOT split.
private extension String {
    /// Small helper so the fuzz corpus can build a long run cheaply.
    func repeatedString(_ n: Int) -> String { String(repeating: self, count: n) }
}

final class FirstSentenceTests: XCTestCase {

    // MARK: The reported case

    func testKeepsOnlyTheFirstSentenceOfAVerboseSummary() {
        let summary = "Field session to discuss mulch work. Only the word \"mulch\" was "
            + "clearly audible in the recording, with no additional context provided "
            + "about scope, timing, or constraints."
        // The boilerplate opener goes too (see the lead-in tests below), so
        // what survives is the part that actually says something.
        XCTAssertEqual(AppModel.firstSentence(of: summary), "Discuss mulch work.")
    }

    func testASingleSentenceIsLeftAlone() {
        let summary = "1418 Alder Ct — mulch, trim, zone-2 head."
        XCTAssertEqual(AppModel.firstSentence(of: summary), summary)
    }

    func testNoTerminatorAtAllIsLeftAlone() {
        // Plenty of good summaries have no full stop.
        let summary = "Marston HOA — irrigation check"
        XCTAssertEqual(AppModel.firstSentence(of: summary), summary)
    }

    // MARK: Must NOT split — this is where a naive version breaks

    func testDoesNotSplitOnAUnitAbbreviation() {
        // "3 yd." is not the end of a thought.
        let summary = "Needs 3 yd. of hardwood mulch across the front beds"
        XCTAssertEqual(AppModel.firstSentence(of: summary), summary)
    }

    func testDoesNotSplitOnAStreetAbbreviation() {
        let summary = "Walked Alder Ct. and the rear beds need edging"
        XCTAssertEqual(AppModel.firstSentence(of: summary), summary)
    }

    func testDoesNotSplitOnADecimal() {
        let summary = "Quoted 1250.00 for the whole job including haul away"
        XCTAssertEqual(AppModel.firstSentence(of: summary), summary)
    }

    func testDoesNotProduceAUselesslyShortFragment() {
        // A period inside the first few characters is an abbreviation, not a
        // sentence — splitting there would leave a title saying nothing.
        //
        // Asserted as a PREFIX rather than by equality: this string is longer
        // than the one-line limit, so the length cut legitimately applies. What
        // matters here is only that it didn't stop at "Mr."
        let summary = "Mr. Henderson wants the boxwoods shaped before the weekend"
        let result = AppModel.firstSentence(of: summary)
        XCTAssertTrue(
            result.hasPrefix("Mr. Henderson wants"),
            "split on the abbreviation: \(result)"
        )
    }

    // MARK: The boilerplate opener

    func testDropsTheFieldSessionLeadIn() {
        // Every summary in the on-device report opened this way.
        XCTAssertEqual(
            AppModel.firstSentence(of: "Field session to discuss mulch work."),
            "Discuss mulch work."
        )
        XCTAssertEqual(
            AppModel.firstSentence(of: "Field session at 1418 Alder Ct to scope mulch."),
            "1418 Alder Ct to scope mulch."
        )
    }

    func testKeepsTheLeadInWhenNothingUsefulWouldRemain() {
        // A summary that is ONLY the boilerplate must not become a fragment.
        XCTAssertEqual(
            AppModel.firstSentence(of: "Field session at site."),
            "Field session at site."
        )
    }

    func testDoesNotTrimTheSamePhraseFromTheMiddle() {
        // Anchored to the start, so this must survive intact.
        let summary = "Client asked about the field session to discuss mulch"
        XCTAssertEqual(AppModel.firstSentence(of: summary), summary)
    }

    // MARK: Length

    func testALongFirstSentenceIsCutOnAWordBoundary() {
        let summary = "Front beds need mulch and the boxwoods along the walkway all "
            + "need shaping before the client visit on Friday morning"
        let result = AppModel.firstSentence(of: summary)
        XCTAssertTrue(result.hasSuffix("…"), "expected an ellipsis, got \(result)")
        XCTAssertLessThanOrEqual(result.count, 57)
        // Cut between words, never mid-word.
        let body = result.dropLast()
        XCTAssertFalse(body.hasSuffix(" "), "trailing space before the ellipsis")
        XCTAssertTrue(
            summary.hasPrefix(body),
            "the kept text must be a real prefix of the summary"
        )
    }

    func testQuestionAndExclamationAlsoTerminate() {
        XCTAssertEqual(
            AppModel.firstSentence(of: "Is the zone 3 head covered? Client asked twice."),
            "Is the zone 3 head covered?"
        )
    }

    // MARK: Degenerate input

    func testEmptyAndWhitespaceComeBackEmpty() {
        XCTAssertEqual(AppModel.firstSentence(of: ""), "")
        XCTAssertEqual(AppModel.firstSentence(of: "   \n  "), "")
    }

    func testWhitespaceIsTrimmedFromTheEnds() {
        XCTAssertEqual(
            AppModel.firstSentence(of: "  Marston HOA — irrigation check  "),
            "Marston HOA — irrigation check"
        )
    }

    /// Fuzz. `firstSentence` does index arithmetic over model-generated text —
    /// the newest untrusted-input code on the board path, and a crash was
    /// reported on build 93. If it can be made to trap, that happens here rather
    /// than on a job site.
    func testSurvivesAdversarialInput() {
        let pieces = [
            "", " ", ".", "..", "...", "!", "?", ".!?", "  .  ", "\n", ".\n",
            "a.", "a. ", "a. B", "A.", "Field session ", "Field session at ",
            "Field session at .", "Field session to A.", "\u{00A0}", "\u{200B}",
            "e\u{0301}.", "\u{1F600}.", "\u{1F600}\u{1F600}. A", "İ. A",
            "ß. A", "\u{0041}\u{030A}. B", "— . —", "“mulch.” A", "3 yd. 4 yd.",
            "Mr. Mrs. Dr. A", "a".repeatedString(200) + ". B",
        ]
        // Every single piece, and every ordered pair, at several limits.
        for a in pieces {
            for b in pieces {
                for limit in [1, 2, 3, 8, 16, 56, 500] {
                    _ = AppModel.firstSentence(of: a + b, limit: limit)
                    _ = AppModel.firstSentence(of: b + a, limit: limit)
                }
            }
        }
    }

    /// A limit of zero or below must not trap on `prefix`/index math.
    func testDegenerateLimits() {
        for limit in [-5, 0, 1] {
            _ = AppModel.firstSentence(of: "Front beds need mulch and edging", limit: limit)
            _ = AppModel.firstSentence(of: "", limit: limit)
            _ = AppModel.firstSentence(of: "Field session at 1418 Alder Ct.", limit: limit)
        }
    }

    /// The row falls back to a plain label when there is no summary, so this
    /// must not resurrect a blank title (#221).
    func testAnEmptySummaryStillLetsTheRowFallBack() {
        let record = AppModel.WalkRecord(
            time: "9:41", docNo: "", docKind: "", sent: false,
            sessionId: "s1", queued: false, summary: "   ", itemCount: 0
        )
        XCTAssertEqual(record.title, "Walk notes")
    }
}
