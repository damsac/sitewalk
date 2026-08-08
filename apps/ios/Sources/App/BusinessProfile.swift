import Foundation

// The operator's business — the name the paperwork carries. Once a profile
// exists, the fixture letterhead ("Ridgeline Landscape Co.") disappears and
// every document goes out under THIS name.
//
// Deliberately app-side only (UserDefaults JSON), NOT in murmur-core: the
// core has no notion of an operator yet, and inventing FFI storage for one
// field-set ahead of dam's schema would be a fake seam. When the core grows
// an operator/tenant concept (so IT can stamp documents and mint doc
// numbers), this struct is the shape to migrate — see the PR thinking.
struct BusinessProfile: Codable, Equatable {

    var businessName: String
    /// One line, as the operator types it — "Denver CO". Optional-by-empty.
    var cityState: String
    var licenseNumber: String?
    /// Matches `Fixtures.all` keys: landscape | property | inspection.
    var tradeKey: String
    /// What the operator typed when they picked "Something else". Optional and
    /// decorative — the KEY is what scopes schemas and templates. Kept so a
    /// septic contractor's letterhead can say "Septic" instead of "Other".
    ///
    /// Optional with a default so decoding a profile written before this field
    /// existed still succeeds. `current` decodes with `try?`, so a
    /// non-defaulted addition would silently wipe every existing profile and
    /// re-onboard the whole beta.
    var tradeLabel: String? = nil
    /// Migration seam (dam review #190). `current` decodes with `try?`, so a
    /// future breaking schema change silently returns nil and the operator
    /// gets re-onboarded — the stored profile is gone, not just unreadable.
    /// The contract to avoid that: bump this only for changes you're willing
    /// to eat as a re-onboard, and prefer optional-only additions (like
    /// `licenseNumber` above) that decode fine against old JSON instead.
    var schemaVersion: Int = 1

    // MARK: Persistence (UserDefaults as JSON)

    private static let defaultsKey = "sitewalk.businessProfile"

    static var current: BusinessProfile? {
        guard let data = UserDefaults.standard.data(forKey: defaultsKey) else { return nil }
        return try? JSONDecoder().decode(BusinessProfile.self, from: data)
    }

    static func save(_ profile: BusinessProfile) {
        guard let data = try? JSONEncoder().encode(profile) else { return }
        UserDefaults.standard.set(data, forKey: defaultsKey)
    }

    static func clear() {
        UserDefaults.standard.removeObject(forKey: defaultsKey)
    }

    // MARK: Derived display

    /// Letterhead sub line — "DENVER CO · LIC 44-1234". Empty parts are
    /// omitted; both empty yields "" (the letterhead renders name-only).
    var letterheadSub: String {
        var parts: [String] = []
        let city = cityState.trimmingCharacters(in: .whitespaces)
        if !city.isEmpty { parts.append(city.uppercased()) }
        if let lic = licenseNumber?.trimmingCharacters(in: .whitespaces), !lic.isEmpty {
            parts.append("LIC \(lic.uppercased())")
        }
        return parts.joined(separator: " · ")
    }

    /// The trade template this operator works in.
    /// What to SHOW for this operator's trade — their own words when they gave
    /// them, the catalog label otherwise.
    var tradeDisplayName: String {
        if let custom = tradeLabel?.trimmingCharacters(in: .whitespaces), !custom.isEmpty {
            return custom
        }
        return Trades.label(for: tradeKey)
    }

    var trade: TradeFixture {
        if let exact = Fixtures.all.first(where: { $0.key == tradeKey }) { return exact }
        // A trade with no fixture content borrows landscape's demo copy — which
        // only ever renders in demo mode — but KEEPS ITS OWN KEY. Returning
        // Fixtures.landscape wholesale used to silently re-file a plumber's
        // walks and document types under "landscape".
        var generic = Fixtures.landscape
        generic.key = tradeKey
        return generic
    }
}
