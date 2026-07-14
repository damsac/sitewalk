import Foundation

// Plan 15 D7-15: per-trade starter vocabulary packs, bundled on-device as
// static JSON (privacy invariant: nothing but LLM calls leaves the device).
// Core owns NO pack content — the card sends only the USER-CONFIRMED subset
// through `seedVocabulary(trade:version:terms:)`.
//
// // sac: the pack JSON schema + term curation are yours. Schema contract
// (gated by VocabPackTests): `version >= 1`, `terms` non-empty, `<= 60`
// terms (keep in sync with ffi SEED_MAX), no case-insensitive duplicates,
// each term 1–6 whitespace words. A content revision ships as a `version`
// bump (a deliberate re-seed); same-version edits will NOT re-apply on
// devices that already seeded.
struct VocabPack: Codable, Equatable {
    /// Matches `BusinessProfile.tradeKey`: landscape | property | inspection.
    let trade: String
    let version: UInt32
    let terms: [String]

    /// Decode the bundled pack for a trade key. `nil` when no pack ships for
    /// the key (the card simply doesn't show) or the JSON fails to decode
    /// (VocabPackTests makes that unreachable for shipped packs).
    static func bundled(for tradeKey: String, in bundle: Bundle = .main) -> VocabPack? {
        // xcodegen folder-groups flatten resources into the bundle root; try
        // the subdirectory first for folder-reference setups, then flat.
        let url = bundle.url(forResource: tradeKey, withExtension: "json", subdirectory: "VocabPacks")
            ?? bundle.url(forResource: tradeKey, withExtension: "json")
        guard let url, let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder().decode(VocabPack.self, from: data)
    }
}
