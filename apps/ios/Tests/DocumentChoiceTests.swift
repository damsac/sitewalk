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

    func testUniversalSchemaIsNotOfferedToATradeSession() {
        // The exact shape of Isaac's report: the shipped `report` built-in
        // carries trade_key nil and was listed for a landscape walk.
        let all = [
            schema("estimate", trade: "landscape"),
            schema("report", trade: nil)
        ]
        let kinds = DocumentSchemaModel.buildable(from: all, tradeKey: "landscape").map(\.kind)
        XCTAssertEqual(kinds, ["estimate"], "a nil-trade schema must not be offered to a trade walk")
    }

    func testCustomTypeCopiedFromReportIsNotOfferedUntilItHasATrade() {
        // The PRF case. Duplicating Report used to carry its nil trade through,
        // producing a doc type visible everywhere and buildable nowhere.
        let all = [schema("prf", label: "PRF", trade: nil)]
        XCTAssertTrue(DocumentSchemaModel.buildable(from: all, tradeKey: "landscape").isEmpty)
    }

    func testTheSameCustomTypeIsOfferedOnceItCarriesTheTrade() {
        // And the fix: `saveShape` now stamps the operator's trade, so their
        // own document type works. This is also how a landscaper gets a
        // working Report — duplicate it, and the copy is trade-scoped.
        let all = [schema("prf", label: "PRF", trade: "landscape")]
        XCTAssertEqual(
            DocumentSchemaModel.buildable(from: all, tradeKey: "landscape").map(\.kind), ["prf"]
        )
    }

    // MARK: Trade matching in general

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

    func testNilTradeSessionGetsExactlyTheUniversalSchemas() {
        // The mirror image — nil-to-nil matches, and a trade-scoped schema
        // must not leak into a session with no template.
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
