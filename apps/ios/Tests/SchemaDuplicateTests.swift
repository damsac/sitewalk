import XCTest

@testable import SitewalkGallery

/// Gates on `DocumentSchemaModel.saveShape(of:editedFrom:)` — what the Document
/// Builder's SAVE button actually sends to `save_document_schema`.
///
/// The un-renamed duplicate is the case that had nothing pinning it. #306
/// changed exactly this path (it dropped #283's trade stamp), and the change is
/// invisible when the operator renames: renaming re-derives the kind, so the
/// copy is obviously a new type either way. It is the operator who taps SAVE on
/// a built-in and changes only its SECTIONS — the "I want the standard Report,
/// but with my paragraph" case — who is exposed to whether the trade is
/// inherited or stamped, and to whether the id is really cleared.
final class SchemaDuplicateTests: XCTestCase {
    private func builtin(
        _ kind: String, label: String, trade: String?, id: String
    ) -> DocumentSchemaModel {
        DocumentSchemaModel(
            id: id,
            kind: kind,
            label: label,
            numberPrefix: String(kind.prefix(3)).uppercased(),
            tradeKey: trade,
            updatedAt: 0,
            isBuiltin: true
        )
    }

    // MARK: The un-renamed duplicate

    func testDuplicatingTheUniversalReportWithoutRenamingStaysUniversal() {
        // Isaac, 2026-07-30: "They should come in regardless of trade!" A
        // landscaper who takes the shipped Report and edits its sections must
        // end up with a Report that still builds on every walk. Stamping their
        // trade here (what #283 did) would pin the copy to landscape and make
        // it vanish the day they walk a property.
        let original = builtin("report", label: "Report", trade: nil, id: "…0004")
        var draft = original
        draft.numberPrefix = "RPT"      // edited, but NOT renamed

        let saved = DocumentSchemaModel.saveShape(of: draft, editedFrom: original)

        XCTAssertNil(saved.tradeKey, "an un-renamed copy of a universal built-in stays universal")
        XCTAssertEqual(saved.kind, "report", "no rename means the kind is not re-derived")
        XCTAssertEqual(saved.id, "", "the copy must be a CREATE, not an overwrite of the built-in")
        XCTAssertFalse(saved.isBuiltin, "the copy is the operator's, not a shipped default")
        XCTAssertEqual(saved.numberPrefix, "RPT", "the operator's edit survives")
    }

    func testDuplicatingATradeScopedBuiltinWithoutRenamingKeepsThatTrade() {
        // The other half of "inherit, don't stamp": scope is not widened either.
        //
        // Move-Out is also the built-in whose label does not slug back to its
        // own kind ("Move-Out Report" -> "move_out_report", kind "move_out"), so
        // this is where re-deriving the kind on a save that did NOT rename would
        // show up — as a copy that answers to a kind core has no items for.
        let original = builtin("move_out", label: "Move-Out Report", trade: "property", id: "…0006")
        var draft = original
        draft.numberPrefix = "MO"

        let saved = DocumentSchemaModel.saveShape(of: draft, editedFrom: original)

        XCTAssertEqual(saved.tradeKey, "property", "the copy keeps the scope it was copied from")
        XCTAssertEqual(saved.kind, "move_out", "no rename means the kind is NOT re-derived")
        XCTAssertEqual(saved.id, "")
    }

    func testAnUnrenamedCopyStillCollidesWithItsBuiltinAndWinsOnlyByBeingNewer() {
        // The consequence of keeping the kind: the copy and the built-in share
        // one kind, so `buildable` collapses them to one button. Core stamps a
        // real clock on the save while every built-in sits at updatedAt 0, so
        // the operator's copy is the one that builds.
        let original = builtin("report", label: "Report", trade: nil, id: "…0004")
        var saved = DocumentSchemaModel.saveShape(of: original, editedFrom: original)
        saved.id = "minted-by-core"
        saved.updatedAt = 1_700_000_000

        let winners = DocumentSchemaModel.buildable(from: [original, saved], tradeKey: "landscape")
        XCTAssertEqual(winners.count, 1, "one kind, one button")
        XCTAssertEqual(winners.first?.id, "minted-by-core", "the operator's copy is what builds")
    }

    // MARK: Rename, and the non-built-in passthrough

    func testRenamingRederivesTheKindSoTheButtonBuildsWhatItSays() {
        let original = builtin("report", label: "Report", trade: nil, id: "…0004")
        var draft = original
        draft.label = "Property Request Form"

        let saved = DocumentSchemaModel.saveShape(of: draft, editedFrom: original)

        XCTAssertEqual(saved.kind, "property_request_form")
        XCTAssertNil(saved.tradeKey, "renaming does not scope it either")
        XCTAssertEqual(saved.id, "")
    }

    func testTheOperatorsOwnTypeIsSavedUnchanged() {
        // Not a built-in: this is an EDIT of something they already own, so the
        // id must survive or the edit forks a second copy on every save.
        var original = builtin("hoa_addendum", label: "HOA Addendum", trade: nil, id: "mine-1")
        original.isBuiltin = false
        var draft = original
        draft.label = "HOA Addendum v2"

        let saved = DocumentSchemaModel.saveShape(of: draft, editedFrom: original)

        XCTAssertEqual(saved.id, "mine-1", "editing your own type updates it in place")
        XCTAssertEqual(saved.kind, "hoa_addendum", "a saved type's kind is frozen")
        XCTAssertEqual(saved.label, "HOA Addendum v2")
    }

    // MARK: slug

    func testSlugCollapsesPunctuationAndTrimsUnderscores() {
        XCTAssertEqual(DocumentSchemaModel.slug("Move-Out Report"), "move_out_report")
        XCTAssertEqual(DocumentSchemaModel.slug("  RFP!!  "), "rfp")
        XCTAssertEqual(DocumentSchemaModel.slug("Punch List 2"), "punch_list_2")
    }
}
