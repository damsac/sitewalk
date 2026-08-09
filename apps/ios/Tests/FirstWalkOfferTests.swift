import XCTest

@testable import SitewalkGallery

/// Gates on `FirstWalkOffer` — whether Pro is offered on the way out of a walk.
///
/// This fires **at most once per install**, which is exactly why it needs tests:
/// a mistake here does not announce itself. It either silently never happens —
/// and the only proactive price exposure in the whole app quietly does not
/// exist — or it happens at the one moment guaranteed to annoy, right after a
/// walk the operator watched fail.
final class FirstWalkOfferTests: XCTestCase {

    private func notes(items: Int = 0, entries: Int = 0) -> NotesModel {
        NotesModel(
            summary: "",
            items: (0..<items).map { _ in
                CapturedFixture(
                    tag: TagFixture(kind: .yellow, label: "MULCH"),
                    text: "Bark mulch, front beds",
                    right: "$285"
                )
            },
            docKind: "estimate",
            queued: false,
            notes: (0..<entries).map { _ in
                NotesEntryFixture(
                    bucket: .constraints, label: "Access", detail: "Gate code 4412"
                )
            }
        )
    }

    // MARK: The happy path

    func testOffersAfterAWalkThatCapturedLineItems() {
        XCTAssertTrue(
            FirstWalkOffer.shouldOffer(isPro: false, produced: notes(items: 3), alreadyOffered: false)
        )
    }

    func testOffersAfterAWalkThatCapturedOnlyCoordinationNotes() {
        // A walk can produce no priced line items and still be a real write-up
        // — "gate code is 4412, dog in the back yard". That is the product
        // working, so it counts.
        XCTAssertTrue(
            FirstWalkOffer.shouldOffer(isPro: false, produced: notes(entries: 2), alreadyOffered: false)
        )
    }

    // MARK: The three refusals

    func testNeverOffersToASubscriber() {
        XCTAssertFalse(
            FirstWalkOffer.shouldOffer(isPro: true, produced: notes(items: 3), alreadyOffered: false)
        )
    }

    func testNeverOffersTwice() {
        XCTAssertFalse(
            FirstWalkOffer.shouldOffer(isPro: false, produced: notes(items: 3), alreadyOffered: true)
        )
    }

    func testNeverOffersAfterAWalkThatCapturedNothing() {
        // The one that matters most. Asking for money straight after the
        // operator watched a walk come up empty is the worst read of the room
        // available, and it is the moment they are likeliest to delete the app.
        XCTAssertFalse(
            FirstWalkOffer.shouldOffer(isPro: false, produced: notes(), alreadyOffered: false)
        )
    }

    func testNeverOffersWhenThereAreNoNotesAtAll() {
        // Every exit path captures `notes` before it is nil'd; if that capture
        // is ever moved after the reset, this is what catches it.
        XCTAssertFalse(
            FirstWalkOffer.shouldOffer(isPro: false, produced: nil, alreadyOffered: false)
        )
    }

    // MARK: The empty-walk rule itself

    func testProducedSomethingMirrorsTheNotesScreensEmptyState() {
        // NotesView shows its empty state on `items.isEmpty && notes.isEmpty`.
        // If the screen says the walk came up empty, this must agree — an offer
        // rendered over that copy would directly contradict it.
        XCTAssertFalse(FirstWalkOffer.producedSomething(notes()))
        XCTAssertTrue(FirstWalkOffer.producedSomething(notes(items: 1)))
        XCTAssertTrue(FirstWalkOffer.producedSomething(notes(entries: 1)))
        XCTAssertTrue(FirstWalkOffer.producedSomething(notes(items: 1, entries: 1)))
    }
}
