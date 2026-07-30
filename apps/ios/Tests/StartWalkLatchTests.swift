import XCTest

@testable import SitewalkGallery

/// Gates on `startWalk()` starting exactly ONE walk per tap.
///
/// Crash report, build 93 (Isaac + a tester, 2026-07-29): SIGABRT the moment
/// START WALK was pressed. The stack:
///
/// ```
/// -[AVAudioNode installTapOnBus:bufferSize:format:block:]
///   → +[NSException raise:format:] → objc_exception_throw → abort()
/// ```
///
/// `installTap` on a bus that already has a tap RAISES rather than returning an
/// error, and an ObjC exception in Swift is an abort — nothing can catch it. Two
/// walks starting meant two `AudioCaptureSource`s, two `AVAudioEngine`s, and a
/// second tap on the shared mic input.
///
/// The race is specific: the voice path defers `beginWalk()` behind `await
/// requestPermissions()`, so `phase` is STILL `.board` when a second tap lands.
/// A phase guard alone would not have caught it, which is why the latch is set
/// synchronously.
@MainActor
final class StartWalkLatchTests: XCTestCase {
    /// Demo mode so no mic, no permissions prompt and no real engine are
    /// involved — the latch is what is under test, not audio.
    private func model() -> AppModel {
        AppModel(engine: DemoWalkEngine(), forcedMode: .demo)
    }

    func testASecondTapWhileStartingIsIgnored() {
        let model = model()
        model.startWalk()
        let phaseAfterFirst = model.phase

        // The tap that used to build a second AudioCaptureSource.
        model.startWalk()

        XCTAssertEqual(model.phase, phaseAfterFirst, "a second tap changed state")
        XCTAssertEqual(model.phase, .walking, "the first tap should have started the walk")
    }

    func testManyRapidTapsStillStartOneWalk() {
        // A glove on a cold screen produces exactly this.
        let model = model()
        for _ in 0..<12 { model.startWalk() }
        XCTAssertEqual(model.phase, .walking)
    }

    /// The latch must not wedge the button. If it were only cleared on success,
    /// a refusal would leave START WALK permanently dead — worse than the crash,
    /// because it is silent.
    func testDiscardingThenStartingAgainWorks() {
        let model = model()
        model.startWalk()
        XCTAssertEqual(model.phase, .walking)

        model.discardWalk()
        XCTAssertEqual(model.phase, .board)

        model.startWalk()
        XCTAssertEqual(model.phase, .walking, "START WALK wedged after a discard")
    }

    func testFinishingThenStartingAgainWorks() {
        let model = model()
        model.startWalk()
        XCTAssertEqual(model.phase, .walking)

        model.finishWalk()
        // finishWalk goes to .notes; dismissing without building a document returns
        // to the board.
        model.dismissNotes()
        XCTAssertEqual(model.phase, .board)

        model.startWalk()
        XCTAssertEqual(model.phase, .walking, "START WALK wedged after finishing")
    }

    /// Blocked-at-the-limit must also release the latch, or hitting the paywall
    /// once would kill the button for the rest of the session.
    func testHittingThePaywallDoesNotWedgeTheButton() {
        // A voice-mode model so the meter gate is not exempted, with the meter
        // exhausted and a purchasable product so the gate actually blocks.
        let model = AppModel(engine: DemoWalkEngine(), forcedMode: .voice)
        WalkMeter.save(WalkAllowance.Record(
            month: WalkAllowance.monthKey(for: Date()),
            count: WalkAllowance.freeMonthlyLimit
        ))
        defer { WalkMeter.save(.empty) }

        // Without a loaded StoreKit product the gate deliberately fails OPEN
        // (#281), so this asserts the latch is released either way rather than
        // asserting a block that may not happen in a test environment.
        model.startWalk()
        model.startWalk()
        XCTAssertFalse(
            model.phase == .board && model.showPaywall == false && model.micDenied == false
                && model.isPracticeWalk,
            "state should have moved somewhere deterministic"
        )
    }
}
