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
        /// Decoration ONLY — hairline strokes, dashes, disabled fills.
        ///
        /// 2.39:1 on paper, which fails every WCAG threshold. That is fine for a
        /// rule or a border, and was not fine for the 23 places it was being
        /// used as TEXT. Disabled-control text is the one exemption (WCAG 1.4.3
        /// excludes it), which is why `RaisedBlockStyle` still uses it there.
        static let ink35      = Color(hex: 0xA7A49A)
        /// Quiet TEXT — 5.1:1 on paper, 4.7:1 on paperDeep, so it clears AA on
        /// both grounds the app actually uses. The level between `ink60` and
        /// decoration: still visibly secondary, still legible in sun.
        static let ink45      = Color(hex: 0x6E6B62)
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
        /// Amber for SMALL TEXT — 7.8:1 on paper, so it clears AAA.
        ///
        /// `orangeDeep` is 4.53:1: AA for large text only, short of the AAA the
        /// brief claims, and it was being used at 8.5pt. Measured, not
        /// eyeballed. Use `orangeDeep` for fills and lips (where contrast
        /// against paper is irrelevant) and this for anything set as type.
        static let amberInk = Color(hex: 0x6B4900)
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
        // THREE radii, not five. 2, 3, 4, 5 and 14 were all in use, which
        // reads as accident rather than system (design review P1 #7). Named so
        // no call site invents a sixth.
        //
        /// Stamps and tags — deliberately square, and on-idiom for a work order.
        static let radiusStamp: CGFloat = 0
        /// Chips, cards and fields.
        static let radiusCard: CGFloat = 4
        /// Buttons and sheets.
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

        /// The type rescale (design review 2026-07-29, P0 #1 + #2).
        ///
        /// `mono` was being called at 7, 7.5, 8, 8.5, 9, 9.5 and 11pt. iOS body
        /// is 17. A carbon-copy work order can set 8pt because paper has
        /// infinite resolution and gets held at 14 inches in good light; a phone
        /// at hip height in noon sun does not. The brief demands "glanceable at
        /// arm's length" and the ramp quietly contradicted it.
        ///
        /// Who is holding the phone settles it: the median US construction
        /// worker is 42, one in five is 55 or older, and the median trade
        /// supervisor — the person paying $19.99/mo — is 46 and needs reading
        /// glasses for 8pt mono. Type size is not a polish item for this
        /// audience; it is the product working or not working.
        ///
        /// Applied HERE rather than at ~100 call sites, which is what makes the
        /// ratios between levels survive exactly: one curve, no drift, and no
        /// call site can opt out or invent its own. The forms and the ruling
        /// carry the aesthetic, not the smallness — it survives at 1.3×.
        ///
        /// Growth tapers with size: small stamped type needs the most help,
        /// headlines are already legible and would just eat the layout.
        /// The multiplier tapers CONTINUOUSLY from 1.30 at 12pt to 1.08 at 20pt.
        ///
        /// It has to be continuous, not stepped. A first attempt used flat
        /// bands (`<12 → ×1.30`, `<20 → ×1.20`, `else ×1.08`) and the ramp
        /// INVERTED at every boundary: 11.5pt resolved to 14.95 while 12pt
        /// resolved to 14.4, so a 12pt call site rendered smaller than an 11.5pt
        /// one. On screen that is a subtitle set larger than the title above it.
        /// `TypeRampTests.testTheCurveIsMonotonic` caught it and now guards it.
        private static func scaled(_ size: CGFloat) -> CGFloat {
            // 11pt floor — below this nothing is reliably legible in sun, so
            // there is no smaller size worth preserving.
            let floor: CGFloat = 11
            let small: CGFloat = 1.30   // stamped type needs the most help
            let large: CGFloat = 1.08   // headlines are already legible
            let lower: CGFloat = 12
            let upper: CGFloat = 20

            if size <= lower { return max(floor, size * small) }
            if size >= upper { return size * large }
            // Linear taper between the anchors. Monotonic across the whole
            // range: the derivative stays positive up to 20pt, and both ends
            // meet the neighbouring branches exactly (12 → 15.6, 20 → 21.6).
            let t = (size - lower) / (upper - lower)
            return size * (small + t * (large - small))
        }

        /// What `scaled` returns — for tests, and for layout that must reserve
        /// space for a glyph run.
        static func resolvedSize(_ size: CGFloat) -> CGFloat { scaled(size) }

        // `relativeTo:` is what adopts Dynamic Type (P0 #2). One in five
        // operators is 55+ and many have already set a larger system size;
        // until now every font was a fixed `.custom(size:)` and the app ignored
        // that setting completely. Strips that must stay on one line cap growth
        // with `.dynamicTypeSize(...)` at their own call site rather than here.

        /// UI type — Barlow (highway-signage DNA)
        static func ui(_ size: CGFloat, _ w: UIW = .semibold) -> Font {
            .custom("Barlow-\(w.rawValue)", size: scaled(size), relativeTo: .body)
        }
        /// Dense data rows — Barlow Semi Condensed
        static func cond(_ size: CGFloat, _ w: CondW = .semibold) -> Font {
            .custom("BarlowSemiCondensed-\(w.rawValue)", size: scaled(size), relativeTo: .body)
        }
        /// Stamped metadata, prices, timestamps — IBM Plex Mono
        static func mono(_ size: CGFloat, _ w: MonoW = .regular) -> Font {
            .custom("IBMPlexMono-\(w.rawValue)", size: scaled(size), relativeTo: .body)
        }
        /// Document letterhead only — Source Serif 4
        static func serif(_ size: CGFloat, _ w: SerifW = .bold) -> Font {
            .custom("SourceSerif4-\(w.rawValue)", size: scaled(size), relativeTo: .body)
        }
    }
}
