import XCTest

@testable import SitewalkGallery

/// Gates on `EngineErrorText.readable` — what an operator actually reads when
/// something fails.
///
/// The anchor case is real: Isaac photographed this on TestFlight 2026-07-28,
/// rendered verbatim in a red bar on the notes screen. The payload inside it
/// named a genuine bug, so the goal is to keep every word of that and throw
/// away only the packaging.
///
/// The failure mode to guard against is over-cleaning. A helper that mangles a
/// StoreKit error, or swallows an unfamiliar shape into "something went wrong",
/// would cost us the next field diagnosis — which is the whole reason this text
/// is on screen.
final class EngineErrorTextTests: XCTestCase {
    /// Stands in for a UniFFI-generated error: `LocalizedError` whose
    /// description is `String(reflecting:)` of the case.
    private struct Reflected: LocalizedError {
        let text: String
        var errorDescription: String? { text }
    }

    // MARK: The case from the field

    func testUnwrapsTheRealTestFlightError() {
        let error = Reflected(text:
            #"MurmurCoreFFI.EngineError.Document(message: "document build error: invalid state: 'prf' is not a legal document kind for template Some(\"landscape\")")"#
        )
        XCTAssertEqual(
            EngineErrorText.readable(error),
            #"document build error: 'prf' is not a legal document kind for template Some("landscape")"#
        )
    }

    func testKeepsTheLeadingContextAndDropsOnlyTheVariantName() {
        // "document build error:" is context worth keeping. "invalid state:"
        // names a CoreError case — an implementation detail of the crate.
        let error = Reflected(text:
            #"MurmurCoreFFI.EngineError.Item(message: "invalid state: item is not editable")"#
        )
        XCTAssertEqual(EngineErrorText.readable(error), "item is not editable")
    }

    func testUnescapesEmbeddedQuotesAndBackslashes() {
        let error = Reflected(text:
            #"MurmurCoreFFI.EngineError.Store(message: "bad path \"C:\\jobs\" rejected")"#
        )
        XCTAssertEqual(EngineErrorText.readable(error), #"bad path "C:\jobs" rejected"#)
    }

    // MARK: Must not mangle anything else

    func testAPlainErrorPassesThroughUntouched() {
        // StoreKit and Foundation errors already have good descriptions; this
        // helper must leave them exactly alone.
        struct Plain: LocalizedError {
            var errorDescription: String? { "The network connection was lost." }
        }
        XCTAssertEqual(EngineErrorText.readable(Plain()), "The network connection was lost.")
    }

    func testAnUnfamiliarShapeFallsBackToTheFullDescription() {
        // A future UniFFI that formats differently must degrade to showing
        // everything, never to showing nothing.
        let error = Reflected(text: "SomeModule.Weird.case(code: 42)")
        XCTAssertEqual(EngineErrorText.readable(error), "SomeModule.Weird.case(code: 42)")
    }

    func testAnEmptyPayloadFallsBackRatherThanRenderingBlank() {
        // An empty red bar tells the operator nothing at all — worse than the
        // ugly version, because there is nothing to report back.
        let error = Reflected(text: #"MurmurCoreFFI.EngineError.Store(message: "")"#)
        XCTAssertFalse(EngineErrorText.readable(error).isEmpty)
    }

    func testTrimsSurroundingWhitespace() {
        let error = Reflected(text:
            #"MurmurCoreFFI.EngineError.Store(message: "  invalid state: locked  ")"#
        )
        XCTAssertEqual(EngineErrorText.readable(error), "locked")
    }
}
