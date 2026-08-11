import XCTest

@testable import SitewalkGallery

/// Gates on `DocumentSchemaModel.buildable(from:tradeKey:)` — the picker's
/// mirror of core's `resolve_active_schema`.
///
/// This exists because the two drifted in the field. `listDocumentSchemas`
/// returns nil-trade rows for EVERY trade, but core resolves a nil-trade schema
/// only for a nil-template session. So a landscape operator was shown REPORT and
/// a custom PRF, tapped either, and got:
///
///     'report' is not a legal document kind for template Some("landscape")
///
/// A button that cannot build the document it names is worse than a missing
/// button, so these pin the filter in both directions: nothing offered that core
/// refuses, and nothing dropped that core accepts.
final class DocumentChoiceTests: XCTestCase {
    private func schema(
        _ kind: String, label: String? = nil, trade: String?, updatedAt: UInt64 = 0,
        id: String = ""
    ) -> DocumentSchemaModel {
        DocumentSchemaModel(
            id: id.isEmpty ? kind : id,
            kind: kind,
            label: label ?? kind.capitalized,
            numberPrefix: String(kind.prefix(3)).uppercased(),
            tradeKey: trade,
            updatedAt: updatedAt
        )
    }

    // MARK: The field bug

    func testUniversalSchemaIsOfferedOnEveryTrade() {
        // Isaac, 2026-07-30: "They should come in regardless of trade!" A
        // nil-trade schema is UNIVERSAL, and core's resolver now agrees, so the
        // picker offers it everywhere.
        let all = [
            schema("estimate", trade: "landscape"),
            schema("report", trade: nil)
        ]
        for trade in ["landscape", "property", "inspection"] {
            let kinds = Set(DocumentSchemaModel.buildable(from: all, tradeKey: trade).map(\.kind))
            XCTAssertTrue(kinds.contains("report"), "report missing on \(trade)")
        }
    }

    func testACustomTypeWithNoTradeShowsUpEverywhere() {
        // The RFP case, which is what Isaac actually lost: a custom type
        // duplicated from Report carries nil and must work on every walk.
        let all = [schema("prf", label: "PRF", trade: nil)]
        XCTAssertEqual(
            DocumentSchemaModel.buildable(from: all, tradeKey: "landscape").map(\.kind), ["prf"]
        )
        XCTAssertEqual(
            DocumentSchemaModel.buildable(from: all, tradeKey: "property").map(\.kind), ["prf"]
        )
    }

    func testATradeSpecificSchemaBeatsAUniversalOneOfTheSameKind() {
        // Mirrors the resolver's ORDER BY: the operator's own version must
        // never be shadowed by a shared default, even if the default is newer.
        let all = [
            schema("report", label: "Shared report", trade: nil, updatedAt: 900, id: "universal"),
            schema("report", label: "My report", trade: "landscape", updatedAt: 100, id: "mine")
        ]
        let winners = DocumentSchemaModel.buildable(from: all, tradeKey: "landscape")
        XCTAssertEqual(winners.count, 1)
        XCTAssertEqual(winners.first?.id, "mine", "trade-specific must win over universal")
        // On a trade with no specific version, the universal one serves.
        XCTAssertEqual(
            DocumentSchemaModel.buildable(from: all, tradeKey: "property").first?.id, "universal"
        )
    }

    // MARK: Trade matching in general

    /// Universal must not become a back door: a schema naming a DIFFERENT trade
    /// still must not appear.
    func testAnotherTradesSchemaIsNeverOffered() {
        let all = [
            schema("condition", trade: "property"),
            schema("estimate", trade: "landscape")
        ]
        XCTAssertEqual(
            DocumentSchemaModel.buildable(from: all, tradeKey: "landscape").map(\.kind),
            ["estimate"]
        )
    }

    func testNilTradeSessionGetsOnlyTheUniversalSchemas() {
        // A walk with no template still must not see a trade-scoped schema.
        let all = [
            schema("report", trade: nil),
            schema("estimate", trade: "landscape")
        ]
        XCTAssertEqual(
            DocumentSchemaModel.buildable(from: all, tradeKey: nil).map(\.kind), ["report"]
        )
    }

    // MARK: One winner per kind

    func testTwoSchemasSharingAKindCollapseToTheNewest() {
        // Core builds ORDER BY updated_at DESC LIMIT 1, so only one is ever
        // built. Two buttons would mean one dead button — and duplicate
        // ForEach ids, which is undefined in SwiftUI and can swallow taps.
        let all = [
            schema("estimate", label: "Estimate", trade: "landscape", updatedAt: 100, id: "builtin"),
            schema("estimate", label: "My Estimate", trade: "landscape", updatedAt: 900, id: "custom")
        ]
        let winners = DocumentSchemaModel.buildable(from: all, tradeKey: "landscape")
        XCTAssertEqual(winners.count, 1)
        XCTAssertEqual(winners.first?.id, "custom", "newest updatedAt wins, matching core")
        XCTAssertEqual(
            winners.first?.label, "My Estimate",
            "the label must describe the schema that actually builds"
        )
    }

    func testAnExactTieIsBrokenByIdSoThePickerAgreesWithCore() {
        // Two schemas of equal rank — same trade-ness, same `updatedAt` — must
        // collapse to the SAME one the resolver picks, whatever order they
        // arrive in. Core closes its ORDER BY with `id ASC`; this mirrors it.
        //
        // Not a synthetic tie: every seeded built-in carries `updatedAt = 0`,
        // so a device that has duplicated one and not yet edited the copy is in
        // exactly this state.
        let a = schema("report", label: "A report", trade: nil, updatedAt: 0, id: "aaa")
        let z = schema("report", label: "Z report", trade: nil, updatedAt: 0, id: "zzz")
        for input in [[z, a], [a, z]] {
            XCTAssertEqual(
                DocumentSchemaModel.buildable(from: input, tradeKey: "landscape").first?.id, "aaa",
                "a tie must resolve by lowest id, not by input order"
            )
        }
    }

    func testDistinctKindsAreAllKeptAndSortedByLabel() {
        let all = [
            schema("work_order", label: "Work Order", trade: "landscape"),
            schema("estimate", label: "Estimate", trade: "landscape"),
            schema("invoice", label: "Invoice", trade: "landscape")
        ]
        XCTAssertEqual(
            DocumentSchemaModel.buildable(from: all, tradeKey: "landscape").map(\.label),
            ["Estimate", "Invoice", "Work Order"]
        )
    }

    func testEmptyInputIsEmptyNotACrash() {
        XCTAssertTrue(DocumentSchemaModel.buildable(from: [], tradeKey: "landscape").isEmpty)
    }
}
