import XCTest
@testable import SitewalkGallery

// Plan 15 D7-15: the schema gate for every bundled vocab pack (the 2026-07-10
// ruling's "CI schema test"). This gate is what makes `Empty`/`TooLong`
// unreachable inside the Rust `seed_vocabulary` funnel — a shipped pack can
// never contain a blank or a sentence. // sac: pack content is yours; keep
// every pack green here.
//
// Drift note: the `60` and `6` below are Swift literals with NO compile-time
// cross-pin to the Rust constants (different toolchains) — accepted drift,
// pinned by paired comments on both sides.
final class VocabPackTests: XCTestCase {

    /// One pack per trade key (mirrors `BusinessProfile.tradeKey`).
    private let tradeKeys = ["landscape", "property", "inspection"]

    /// Per-pass seed budget. // keep in sync with ffi SEED_MAX (60)
    private let seedMax = 60
    /// Max whitespace words per term. // keep in sync with harness MAX_VOCABULARY_TERM_WORDS (6)
    private let maxTermWords = 6

    private func bundledPack(for key: String) throws -> VocabPack {
        // Resolve the APP bundle via a class that lives in the app module —
        // robust whether or not the test runs hosted (Bundle.main can be the
        // xctest runner in an unhosted configuration).
        let appBundle = Bundle(for: AppModel.self)
        let pack = VocabPack.bundled(for: key, in: appBundle) ?? VocabPack.bundled(for: key)
        return try XCTUnwrap(pack, "pack for '\(key)' must be bundled and decodable")
    }

    func testEveryBundledPackPassesTheSchemaGate() throws {
        for key in tradeKeys {
            let pack = try bundledPack(for: key)

            XCTAssertEqual(pack.trade, key, "\(key): pack trade must match its file name")
            XCTAssertGreaterThanOrEqual(pack.version, 1, "\(key): version >= 1")

            XCTAssertFalse(pack.terms.isEmpty, "\(key): terms must be non-empty")
            XCTAssertLessThanOrEqual(
                pack.terms.count, seedMax,
                "\(key): a pack must fit one seeding pass (SEED_MAX)"
            )

            // No case-insensitive duplicates (the funnel would dedup them, but
            // a duplicate chip is a curation bug).
            let lowered = pack.terms.map { $0.lowercased() }
            XCTAssertEqual(
                Set(lowered).count, lowered.count,
                "\(key): no case-insensitive duplicate terms"
            )

            for term in pack.terms {
                let words = term.split(whereSeparator: \.isWhitespace)
                XCTAssertFalse(words.isEmpty, "\(key): term '\(term)' must not be blank")
                XCTAssertLessThanOrEqual(
                    words.count, maxTermWords,
                    "\(key): term '\(term)' must be a term, not a sentence (1–\(maxTermWords) words)"
                )
            }
        }
    }
}
