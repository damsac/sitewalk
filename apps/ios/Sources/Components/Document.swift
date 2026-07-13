import SwiftUI

// The paper. The generated document is rendered as an actual sheet —
// letterhead, form ruling, stamped labels — because "that's the paperwork
// I'd have typed tonight" is the entire pitch. Serif lives here and only here.

// MARK: - Letterhead

struct Letterhead: View {
    let biz: String
    let bizSub: String
    let docKind: String
    let docNo: String
    let docDate: String
    /// The operator's branding — logo, accent, biz font, contact. `.default`
    /// reproduces the stock look, so the demo/gallery path renders unchanged.
    var branding: Branding = .default

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            if let logo = branding.logoImage {
                Image(uiImage: logo)
                    .resizable()
                    .scaledToFit()
                    .frame(width: 34, height: 34)
            }
            VStack(alignment: .leading, spacing: 4) {
                Text(biz)
                    .font(branding.bizFont(15))
                    .lineLimit(2)
                if !bizSub.isEmpty {
                    Text(bizSub)
                        .font(Theme.F.mono(7.5))
                        .tracking(0.6)
                        .foregroundStyle(Theme.C.ink60)
                }
                if !branding.contactLine.isEmpty {
                    Text(branding.contactLine)
                        .font(Theme.F.mono(7))
                        .tracking(0.4)
                        .foregroundStyle(Theme.C.ink60)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            VStack(alignment: .trailing, spacing: 3) {
                Text(docKind)
                    .font(Theme.F.mono(8.5, .semibold))
                    .tracking(1.8)
                    .foregroundStyle(branding.accentColor)
                Text(docNo)
                    .font(Theme.F.mono(11, .semibold))
                Text(docDate)
                    .font(Theme.F.mono(8))
                    .foregroundStyle(Theme.C.ink60)
            }
            .lineLimit(1)
            .fixedSize()
        }
        .padding(.bottom, 12)
        .overlay(alignment: .bottom) { Theme.C.ink.frame(height: 2) }
    }
}

// MARK: - Document line row (normal / edit / gap states)

struct DocRowView: View {
    let row: DocRowFixture

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                Text(row.title)
                    .font(Theme.F.cond(12.5, .semibold))
                Text(row.sub)
                    .font(Theme.F.mono(8, row.subWarn ? .semibold : .regular))
                    .tracking(0.3)
                    .foregroundStyle(row.subWarn ? Theme.C.yellowTag : Theme.C.ink60)
                if let hint = row.hint {
                    Text(hint)
                        .font(Theme.F.mono(8))
                        .tracking(0.3)
                        .foregroundStyle(Theme.C.orangeDeep)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            Text(row.qty)
                .font(Theme.F.mono(9.5))
                .foregroundStyle(row.isGap && row.qty == "——" ? Theme.C.yellowTag : Theme.C.ink60)

            amount
        }
        .padding(.vertical, 9)
        .overlay(alignment: .bottom) { Theme.C.hairlineSoft.frame(height: 1) }
    }

    @ViewBuilder
    private var amount: some View {
        if row.isEdit {
            HStack(spacing: 1) {
                Text(row.amount)
                    .font(Theme.F.mono(12, .semibold))
                    .foregroundStyle(Theme.C.orangeDeep)
                Caret()
            }
            .padding(.bottom, 2)
            .overlay(alignment: .bottom) { Theme.C.orangeDeep.frame(height: 2) }
        } else if row.isGap && row.amount == "——" {
            Text(row.amount)
                .font(Theme.F.mono(12, .semibold))
                .underline(true, pattern: .dash, color: Theme.C.yellowTag)
                .foregroundStyle(Theme.C.yellowTag)
        } else {
            Text(row.amount)
                .font(Theme.F.mono(12, .semibold))
                .foregroundStyle(Theme.C.ink)
        }
    }
}

// MARK: - Total row (+ gap chip when values are missing)

struct TotalRow: View {
    let key: String
    let value: String
    var gaps: Int = 0

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(key)
                .font(Theme.F.mono(9, .semibold))
                .tracking(2.0)
            Spacer()
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(value)
                    .font(Theme.F.mono(value.count > 9 ? 11.5 : 17, .semibold))
                    .tracking(value.count > 9 ? 0.7 : 0)
                if gaps > 0 { GapChip(count: gaps) }
            }
        }
        .padding(.top, 10)
        .overlay(alignment: .top) { Theme.C.ink.frame(height: 2) }
    }
}

// MARK: - Review note (yellow-tint bar)

struct RevNote: View {
    let text: String

    var body: some View {
        HStack(spacing: 0) {
            Theme.C.yellowTag.frame(width: 3)
            Text(text)
                .font(Theme.F.mono(8))
                .tracking(0.4)
                .foregroundStyle(Theme.C.ink60)
                .padding(.horizontal, 9)
                .padding(.vertical, 6)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(Theme.C.yellowTint)
        .fixedSize(horizontal: false, vertical: true)
    }
}

// MARK: - The full sheet

struct DocumentSheet: View {
    let trade: TradeFixture
    var showNote: Bool = true

    private var gapCount: Int { trade.rows.filter(\.isGap).count }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Letterhead(
                biz: trade.biz,
                bizSub: trade.bizSub,
                docKind: trade.docKind,
                docNo: trade.docNo,
                docDate: trade.docDate
            )
            ForEach(trade.rows) { row in
                DocRowView(row: row)
            }
            TotalRow(key: trade.totalKey, value: trade.totalValue, gaps: gapCount)
                .padding(.top, 2)
            if showNote {
                RevNote(text: trade.note)
                    .padding(.top, 10)
            }
        }
        .padding(18)
        .background(Theme.C.sheet)
        .compositingGroup()
        .shadow(color: Theme.C.ink.opacity(0.12), radius: 1, y: 1)
        .shadow(color: Theme.C.ink.opacity(0.18), radius: 14, y: 10)
    }
}
