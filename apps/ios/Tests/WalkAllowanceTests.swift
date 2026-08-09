import XCTest

@testable import SitewalkGallery

/// Gates on `WalkAllowance` — the free-tier meter's decision logic.
///
/// Two failure directions, both bad in different ways. Blocking a paying
/// subscriber, or blocking someone who has walks left, stops a contractor
/// working mid-job — the worst thing this app can do. Failing open lets the
/// free tier run unbounded, which costs real money at $-per-token.
///
/// The allowance became **lifetime** on 2026-08-08 (it used to reset every
/// calendar month), so the month-rollover tests are gone and two new risks take
/// their place: an existing install's count must survive the format change, and
/// failing open matters more than it did — under a monthly reset, wrongly
/// blocking someone cost them days; now it would cost them the app.
final class WalkAllowanceTests: XCTestCase {

    private func record(_ count: Int) -> WalkAllowance.Record {
        WalkAllowance.Record(count: count)
    }

    // MARK: Migration off the monthly record

    /// The one-way door. Records on every existing install look like
    /// `{"month":"2026-08","count":3}`; the new shape has no `month`. If this
    /// decode ever breaks, `load()` falls back to `.empty` and **every install
    /// silently gets its free allowance back** — the failure is invisible and it
    /// is the expensive direction.
    func testALegacyMonthlyRecordCarriesItsCountForward() throws {
        let legacy = Data(#"{"month":"2026-08","count":3}"#.utf8)
        let decoded = try JSONDecoder().decode(WalkAllowance.Record.self, from: legacy)
        XCTAssertEqual(decoded.count, 3)
    }

    func testALegacyRecordFromAnOlderMonthAlsoCountsNow() throws {
        // Under the old rules a June record read as zero usage in August. Under
        // lifetime rules it counts — those walks did happen. Conservative on
        // purpose: nobody is handed a second free allowance by the migration.
        let legacy = Data(#"{"month":"2026-06","count":5}"#.utf8)
        let decoded = try JSONDecoder().decode(WalkAllowance.Record.self, from: legacy)
        XCTAssertEqual(
            WalkAllowance.decide(isPro: false, record: decoded, limit: 5),
            .blocked(used: 5, limit: 5)
        )
    }

    func testARecordRoundTripsThroughItsOwnEncoding() throws {
        let data = try JSONEncoder().encode(record(2))
        XCTAssertEqual(try JSONDecoder().decode(WalkAllowance.Record.self, from: data), record(2))
    }

    // MARK: The gate

    func testProIsAlwaysAllowedAndNeverCounted() {
        // Even with a maxed-out record: Pro short-circuits before usage is read.
        XCTAssertEqual(
            WalkAllowance.decide(isPro: true, record: record(99)),
            .allowed(remaining: nil)
        )
    }

    func testFreshFreeUserIsAllowedWithFullRemainder() {
        // 4, not 5: `remaining` is what's left AFTER the walk being started.
        XCTAssertEqual(
            WalkAllowance.decide(isPro: false, record: .empty, limit: 5),
            .allowed(remaining: 4)
        )
    }

    func testLastFreeWalkIsAllowedWithZeroRemaining() {
        // The off-by-one that matters: 4 used against a limit of 5 must still
        // let the 5th walk happen.
        XCTAssertEqual(
            WalkAllowance.decide(isPro: false, record: record(4), limit: 5),
            .allowed(remaining: 0)
        )
    }

    func testExactlyAtTheLimitIsBlocked() {
        XCTAssertEqual(
            WalkAllowance.decide(isPro: false, record: record(5), limit: 5),
            .blocked(used: 5, limit: 5)
        )
    }

    func testOverTheLimitIsBlockedRatherThanWrapping() {
        // Guards the `>=`. A record written by a build with a higher limit (or
        // a limit we later lower) must not read as "negative remaining, so
        // allowed" and hand out an unbounded free tier.
        XCTAssertEqual(
            WalkAllowance.decide(isPro: false, record: record(9), limit: 5),
            .blocked(used: 9, limit: 5)
        )
    }

    func testTheAllowanceNeverRefills() {
        // The whole behaviour change, stated once and directly. There is no
        // input — no date, no calendar — that turns an exhausted record back
        // into an allowed one while `isPro` is false.
        let exhausted = record(5)
        XCTAssertEqual(
            WalkAllowance.decide(isPro: false, record: exhausted, limit: 5),
            .blocked(used: 5, limit: 5)
        )
        XCTAssertEqual(WalkAllowance.remaining(in: exhausted, limit: 5), 0)
    }

    func testACorruptNegativeCountIsTreatedAsUnused() {
        // Defensive: a hand-edited or partially-written record must not produce
        // "remaining: 6" and hand out more than the limit.
        XCTAssertEqual(
            WalkAllowance.decide(isPro: false, record: record(-3), limit: 5),
            .allowed(remaining: 4)
        )
        XCTAssertEqual(WalkAllowance.remaining(in: record(-3), limit: 5), 5)
    }

    // MARK: Never block someone we can't sell to

    func testExhaustedButNoProductAvailableIsAllowed() {
        // The defect this exists to prevent: until the App Store Connect
        // product exists, NO install can subscribe. A hard gate would brick
        // every tester at walk six with no way to pay — and now with no month
        // rollover to wait for either, so the app would simply stop working.
        XCTAssertEqual(
            WalkAllowance.decide(isPro: false, record: record(5), limit: 5, canSubscribe: false),
            .allowed(remaining: nil)
        )
    }

    func testTheLimitStillAppliesOnceAProductExists() {
        // The other half — failing open must be conditional, not a hole. The
        // same exhausted record blocks the moment a product is purchasable.
        XCTAssertEqual(
            WalkAllowance.decide(isPro: false, record: record(5), limit: 5, canSubscribe: true),
            .blocked(used: 5, limit: 5)
        )
    }

    func testNoProductDoesNotDisturbSomeoneUnderTheLimit() {
        // Failing open must not change what an under-limit user sees; it is a
        // release valve at the boundary, not a separate mode.
        XCTAssertEqual(
            WalkAllowance.decide(isPro: false, record: record(1), limit: 5, canSubscribe: false),
            // `nil` remaining, same as Pro: nothing is being counted down toward.
            .allowed(remaining: nil)
        )
    }

    // MARK: Recording a finish

    func testFinishIncrements() {
        XCTAssertEqual(WalkAllowance.recordingFinish(in: record(2)), record(3))
    }

    func testFinishFromEmptyStartsAtOne() {
        XCTAssertEqual(WalkAllowance.recordingFinish(in: .empty), record(1))
    }

    func testFinishFromACorruptNegativeCountStartsAtOne() {
        XCTAssertEqual(WalkAllowance.recordingFinish(in: record(-2)), record(1))
    }

    // MARK: The board chip

    func testRemainingCountsDownAndFloorsAtZero() {
        XCTAssertEqual(WalkAllowance.remaining(in: .empty, limit: 5), 5)
        XCTAssertEqual(WalkAllowance.remaining(in: record(3), limit: 5), 2)
        XCTAssertEqual(WalkAllowance.remaining(in: record(5), limit: 5), 0)
        // Never negative — the chip would render "-4 left".
        XCTAssertEqual(WalkAllowance.remaining(in: record(9), limit: 5), 0)
    }

    // MARK: The whole arc

    func testFiveWalksThenBlockedForGood() {
        var current = WalkAllowance.Record.empty

        for walk in 1...5 {
            XCTAssertEqual(
                WalkAllowance.decide(isPro: false, record: current, limit: 5),
                .allowed(remaining: 5 - walk),
                "walk \(walk) of 5 should be allowed"
            )
            current = WalkAllowance.recordingFinish(in: current)
        }

        XCTAssertEqual(
            WalkAllowance.decide(isPro: false, record: current, limit: 5),
            .blocked(used: 5, limit: 5),
            "the 6th walk must be refused"
        )
        XCTAssertEqual(current, record(5))
    }

    func testSubscribingUnblocksImmediatelyWithoutTouchingTheRecord() {
        // What happens the instant a purchase completes: the same exhausted
        // record must stop blocking, and must NOT need to be reset — so that
        // cancelling later returns the operator to their honest usage rather
        // than to a laundered zero. That matters more now: a laundered record
        // would be a permanent free tier, not one extra month.
        let exhausted = record(5)

        XCTAssertEqual(
            WalkAllowance.decide(isPro: false, record: exhausted),
            .blocked(used: 5, limit: 5)
        )
        XCTAssertEqual(
            WalkAllowance.decide(isPro: true, record: exhausted),
            .allowed(remaining: nil)
        )
    }
}
