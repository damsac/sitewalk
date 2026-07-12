import SwiftUI

// Field Instrument design tokens — source of truth is design/BRIEF.md in the
// sitewalk repo. Paper/ink base, one amber accent (the Jefe hard-hat gold),
// job-site tag colors.

extension Color {
    init(hex: UInt32) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255
        )
    }
}

enum Theme {

    // MARK: - Colors
    enum C {
        static let paper      = Color(hex: 0xFAFAF7)
        static let paperDeep  = Color(hex: 0xF1F0EA)
        static let sheet      = Color(hex: 0xFFFFFE)
        static let ink        = Color(hex: 0x141412)
        static let ink60      = Color(hex: 0x5E5C54)
        static let ink35      = Color(hex: 0xA7A49A)
        static let hairline     = Color(hex: 0x141412).opacity(0.16)
        static let hairlineSoft = Color(hex: 0x141412).opacity(0.09)

        // Amber accent (Jefe hard-hat gold). Names kept for call-site stability.
        // `orange`     — bright fills / buttons / live marks
        // `orangeDeep` — dark amber: accent TEXT & rules on paper, button shadow
        // `orangeTint` — pale gold wash behind selected chips
        // `onOrange`   — text/marks ON the amber fill; ink, since white would
        //                vanish on gold (the black-on-amber "caution label" look)
        static let orange     = Color(hex: 0xFFBB26)
        static let orangeDeep = Color(hex: 0x9A6A00)
        static let orangeTint = Color(hex: 0xFAF1D9)
        static let onOrange   = Color(hex: 0x141412)

        static let redTag     = Color(hex: 0xA63A2E)
        static let redTint    = Color(hex: 0xF7E8E5)
        static let yellowTag  = Color(hex: 0x9A7213)
        static let yellowTint = Color(hex: 0xF6EFD9)
        static let greenTag   = Color(hex: 0x3E6B35)
        static let greenTint  = Color(hex: 0xE9F0E4)
    }

    // MARK: - Spacing / metrics
    enum S {
        static let screenPad: CGFloat = 20
        static let radius: CGFloat = 14
        static let buttonHeight: CGFloat = 62   // glove-sized; never below minTarget
        static let minTarget: CGFloat = 56
    }

    // MARK: - Type (bundled statics; PostScript face names)
    enum F {
        enum UIW: String { case regular = "Regular", medium = "Medium", semibold = "SemiBold", bold = "Bold", extraBold = "ExtraBold" }
        enum CondW: String { case medium = "Medium", semibold = "SemiBold" }
        enum MonoW: String { case regular = "Regular", medium = "Medium", semibold = "SemiBold" }
        enum SerifW: String { case semibold = "SemiBold", bold = "Bold" }

        /// UI type — Barlow (highway-signage DNA)
        static func ui(_ size: CGFloat, _ w: UIW = .semibold) -> Font {
            .custom("Barlow-\(w.rawValue)", size: size)
        }
        /// Dense data rows — Barlow Semi Condensed
        static func cond(_ size: CGFloat, _ w: CondW = .semibold) -> Font {
            .custom("BarlowSemiCondensed-\(w.rawValue)", size: size)
        }
        /// Stamped metadata, prices, timestamps — IBM Plex Mono
        static func mono(_ size: CGFloat, _ w: MonoW = .regular) -> Font {
            .custom("IBMPlexMono-\(w.rawValue)", size: size)
        }
        /// Document letterhead only — Source Serif 4
        static func serif(_ size: CGFloat, _ w: SerifW = .bold) -> Font {
            .custom("SourceSerif4-\(w.rawValue)", size: size)
        }
    }
}
