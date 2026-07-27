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
