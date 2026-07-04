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
        .tracking(0.8)
        .foregroundStyle(Theme.C.ink60)
        .lineLimit(1)
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
