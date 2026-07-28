import XCTest

@testable import SitewalkGallery

/// Gates on `AppModel.jobMatching` — the client-side auto-filing matcher.
///
/// This is the one place a walk can be filed WITHOUT the operator choosing, so
/// the failure that matters is a false positive: a walk silently landing under
/// the wrong job is worse than not filing at all, because nobody goes looking
/// for it there. These tests exist mostly to pin the cases where it must
/// decline.
final class JobMatchTests: XCTestCase {
    private func job(_ id: String, _ name: String) -> JobModel {
        JobModel(
            id: id, name: name, client: nil, site: nil, scheduledAt: nil,
            status: .active, createdAt: 0, updatedAt: 0
        )
    }

    func testMatchesSpokenJobNameMidSentence() {
        let jobs = [job("1", "117 Lexington"), job("2", "The Hendersons")]
        let transcript = "Okay so we're out here at 117 Lexington, front beds need mulch."
        XCTAssertEqual(AppModel.jobMatching(transcript: transcript, jobs: jobs)?.id, "1")
    }

    func testIgnoresCasePunctuationAndSpacing() {
        let jobs = [job("1", "117 Lexington")]
        // Whisper punctuates and capitalizes unpredictably; a match must
        // survive that or it will fire roughly never in the field.
        let transcript = "walking   117   LEXINGTON. Bed edging next."
        XCTAssertEqual(AppModel.jobMatching(transcript: transcript, jobs: jobs)?.id, "1")
    }

    func testDeclinesWhenTwoJobsBothMatch() {
        // Guessing here is a coin flip that hides the walk from the other job.
        let jobs = [job("1", "Maple Street"), job("2", "Maple Street Rear")]
        let transcript = "over at maple street rear today"
        XCTAssertNil(AppModel.jobMatching(transcript: transcript, jobs: jobs))
    }

    func testDeclinesOnVeryShortNames() {
        // A 2-3 character name collides with ordinary speech and would file
        // walks essentially at random.
        let jobs = [job("1", "A1")]
        let transcript = "a1 steak sauce came up somehow, anyway the beds"
        XCTAssertNil(AppModel.jobMatching(transcript: transcript, jobs: jobs))
    }

    func testDeclinesWhenNothingMatches() {
        let jobs = [job("1", "117 Lexington")]
        let transcript = "mulch the front beds and re-cut the edging"
        XCTAssertNil(AppModel.jobMatching(transcript: transcript, jobs: jobs))
    }

    func testDeclinesOnEmptyTranscript() {
        // A walk that captured nothing must not be filed anywhere.
        XCTAssertNil(AppModel.jobMatching(transcript: "", jobs: [job("1", "117 Lexington")]))
        XCTAssertNil(AppModel.jobMatching(transcript: "   ", jobs: [job("1", "117 Lexington")]))
    }

    func testDeclinesWithNoJobs() {
        XCTAssertNil(AppModel.jobMatching(transcript: "at 117 lexington", jobs: []))
    }

    func testPartialNameDoesNotMatch() {
        // "Lexington" alone must not match "117 Lexington" — the operator may
        // have several Lexington properties, and a whole-name hit is the only
        // signal strong enough to act on unasked.
        let jobs = [job("1", "117 Lexington")]
        let transcript = "we're on lexington today"
        XCTAssertNil(AppModel.jobMatching(transcript: transcript, jobs: jobs))
    }
}

/// Gates on `AppModel.walkDateLabel` — the job-card timestamp.
///
/// Jobs exist so an operator can answer "what happened here?" months later, so
/// a walk from March showing a bare "9:41" defeats the feature. These pin the
/// three branches and the boundaries between them.
final class WalkDateLabelTests: XCTestCase {
    private func epoch(_ iso: String) -> UInt64 {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return UInt64(f.date(from: iso)!.timeIntervalSince1970)
    }
    private func date(_ iso: String) -> Date {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f.date(from: iso)!
    }

    func testTodayShowsAClockTime() {
        // A date on something that happened an hour ago is noise.
        let label = AppModel.walkDateLabel(
            epochSeconds: epoch("2026-07-27T09:41:00Z"),
            now: date("2026-07-27T18:00:00Z")
        )
        XCTAssertTrue(label.contains(":"), "expected a clock time, got \(label)")
    }

    func testEarlierThisYearShowsMonthAndDayWithoutYear() {
        let label = AppModel.walkDateLabel(
            epochSeconds: epoch("2026-03-14T09:41:00Z"),
            now: date("2026-07-27T18:00:00Z")
        )
        XCTAssertTrue(label.contains("MAR"), "expected a month, got \(label)")
        XCTAssertFalse(label.contains("2026"), "same year shouldn't repeat the year: \(label)")
    }

    func testPriorYearCarriesTheYear() {
        // The "email months later" case — without a year this is ambiguous.
        let label = AppModel.walkDateLabel(
            epochSeconds: epoch("2025-11-02T09:41:00Z"),
            now: date("2026-07-27T18:00:00Z")
        )
        XCTAssertTrue(label.contains("2025"), "expected the year, got \(label)")
    }

    func testYesterdayIsNotTreatedAsToday() {
        // Guards the same-day check against being a naive 24-hour window.
        //
        // Built through Calendar rather than fixed UTC instants: the label uses
        // `isDate(_:inSameDayAs:)`, which is LOCAL-time based, so a UTC pair
        // straddling midnight-Z can still be the same local day. An earlier
        // version of this test asserted exactly that and failed against
        // correct code.
        let cal = Calendar.current
        let now = date("2026-07-27T18:00:00Z")
        let yesterdayEvening = cal.date(byAdding: .hour, value: -6, to: cal.startOfDay(for: now))!
        let label = AppModel.walkDateLabel(
            epochSeconds: UInt64(yesterdayEvening.timeIntervalSince1970),
            now: now
        )
        XCTAssertFalse(
            label.contains(":"),
            "yesterday must render as a date, not a clock time — got \(label)"
        )
    }

    func testMissingTimestampRendersEmptyNotEpochZero() {
        // Legacy/demo rows carry 0; "JAN 1 1970" would be worse than nothing.
        XCTAssertEqual(AppModel.walkDateLabel(epochSeconds: 0), "")
    }
}
