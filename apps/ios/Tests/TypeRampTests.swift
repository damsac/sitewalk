import XCTest

@testable import SitewalkGallery

/// Gates on `Theme.F.resolvedSize` — the one curve every font in the app now
/// passes through.
///
/// It exists because the ramp was the single highest-impact change in the design
/// review: `mono` was called at 7–9.5pt against an iOS body of 17, for an
/// audience whose median trade supervisor is 46 and needs reading glasses for
/// 8pt. Putting the shift in one function is what keeps the ratios between
/// levels intact — but it also means one wrong branch silently resizes the whole
/// app, so the branches are pinned here.
final class TypeRampTests: XCTestCase {
    /// Nothing may resolve below 11pt. This is the floor the whole review turns
    /// on: below it, type is not reliably legible in sun at arm's length, so
    /// there is no smaller size worth preserving.
    func testNothingResolvesBelowElevenPoints() {
        for size in stride(from: CGFloat(6), through: 11, by: 0.5) {
            XCTAssertGreaterThanOrEqual(
                Theme.F.resolvedSize(size), 11,
                "\(size)pt resolved below the 11pt floor"
            )
        }
    }

    func testTheSmallestStampedSizesLandOnTheFloor() {
        // 7 and 8pt were the worst offenders — tags and doc subtitles.
        XCTAssertEqual(Theme.F.resolvedSize(7), 11, accuracy: 0.01)
        XCTAssertEqual(Theme.F.resolvedSize(8), 11, accuracy: 0.01)
    }

    func testSmallTypeGetsTheMostHelp() {
        // 8.5 → ~11, 9 → ~11.7: the review's targets for MetaStrip and section
        // labels.
        XCTAssertEqual(Theme.F.resolvedSize(8.5), 11.05, accuracy: 0.05)
        XCTAssertEqual(Theme.F.resolvedSize(9), 11.7, accuracy: 0.05)
    }

    func testMidRangeMatchesTheReviewsTargets() {
        // The live transcript (review target 11.5 → 15) is read while walking
        // with the phone at hip height; the captured row (13 → ~16) is the
        // walk's output; the job row title (14.5 → ~17) is the board.
        XCTAssertEqual(Theme.F.resolvedSize(11.5), 14.95, accuracy: 0.05)
        XCTAssertEqual(Theme.F.resolvedSize(13), 16.54, accuracy: 0.05)
        XCTAssertEqual(Theme.F.resolvedSize(14.5), 17.85, accuracy: 0.05)
    }

    /// The two sizes where the flat-band version inverted. Pinned by value, not
    /// just by the monotonic sweep, so a regression names itself.
    func testTheOldBranchBoundariesAreContinuous() {
        XCTAssertEqual(Theme.F.resolvedSize(12), 15.6, accuracy: 0.01)
        XCTAssertEqual(Theme.F.resolvedSize(20), 21.6, accuracy: 0.01)
        XCTAssertGreaterThan(Theme.F.resolvedSize(12), Theme.F.resolvedSize(11.5))
        XCTAssertGreaterThan(Theme.F.resolvedSize(20), Theme.F.resolvedSize(19.5))
    }

    func testHeadlinesBarelyMove() {
        // Already legible; scaling them 30% would eat the layout rather than
        // help anyone.
        XCTAssertEqual(Theme.F.resolvedSize(26), 28.08, accuracy: 0.05)
        XCTAssertEqual(Theme.F.resolvedSize(30), 32.4, accuracy: 0.05)
    }

    /// The ramp has to stay a ramp. If a branch boundary is ever mis-set, two
    /// levels can cross and the hierarchy inverts — a subtitle rendering larger
    /// than the title it sits under.
    func testTheCurveIsMonotonic() {
        var previous = Theme.F.resolvedSize(6)
        for size in stride(from: CGFloat(6.5), through: 40, by: 0.5) {
            let current = Theme.F.resolvedSize(size)
            XCTAssertGreaterThanOrEqual(
                current, previous,
                "\(size)pt resolved smaller than the size below it — the ramp inverted"
            )
            previous = current
        }
    }

    func testGrowthNeverExceedsThirtyPercent() {
        // An upper bound as well as a lower one: unbounded growth would break
        // fixed-width frames (the time column, PAUSE, DISCARD) rather than
        // simply reflow.
        for size in stride(from: CGFloat(12), through: 40, by: 0.5) {
            XCTAssertLessThanOrEqual(
                Theme.F.resolvedSize(size), size * 1.30,
                "\(size)pt grew more than 30%"
            )
        }
    }
}
