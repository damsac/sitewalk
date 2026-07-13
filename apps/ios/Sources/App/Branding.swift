import SwiftUI
import UIKit

// The operator's document branding — logo, brand color, letterhead font, contact
// lines, and the free-tier footer. Makes the exported paperwork *theirs*.
//
// App-side only (UserDefaults JSON), same seam as BusinessProfile: the logo bytes
// live in the app container referenced by filename, the rest is a small Codable
// record. Threads into Letterhead + DocumentPDF; `.default` reproduces the stock
// Field Instrument look so the demo/gallery path renders unchanged.
//
// Scope: this is the STYLE half (design doc §5, PR #207). STRUCTURE (sections /
// custom fields / uploaded docs) is a separate, LLM-touching effort pending dam.
struct Branding: Codable, Equatable {

    /// Letterhead layout preset. v1 renders "field"; the picker persists the
    /// choice so "modern"/"minimal" slot in without a data migration.
    var presetKey: String = "field"
    /// Logo asset filename in the app container (nil = no logo, name-only head).
    var logoFilename: String?
    /// Brand accent — the doc-kind label + rules. Default = the stock dark amber.
    var accentHex: UInt32 = 0x9A6A00
    /// Business-name face. Curated + bundled-only for v1 (serif | sans); adding
    /// more is a TTF-drop + one `case` in `bizFont`.
    var fontKey: String = "serif"
    var phone: String = ""
    var email: String = ""
    var website: String = ""
    /// The free-tier "PREPARED WITH JEFE" footer. Turning it off is Pro.
    var showWatermark: Bool = true
    /// Migration seam (see BusinessProfile): `current` decodes with `try?`, so a
    /// breaking bump silently resets to `.default` instead of crashing. Bump only
    /// for changes you'll eat as a reset; prefer optional-only additions.
    var schemaVersion: Int = 1

    static let `default` = Branding()

    // MARK: - Persistence (UserDefaults JSON)

    private static let defaultsKey = "sitewalk.branding"

    static var current: Branding {
        guard let data = UserDefaults.standard.data(forKey: defaultsKey),
              let branding = try? JSONDecoder().decode(Branding.self, from: data)
        else { return .default }
        return branding
    }

    static func save(_ branding: Branding) {
        guard let data = try? JSONEncoder().encode(branding) else { return }
        UserDefaults.standard.set(data, forKey: defaultsKey)
    }

    // MARK: - Logo asset (app container)

    private static var logoDir: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    }

    /// Persist new logo bytes; returns the filename to store on `logoFilename`.
    /// A fresh UUID name means the SwiftUI `Image` cache never serves a stale
    /// logo after a replace (same path would).
    static func saveLogo(_ data: Data) -> String? {
        let name = "letterhead-logo-\(UUID().uuidString).png"
        let dir = logoDir
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        do {
            try data.write(to: dir.appendingPathComponent(name))
            return name
        } catch {
            return nil
        }
    }

    static func logoURL(_ filename: String) -> URL { logoDir.appendingPathComponent(filename) }

    // MARK: - Derived render values

    var accentColor: Color { Color(hex: accentHex) }

    var logoImage: UIImage? {
        guard let filename = logoFilename else { return nil }
        return UIImage(contentsOfFile: Branding.logoURL(filename).path)
    }

    /// Business-name font for the letterhead. Curated + bundled-only (v1):
    /// serif = Source Serif, sans = Barlow.
    func bizFont(_ size: CGFloat) -> Font {
        switch fontKey {
        case "sans": return Theme.F.ui(size, .bold)
        default:     return Theme.F.serif(size, .bold)
        }
    }

    /// Contact sub-line — "PHONE · EMAIL · WEB", empty parts omitted, "" when none.
    var contactLine: String {
        [phone, email, website]
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
            .joined(separator: "  ·  ")
    }

    /// Footer stamp, or nil when the operator has removed it (Pro).
    var footerText: String? { showWatermark ? "PREPARED WITH JEFE" : nil }
}
