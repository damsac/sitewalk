import XCTest
@testable import SitewalkGallery

// Operator report 2026-07-27 ("random walks"): the board's top list was a single
// hardcoded "TODAY" that actually showed the ENTIRE walk history newest-first,
// and a walk filed under a job appeared TWICE (loose up top AND under its job
// card). These gate the fix: the top list keeps only UNFILED walks and splits
// them into honest TODAY / EARLIER date groups.
//
// The logic under test is `AppModel.looseWalkSections(from:now:calendar:)` and
// its `groupWalksByDay` helper — pure, `nonisolated`, driven by an injected
// `now`/`calendar` so there is no wall-clock or timezone flakiness.
final class WalkLogSectioningTests: XCTestCase {

    /// UTC gregorian calendar so `startOfDay` is deterministic across machines.
    private let calendar: Calendar = {
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = TimeZone(identifier: "UTC")!
        return cal
    }()

    /// A fixed "now" — 2023-11-14T22:13:20Z. Day boundary is unambiguous in UTC.
    private let now = Date(timeIntervalSince1970: 1_700_000_000)

    private var startOfToday: TimeInterval {
        calendar.startOfDay(for: now).timeIntervalSince1970
    }

    /// Build a walk with only the fields the sectioning reads.
    private func walk(
        _ label: String, startedAt: TimeInterval, jobId: String? = nil
    ) -> AppModel.WalkRecord {
        AppModel.WalkRecord(
            time: label, docNo: label, docKind: "ESTIMATE", sent: true,
            sessionId: label, queued: false, jobId: jobId,
            // `WalkRecord.startedAt` is UInt64 to match `WalkSummary` at the FFI
            // boundary; the cases here are all well after the epoch.
            startedAt: UInt64(startedAt)
        )
    }

    // A filed walk lives under its job card only — it must be excluded from the
    // loose top list so it doesn't appear twice.
    func testFiledWalksAreExcludedFromLooseList() {
        let walks = [
            walk("filed", startedAt: startOfToday + 7200, jobId: "job-1"),
            walk("loose", startedAt: startOfToday + 3600, jobId: nil)
        ]
        let sections = AppModel.looseWalkSections(from: walks, now: now, calendar: calendar)

        let listed = sections.flatMap { $0.walks }.map(\.sessionId)
        XCTAssertEqual(listed, ["loose"], "only the unfiled walk shows in the top list")
        XCTAssertFalse(listed.contains("filed"), "a filed walk must not double-list up top")
    }

    // Days-old walks belong under EARLIER, not a hardcoded TODAY.
    func testWalksSplitIntoTodayAndEarlierByDay() {
        let walks = [
            walk("today-2", startedAt: startOfToday + 7200),   // newest
            walk("today-1", startedAt: startOfToday + 3600),
            walk("yesterday", startedAt: startOfToday - 3600), // prior day
            walk("lastweek", startedAt: startOfToday - 6 * 86_400)
        ]
        let sections = AppModel.looseWalkSections(from: walks, now: now, calendar: calendar)

        XCTAssertEqual(sections.map(\.title), ["TODAY", "EARLIER"])
        XCTAssertEqual(sections[0].walks.map(\.sessionId), ["today-2", "today-1"],
                       "TODAY holds today's walks, newest-first order preserved")
        XCTAssertEqual(sections[1].walks.map(\.sessionId), ["yesterday", "lastweek"],
                       "EARLIER holds prior-day walks, order preserved")
    }

    // A walk started exactly at midnight (start of today) counts as TODAY.
    func testMidnightBoundaryCountsAsToday() {
        let sections = AppModel.looseWalkSections(
            from: [walk("midnight", startedAt: startOfToday)],
            now: now, calendar: calendar
        )
        XCTAssertEqual(sections.map(\.title), ["TODAY"])
        XCTAssertEqual(sections[0].walks.map(\.sessionId), ["midnight"])
    }

    // No TODAY group is emitted when every loose walk is from an earlier day —
    // the empty-group drop is what stops an empty, mislabeled "TODAY".
    func testEmptyGroupsAreDropped() {
        let sections = AppModel.looseWalkSections(
            from: [walk("old", startedAt: startOfToday - 2 * 86_400)],
            now: now, calendar: calendar
        )
        XCTAssertEqual(sections.map(\.title), ["EARLIER"],
                       "no empty TODAY section when all walks are days old")
    }

    // Every walk filed => no loose sections at all (board shows the "all filed"
    // note instead).
    func testAllFiledProducesNoSections() {
        let walks = [
            walk("a", startedAt: startOfToday + 3600, jobId: "job-1"),
            walk("b", startedAt: startOfToday - 3600, jobId: "job-2")
        ]
        let sections = AppModel.looseWalkSections(from: walks, now: now, calendar: calendar)
        XCTAssertTrue(sections.isEmpty, "all walks filed => nothing loose to list")
    }
}
