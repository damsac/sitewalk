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
    /// In-memory logo for pre-commit preview (Letterhead Studio) — wins over
    /// the branding's committed file when set.
    var logoOverride: UIImage?

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            if let logo = logoOverride ?? branding.logoImage {
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
                // The second line: what the line covers, what the crew must
                // do, or what was observed. It sets as SENTENCES, not as a
                // stamp — a directive like "strip the old bark first, watch
                // the irrigation heads" is read, not scanned, and 8pt mono
                // all-caps is unreadable at two paragraphs.
                if !row.sub.isEmpty {
                    Text(row.sub)
                        .font(row.subWarn ? Theme.F.mono(8, .semibold) : Theme.F.ui(11, .medium))
                        .tracking(row.subWarn ? 0.3 : 0)
                        .foregroundStyle(row.subWarn ? Theme.C.yellowTag : Theme.C.ink60)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let hint = row.hint {
                    Text(hint)
                        .font(Theme.F.mono(8))
                        .tracking(0.3)
                        .foregroundStyle(Theme.C.amberInk)
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
        // A work order carries no money, so the column the price would
        // occupy is where the crew's name goes — the exact spot a foreman's
        // eye already travels to on every other document they read.
        if let assignee = row.assignee, !assignee.isEmpty {
            Text(assignee.uppercased())
                .font(Theme.F.mono(9, .semibold))
                .tracking(0.8)
                .foregroundStyle(Theme.C.amberInk)
                .padding(.horizontal, 6)
                .padding(.top, 3)
                .padding(.bottom, 2)
                .background(Theme.C.orangeTint)
                .lineLimit(1)
        } else if row.isEdit {
            HStack(spacing: 1) {
                Text(row.amount)
                    .font(Theme.F.mono(12, .semibold))
                    .foregroundStyle(Theme.C.amberInk)
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

// MARK: - Authored section (the prose blocks a trade document is expected to carry)

/// One authored block — ASSIGNMENT, SITE NOTES, SCOPE OF WORK, SUMMARY.
///
/// This is what makes each document type recognizable as itself rather than
/// as the same list under a different heading. A crew reads ASSIGNMENT and
/// SITE NOTES before they read a single line item; a client reads SCOPE OF
/// WORK and may never read the lines at all.
///
/// Gaps show rather than hide. An operator scanning before sending needs to
/// see that nothing was said about safety, in the place the safety note would
/// have been — a silently absent field is indistinguishable from a field that
/// was never on the document, and the whole point is that they can fill it.
struct DocSectionView: View {
    let section: DocSectionFixture
    /// Tapping a field opens it for editing, like a line.
    var onEdit: ((DocFieldFixture) -> Void)?

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            Text(section.label)
                .font(Theme.F.mono(8, .semibold))
                .tracking(1.6)
                .foregroundStyle(Theme.C.ink60)
                .padding(.bottom, 1)
                .overlay(alignment: .bottom) { Theme.C.hairline.frame(height: 1) }

            ForEach(section.fields) { field in
                fieldView(field)
                    .contentShape(Rectangle())
                    .onTapGesture { onEdit?(field) }
                    .accessibilityElement(children: .combine)
                    .accessibilityHint("Edit \(field.label)")
            }
        }
        .padding(.top, 14)
    }

    /// A section holding ONE paragraph is already named by its own heading —
    /// SCOPE OF WORK over a field labelled "Scope of work" is the heading
    /// said twice. Sections with more than one field keep their labels,
    /// because there the labels are what tell Access apart from Safety.
    private var labelsAreRedundant: Bool {
        section.fields.count == 1 && section.fields[0].isParagraph
    }

    @ViewBuilder
    private func fieldView(_ field: DocFieldFixture) -> some View {
        if field.isParagraph {
            VStack(alignment: .leading, spacing: 3) {
                if !labelsAreRedundant {
                    Text(field.label.uppercased())
                        .font(Theme.F.mono(7.5, .semibold))
                        .tracking(1.2)
                        .foregroundStyle(Theme.C.ink45)
                }
                valueText(field)
            }
        } else {
            // Short fields sit inline — "ASSIGNED TO   Jose, Michael" reads
            // as a form field, which is exactly what it is.
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Text(field.label.uppercased())
                    .font(Theme.F.mono(7.5, .semibold))
                    .tracking(1.2)
                    .foregroundStyle(Theme.C.ink45)
                    // Wide enough for "ASSIGNED TO" on one line — a wrapped
                    // two-word label reads as a layout accident.
                    .frame(width: 104, alignment: .leading)
                valueText(field)
            }
        }
    }

    @ViewBuilder
    private func valueText(_ field: DocFieldFixture) -> some View {
        if let value = field.value, !value.isEmpty {
            Text(value)
                .font(Theme.F.ui(12, .medium))
                .foregroundStyle(Theme.C.ink)
                .lineSpacing(2)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        } else if field.isOptional {
            // An INVITATION, not a warning. Nothing was expected to fill this
            // but the operator, so blank is a perfectly good final state and
            // the yellow gap treatment would be nagging about a defect that
            // does not exist. Amber, the app's "you can act here" colour.
            Text("TAP TO ADD")
                .font(Theme.F.mono(8, .semibold))
                .tracking(1.0)
                .foregroundStyle(Theme.C.amberInk)
                .frame(maxWidth: .infinity, alignment: .leading)
        } else {
            Text(OperatorNote.gap)
                .font(Theme.F.mono(8, .semibold))
                .tracking(0.3)
                .foregroundStyle(Theme.C.yellowTag)
                .frame(maxWidth: .infinity, alignment: .leading)
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

// MARK: - Document structure basics (DocumentLayout — app-side, Terms + Signature)

/// Operator-authored terms / payment boilerplate, rendered as a labelled block.
struct TermsBlock: View {
    let text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("TERMS")
                .font(Theme.F.mono(7.5, .semibold)).tracking(1.8)
                .foregroundStyle(Theme.C.ink60)
            Text(text)
                .font(Theme.F.mono(8.5))
                .tracking(0.2)
                .foregroundStyle(Theme.C.ink)
                .lineSpacing(3)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.top, 12)
    }
}

/// Client acceptance — a signature line + a narrower date line.
struct SignatureRow: View {
    var body: some View {
        HStack(alignment: .bottom, spacing: 28) {
            sigLine("CLIENT SIGNATURE")
            sigLine("DATE").frame(width: 110)
        }
        .padding(.top, 20)
    }

    private func sigLine(_ label: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Theme.C.ink.frame(height: 1.5).padding(.top, 16)
            Text(label)
                .font(Theme.F.mono(7, .semibold)).tracking(1.0)
                .foregroundStyle(Theme.C.ink60)
        }
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
