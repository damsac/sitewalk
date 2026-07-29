import XCTest

@testable import SitewalkGallery

/// Gates on `WalkAllowance` — the free-tier meter's decision logic.
///
/// Two failure directions, both bad in different ways. Blocking a paying
/// subscriber, or blocking someone who has walks left, stops a contractor
/// working mid-job — the worst thing this app can do. Failing open lets the
/// free tier run unbounded, which costs real money at $-per-token. So these
/// pin both edges of the limit and every path through the month rollover,
/// which is the part that will rot silently: nothing runs at midnight on the
/// 1st, so if rollover is wrong it stays wrong until someone complains.
final class WalkAllowanceTests: XCTestCase {
    /// Fixed calendar in UTC so month boundaries are unambiguous under test.
    /// (Production uses `.current` on purpose — an operator's month is the one
    /// on their wall, not UTC's.)
    private let calendar: Calendar = {
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = TimeZone(identifier: "UTC")!
        return cal
    }()

    private func date(_ iso: String) -> Date {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        formatter.timeZone = TimeZone(identifier: "UTC")!
        return formatter.date(from: iso)!
    }

    private func record(_ month: String, _ count: Int) -> WalkAllowance.Record {
        WalkAllowance.Record(month: month, count: count)
    }

    // MARK: Month keys

    func testMonthKeyIsZeroPadded() {
        // "2026-7" would sort and compare wrong against "2026-11".
        XCTAssertEqual(
            WalkAllowance.monthKey(for: date("2026-07-28T12:00:00Z"), calendar: calendar),
            "2026-07"
        )
    }

    func testMonthKeyRollsAtTheYearBoundary() {
        XCTAssertEqual(
            WalkAllowance.monthKey(for: date("2026-12-31T23:59:00Z"), calendar: calendar),
            "2026-12"
        )
        XCTAssertEqual(
            WalkAllowance.monthKey(for: date("2027-01-01T00:01:00Z"), calendar: calendar),
            "2027-01"
        )
    }

    // MARK: Usage + rollover

    func testUsageCountsOnlyTheCurrentMonth() {
        let now = date("2026-07-28T12:00:00Z")
        XCTAssertEqual(
            WalkAllowance.usage(in: record("2026-07", 3), now: now, calendar: calendar), 3
        )
    }

    func testUsageFromAnEarlierMonthIsZero() {
        // The rollover. Nothing runs on the 1st — a stale record simply stops
        // counting, so an operator who used all 5 in June starts July at zero
        // without any scheduled work having to fire.
        let now = date("2026-07-01T00:00:00Z")
        XCTAssertEqual(
            WalkAllowance.usage(in: record("2026-06", 5), now: now, calendar: calendar), 0
        )
    }

    func testEmptyRecordIsZeroUsage() {
        XCTAssertEqual(
            WalkAllowance.usage(in: .empty, now: date("2026-07-28T12:00:00Z"), calendar: calendar),
            0
        )
    }

    // MARK: The gate

    func testProIsAlwaysAllowedAndNeverCounted() {
        // Even with a maxed-out record: Pro short-circuits before usage is read.
        let decision = WalkAllowance.decide(
            isPro: true, record: record("2026-07", 99),
            now: date("2026-07-28T12:00:00Z"), calendar: calendar
        )
        XCTAssertEqual(decision, .allowed(remaining: nil))
    }

    func testFreshFreeUserIsAllowedWithFullRemainder() {
        let decision = WalkAllowance.decide(
            isPro: false, record: .empty,
            now: date("2026-07-28T12:00:00Z"), calendar: calendar, limit: 5
        )
        // 4, not 5: `remaining` is what's left AFTER the walk being started.
        XCTAssertEqual(decision, .allowed(remaining: 4))
    }

    func testLastFreeWalkIsAllowedWithZeroRemaining() {
        // The off-by-one that matters: 4 used against a limit of 5 must still
        // let the 5th walk happen.
        let decision = WalkAllowance.decide(
            isPro: false, record: record("2026-07", 4),
            now: date("2026-07-28T12:00:00Z"), calendar: calendar, limit: 5
        )
        XCTAssertEqual(decision, .allowed(remaining: 0))
    }

    func testExactlyAtTheLimitIsBlocked() {
        let decision = WalkAllowance.decide(
            isPro: false, record: record("2026-07", 5),
            now: date("2026-07-28T12:00:00Z"), calendar: calendar, limit: 5
        )
        XCTAssertEqual(decision, .blocked(used: 5, limit: 5))
    }

    func testOverTheLimitIsBlockedRatherThanWrapping() {
        // Guards the `>=`. A record written by a build with a higher limit (or
        // a limit we later lower) must not read as "negative remaining, so
        // allowed" and hand out an unbounded free tier.
        let decision = WalkAllowance.decide(
            isPro: false, record: record("2026-07", 9),
            now: date("2026-07-28T12:00:00Z"), calendar: calendar, limit: 5
        )
        XCTAssertEqual(decision, .blocked(used: 9, limit: 5))
    }

    func testAMaxedOutPriorMonthDoesNotBlockTheNewMonth() {
        // The whole point of the rollover, stated as the gate sees it.
        let decision = WalkAllowance.decide(
            isPro: false, record: record("2026-06", 5),
            now: date("2026-07-01T08:00:00Z"), calendar: calendar, limit: 5
        )
        XCTAssertEqual(decision, .allowed(remaining: 4))
    }

    // MARK: Never block someone we can't sell to

    func testExhaustedButNoProductAvailableIsAllowed() {
        // The defect this exists to prevent: until the App Store Connect
        // product exists, NO install can subscribe. A hard gate would brick
        // every TestFlight tester at walk six with no way to pay and no
        // recourse until the 1st. Refusing someone's money and their work at
        // the same time is indefensible.
        let decision = WalkAllowance.decide(
            isPro: false, record: record("2026-07", 5),
            now: date("2026-07-28T12:00:00Z"), calendar: calendar, limit: 5,
            canSubscribe: false
        )
        XCTAssertEqual(decision, .allowed(remaining: nil))
    }

    func testTheLimitStillAppliesOnceAProductExists() {
        // The other half — failing open must be conditional, not a hole. The
        // same exhausted record blocks the moment a product is purchasable.
        let decision = WalkAllowance.decide(
            isPro: false, record: record("2026-07", 5),
            now: date("2026-07-28T12:00:00Z"), calendar: calendar, limit: 5,
            canSubscribe: true
        )
        XCTAssertEqual(decision, .blocked(used: 5, limit: 5))
    }

    func testNoProductDoesNotDisturbSomeoneUnderTheLimit() {
        // Failing open must not change what an under-limit user sees; it is a
        // release valve at the boundary, not a separate mode.
        let decision = WalkAllowance.decide(
            isPro: false, record: record("2026-07", 1),
            now: date("2026-07-28T12:00:00Z"), calendar: calendar, limit: 5,
            canSubscribe: false
        )
        // `nil` remaining, same as Pro: nothing is being counted down toward.
        XCTAssertEqual(decision, .allowed(remaining: nil))
    }

    // MARK: Recording a finish

    func testFinishIncrementsWithinTheSameMonth() {
        let updated = WalkAllowance.recordingFinish(
            in: record("2026-07", 2), now: date("2026-07-28T12:00:00Z"), calendar: calendar
        )
        XCTAssertEqual(updated, record("2026-07", 3))
    }

    func testFinishInANewMonthRestartsAtOne() {
        // Rolls over on WRITE as well as on read — otherwise a stale month
        // would keep accumulating and the next read would discard the lot.
        let updated = WalkAllowance.recordingFinish(
            in: record("2026-06", 5), now: date("2026-07-02T09:00:00Z"), calendar: calendar
        )
        XCTAssertEqual(updated, record("2026-07", 1))
    }

    func testFinishFromEmptyStartsAtOne() {
        let updated = WalkAllowance.recordingFinish(
            in: .empty, now: date("2026-07-28T12:00:00Z"), calendar: calendar
        )
        XCTAssertEqual(updated, record("2026-07", 1))
    }

    // MARK: The whole arc

    func testFiveWalksThenBlockedThenFreeAgainNextMonth() {
        // End-to-end on the pure logic: walk the free tier to exhaustion, prove
        // it blocks, then prove the calendar alone reopens it.
        let july = date("2026-07-15T10:00:00Z")
        var current = WalkAllowance.Record.empty

        for walk in 1...5 {
            let decision = WalkAllowance.decide(
                isPro: false, record: current, now: july, calendar: calendar, limit: 5
            )
            XCTAssertEqual(
                decision, .allowed(remaining: 5 - walk),
                "walk \(walk) of 5 should be allowed"
            )
            current = WalkAllowance.recordingFinish(in: current, now: july, calendar: calendar)
        }

        XCTAssertEqual(
            WalkAllowance.decide(
                isPro: false, record: current, now: july, calendar: calendar, limit: 5
            ),
            .blocked(used: 5, limit: 5),
            "the 6th walk in the same month must be refused"
        )

        XCTAssertEqual(
            WalkAllowance.decide(
                isPro: false, record: current,
                now: date("2026-08-01T00:00:00Z"), calendar: calendar, limit: 5
            ),
            .allowed(remaining: 4),
            "August must start fresh with no rollover job having run"
        )
    }

    func testSubscribingUnblocksImmediatelyWithoutTouchingTheRecord() {
        // What happens the instant a purchase completes: the same exhausted
        // record must stop blocking, and must NOT need to be reset — so that
        // cancelling later returns the operator to their honest usage rather
        // than to a laundered zero.
        let now = date("2026-07-28T12:00:00Z")
        let exhausted = record("2026-07", 5)

        XCTAssertEqual(
            WalkAllowance.decide(isPro: false, record: exhausted, now: now, calendar: calendar),
            .blocked(used: 5, limit: 5)
        )
        XCTAssertEqual(
            WalkAllowance.decide(isPro: true, record: exhausted, now: now, calendar: calendar),
            .allowed(remaining: nil)
        )
    }
}
