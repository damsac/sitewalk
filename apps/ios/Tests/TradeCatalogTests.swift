import XCTest

@testable import SitewalkGallery

/// Gates on the trade catalog and on what a trade key controls.
///
/// Isaac, 2026-08-08: *"If someone who doesn't belong to one of the trades
/// listed gets the app, I don't want them to think this isn't for them."*
///
/// The risk in widening the picker is silent data damage: trade keys are
/// persisted in `BusinessProfile` AND written to `document_schemas.trade_key` in
/// core, so a renamed key orphans the operator's own document types, and a key
/// that fails to survive the profile round-trip re-files their walks under
/// somebody else's trade. Most of these pin those two failures.
final class TradeCatalogTests: XCTestCase {

    // MARK: The catalog itself

    func testTheThreeOriginalKeysStillExist() {
        // These are already persisted on every beta device. Renaming one
        // orphans that operator's custom document types in core.
        for key in ["landscape", "property", "inspection"] {
            XCTAssertTrue(
                Trades.catalog.contains { $0.key == key },
                "\(key) is a shipped key and must never be renamed or removed"
            )
        }
    }

    func testKeysAreUniqueAndNonEmpty() {
        let keys = Trades.catalog.map(\.key)
        XCTAssertEqual(Set(keys).count, keys.count, "duplicate trade key")
        XCTAssertFalse(keys.contains { $0.isEmpty })
    }

    func testCatalogIsMeaningfullyWiderThanThree() {
        // The whole point. If this ever collapses back toward three, the
        // "this isn't for me" bounce returns.
        XCTAssertGreaterThanOrEqual(Trades.catalog.count, 12)
    }

    func testCatalogEndsWithTheCatchAll() {
        XCTAssertEqual(Trades.catalog.last?.key, Trades.otherKey)
    }

    func testLabelFallsBackRatherThanRenderingBlank() {
        XCTAssertEqual(Trades.label(for: "landscape"), "Landscaping & lawn")
        // A key written by a future build must still render something.
        XCTAssertFalse(Trades.label(for: "septic_pumping").isEmpty)
    }

    // MARK: Every trade can make the general documents

    func testEveryTradeGetsTheGeneralFour() {
        // Mirrors core: Estimate/Invoice/Work Order/Report carry a NULL
        // trade_key and resolve for any template. If this drifts from core, the
        // fallback offers a button whose build then fails.
        for option in Trades.catalog {
            let kinds = DocKinds.legalKinds(for: option.key)
            for kind in ["estimate", "invoice", "work_order", "report"] {
                XCTAssertTrue(kinds.contains(kind), "\(option.key) is missing \(kind)")
            }
        }
    }

    func testAnUnknownTradeStillGetsTheGeneralFour() {
        let kinds = DocKinds.legalKinds(for: "marine_survey")
        XCTAssertEqual(Set(kinds), Set(DocKinds.generalKinds))
    }

    func testSpecialistKindsStayWithTheirTrades() {
        XCTAssertTrue(DocKinds.legalKinds(for: "property").contains("move_out"))
        XCTAssertTrue(DocKinds.legalKinds(for: "inspection").contains("inspection"))
        // A fencing contractor has no use for a move-out report.
        XCTAssertFalse(DocKinds.legalKinds(for: "fencing").contains("move_out"))
        XCTAssertFalse(DocKinds.legalKinds(for: "plumbing").contains("inspection"))
    }

    // MARK: The key must survive — this is the bug that prompted the tests

    func testAnUnknownTradeKeepsItsOwnKey() {
        // `BusinessProfile.trade` used to return Fixtures.landscape wholesale
        // for any unrecognised key, so `model.trade.key` came back "landscape"
        // — silently filing a plumber's walks and schemas under landscaping.
        let profile = BusinessProfile(
            businessName: "Ace Plumbing", cityState: "Denver CO",
            licenseNumber: nil, tradeKey: "plumbing"
        )
        XCTAssertEqual(profile.trade.key, "plumbing")
    }

    func testAKnownTradeStillGetsItsOwnFixture() {
        let profile = BusinessProfile(
            businessName: "X", cityState: "Y", licenseNumber: nil, tradeKey: "property"
        )
        XCTAssertEqual(profile.trade.key, "property")
        XCTAssertEqual(profile.trade.site, Fixtures.property.site)
    }

    // MARK: The "Something else" label

    func testCustomLabelIsPreferredWhenGiven() {
        let profile = BusinessProfile(
            businessName: "X", cityState: "Y", licenseNumber: nil,
            tradeKey: Trades.otherKey, tradeLabel: "Septic pumping"
        )
        XCTAssertEqual(profile.tradeDisplayName, "Septic pumping")
    }

    func testBlankCustomLabelFallsBackToTheCatalog() {
        for blank in [nil, "", "   "] {
            let profile = BusinessProfile(
                businessName: "X", cityState: "Y", licenseNumber: nil,
                tradeKey: Trades.otherKey, tradeLabel: blank
            )
            XCTAssertEqual(profile.tradeDisplayName, "Something else")
        }
    }

    /// A profile written before `tradeLabel` existed must still decode. It is
    /// read with `try?`, so a non-defaulted field would silently wipe every
    /// beta tester's profile and re-onboard them.
    func testProfilesWrittenBeforeTradeLabelStillDecode() throws {
        // Shape of a profile written before `tradeLabel` existed — every other
        // field present, that one absent.
        let legacy = """
        {"schemaVersion":1,"businessName":"Ace Lawn","cityState":"Denver CO",
         "tradeKey":"landscape"}
        """.data(using: .utf8)!
        let decoded = try JSONDecoder().decode(BusinessProfile.self, from: legacy)
        XCTAssertEqual(decoded.tradeKey, "landscape")
        XCTAssertNil(decoded.tradeLabel)
        XCTAssertEqual(decoded.tradeDisplayName, "Landscaping & lawn")
    }
}
