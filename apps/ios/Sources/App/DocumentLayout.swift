import Foundation

// The operator's document STRUCTURE basics (app-side, v1): static sections that
// render on every document — terms / payment boilerplate and a client signature
// line. This is the "B-basics" dam greenlit app-side (design doc §8, PR #207)
// ahead of the core `DocumentSchema` seam (§7.2), which will eventually own the
// richer, LLM-filled structure (custom fields / new doc types).
//
// Kept SEPARATE from Branding on purpose: Branding is STYLE (how it looks),
// this is STRUCTURE (what's in it). Same app-side UserDefaults-JSON seam as
// Branding / BusinessProfile; migrates into the core schema when that lands.
struct DocumentLayout: Codable, Equatable {

    /// Operator-authored terms / payment text — rendered as a TERMS block on the
    /// document when non-empty (deposit, validity window, etc.). Static: never
    /// LLM-filled, so no core involvement.
    var termsText: String = ""
    /// Show a "client signature · date" line at the foot of the document.
    var showSignature: Bool = false
    /// Migration seam (see Branding): `current` decodes with `try?`, so a
    /// breaking bump silently resets to `.default` rather than crashing.
    var schemaVersion: Int = 1

    static let `default` = DocumentLayout()

    // MARK: - Persistence (UserDefaults JSON)

    private static let defaultsKey = "sitewalk.documentLayout"

    static var current: DocumentLayout {
        guard let data = UserDefaults.standard.data(forKey: defaultsKey),
              let layout = try? JSONDecoder().decode(DocumentLayout.self, from: data)
        else { return .default }
        return layout
    }

    static func save(_ layout: DocumentLayout) {
        guard let data = try? JSONEncoder().encode(layout) else { return }
        UserDefaults.standard.set(data, forKey: defaultsKey)
    }

    /// True when nothing extra renders — lets callers skip the section entirely.
    var isEmpty: Bool {
        termsText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !showSignature
    }
}
