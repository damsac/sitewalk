import SwiftUI

// MARK: - Field tag (job-site tag language: red / yellow / green / plain)

struct FieldTag: View {
    let tag: TagFixture

    private var color: Color {
        switch tag.kind {
        case .red: return Theme.C.redTag
        case .yellow: return Theme.C.yellowTag
        case .green: return Theme.C.greenTag
        case .plain: return Theme.C.ink60
        }
    }
    private var tint: Color {
        switch tag.kind {
        case .red: return Theme.C.redTint
        case .yellow: return Theme.C.yellowTint
        case .green: return Theme.C.greenTint
        case .plain: return Theme.C.paperDeep
        }
    }

    var body: some View {
        Text(tag.label)
            .font(Theme.F.mono(8, .semibold))
            .tracking(1.0)
            .foregroundStyle(color)
            .padding(.horizontal, 6)
            .padding(.top, 3)
            .padding(.bottom, 2)
            .background(tint)
    }
}

// MARK: - Stamped section label

struct SectionLabel: View {
    let text: String
    var color: Color = Theme.C.ink60

    init(_ text: String, color: Color = Theme.C.ink60) {
        self.text = text
        self.color = color
    }

    var body: some View {
        Text(text.uppercased())
            .font(Theme.F.mono(9, .semibold))
            .tracking(2.0)
            .foregroundStyle(color)
    }
}

// MARK: - Empty panel (one idiom for "nothing here yet")

/// The app's ONE way of saying a list is empty.
///
/// It had three. The board's walks list used a filled `paperDeep` slab, the
/// jobs list directly beneath it used a white card with a hairline, and the
/// document's photo well used a dashed outline — three answers to the same
/// question, two of them stacked in the same scroll (Isaac's board shot,
/// 2026-08-09: "the grey backdrop looks off when there are no walks"). Grey
/// was the wrong one to keep: on a board whose section beds are also grey it
/// stops reading as a card and starts reading as a hole in the layout.
///
/// A quiet white card with a hairline is what "nothing here yet" looks like
/// in this app — the same sheet-on-paper grammar every real row uses, so an
/// empty list reads as a list that is empty rather than as a different kind
/// of surface. Dashed is deliberately NOT it: in app language dashed means
/// disabled, and these lists are the opposite of disabled — they are waiting
/// for the operator's first tap.
///
/// One documented exception: the photo well inside the rendered document
/// (`ReviewView.photoGallery`) keeps its dashed outline, because this panel is
/// sheet-white and the document it would sit on is too — an invisible card is
/// not consistency. Dashed there also means what it says: a space waiting to
/// be filled, on a page rather than in a list.
struct EmptyPanel: View {
    let text: String

    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text)
            .font(Theme.F.ui(14, .medium))
            .foregroundStyle(Theme.C.ink60)
            .multilineTextAlignment(.center)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 18)
            .padding(.vertical, 22)
            .background(Theme.C.sheet)
            .clipShape(RoundedRectangle(cornerRadius: Theme.S.radiusCard))
            .overlay(
                RoundedRectangle(cornerRadius: Theme.S.radiusCard)
                    .stroke(Theme.C.hairline, lineWidth: 1)
            )
    }
}

// MARK: - Metadata strip (provenance: site, sync, signal state)

struct MetaStrip: View {
    let left: String
    let right: String
    var warn: Bool = false

    var body: some View {
        HStack {
            Text(left)
            Spacer(minLength: 12)
            Text(right)
                .fontWeight(warn ? .semibold : .regular)
                .foregroundStyle(warn ? Theme.C.yellowTag : Theme.C.ink60)
        }
        .font(Theme.F.mono(8.5))
        // Tracking eased from 0.8 as the type grew. Heavy tracking is a print
        // device that helps at 8.5pt and actively hurts once the glyphs are big
        // enough to hold their own shape.
        .tracking(0.4)
        .foregroundStyle(Theme.C.ink60)
        .lineLimit(1)
        // Dynamic Type is now honoured everywhere (Theme.F `relativeTo:`), but
        // this strip is two labels sharing one line with `lineLimit(1)` — at
        // accessibility sizes one of them would simply truncate away. Capping
        // growth keeps both readable; the review calls this out as the pattern
        // for any strip that must stay on one line.
        .dynamicTypeSize(...DynamicTypeSize.accessibility1)
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.vertical, 7)
        .overlay(alignment: .top) { Theme.C.hairline.frame(height: 1) }
        .overlay(alignment: .bottom) { Theme.C.hairline.frame(height: 1) }
    }
}

// MARK: - Photo chip (count of photos pinned to an item)

struct PhotoChip: View {
    let count: Int

    var body: some View {
        HStack(spacing: 3) {
            Image(systemName: "camera")
                .font(.system(size: 8, weight: .semibold))
            Text("×\(count)")
                .font(Theme.F.mono(8, .semibold))
        }
        .foregroundStyle(Theme.C.ink60)
        .padding(.horizontal, 5)
        .padding(.top, 3)
        .padding(.bottom, 2)
        .background(Theme.C.paperDeep)
    }
}

// MARK: - Blinking caret

struct Caret: View {
    var color: Color = Theme.C.orange
    var width: CGFloat = 2
    var height: CGFloat = 11
    @State private var on = true

    var body: some View {
        Rectangle()
            .fill(color)
            .frame(width: width, height: height)
            .opacity(on ? 1 : 0.15)
            .animation(.easeInOut(duration: 0.5).repeatForever(), value: on)
            .onAppear { on = false }
    }
}

// MARK: - Gap chip (+N GAP next to a total)

struct GapChip: View {
    let count: Int

    var body: some View {
        Text("+\(count) GAP")
            .font(Theme.F.mono(8.5, .semibold))
            .tracking(0.9)
            .foregroundStyle(Theme.C.yellowTag)
            .padding(.horizontal, 6)
            .padding(.top, 3)
            .padding(.bottom, 2)
            .background(Theme.C.yellowTint)
    }
}
