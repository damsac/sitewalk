import SwiftUI

// Plan 15 D9-15: the onboarding vocabulary card — seed the mic's vocabulary
// from the trade pack AFTER the user's first walk (they now have context for
// "what did it get wrong?"). Chips are DEFAULT-ON, tap to deselect — never a
// silent write (R6: only confirmed terms reach the engine). DONE seeds the
// confirmed chips in one idempotent batch + adds free-form terms one at a
// time (per-term catch-and-continue in AppModel.applyVocabSeed); SKIP writes
// nothing (cold start = today's behavior exactly).
//
// // sac: this is minimal REFERENCE wiring — card visuals, chip layout, copy,
// and exact placement in the flow are yours. The data path underneath
// (seedVocabulary + SeedReport.terms refresh) is the contract to keep.
struct VocabSeedCard: View {
    @Bindable var model: AppModel
    let pack: VocabPack
    /// Called when the card is finished (DONE or SKIP) — the presenter dismisses.
    var onDismiss: () -> Void

    @State private var deselected: Set<String> = []
    @State private var freeform: [String] = []
    @State private var newTerm = ""
    @State private var confirmation: String?

    private var confirmedChips: [String] { pack.terms.filter { !deselected.contains($0) } }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SectionLabel("TEACH THE MIC", color: Theme.C.orangeDeep)
                .padding(.top, 18)
            Text("Words your trade actually says")
                .font(Theme.F.ui(22, .bold))
                .padding(.top, 6)
            Text("TAP OFF ANY YOU DON’T USE — WALKS HEAR THE REST BETTER")
                .font(Theme.F.mono(8.5))
                .tracking(0.8)
                .foregroundStyle(Theme.C.ink60)
                .padding(.top, 4)

            ScrollView {
                // sac: chip layout — a simple wrap for the reference wiring.
                WrapChips(terms: pack.terms, deselected: $deselected)
                    .padding(.top, 16)

                // Free-form additions (the VocabularyView add-bar idiom).
                HStack(spacing: 10) {
                    TextField("boxwood, zone 2, Hollis…", text: $newTerm)
                        .font(Theme.F.mono(14, .medium))
                        .autocorrectionDisabled()
                        .textInputAutocapitalization(.never)
                        .padding(.bottom, 6)
                        .overlay(alignment: .bottom) { Theme.C.orangeDeep.frame(height: 2) }
                    Button {
                        let term = newTerm.trimmingCharacters(in: .whitespaces)
                        newTerm = ""
                        if !term.isEmpty { freeform.append(term) }
                    } label: {
                        Text("ADD")
                            .font(Theme.F.ui(14, .bold))
                            .tracking(1.4)
                            .foregroundStyle(Theme.C.paper)
                            .frame(width: 76, height: 48)
                            .background(RoundedRectangle(cornerRadius: Theme.S.radius).fill(Theme.C.ink))
                    }
                    .buttonStyle(.plain)
                }
                .padding(.top, 18)

                if !freeform.isEmpty {
                    Text(freeform.joined(separator: " · "))
                        .font(Theme.F.mono(10, .medium))
                        .foregroundStyle(Theme.C.ink60)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.top, 8)
                }

                if let confirmation {
                    Text(confirmation.uppercased())
                        .font(Theme.F.mono(9, .semibold))
                        .tracking(1.0)
                        .foregroundStyle(Theme.C.greenTag)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.top, 10)
                }
            }

            Spacer(minLength: 0)

            // DONE seeds; SKIP writes nothing. Core stays authoritative for
            // idempotency — a re-shown card re-seeding the same pack no-ops.
            HStack(spacing: 10) {
                Button {
                    onDismiss()
                } label: {
                    Text("SKIP")
                        .font(Theme.F.ui(14, .bold))
                        .tracking(1.4)
                        .foregroundStyle(Theme.C.ink60)
                        .frame(maxWidth: .infinity)
                        .frame(height: Theme.S.buttonHeight)
                        .overlay(RoundedRectangle(cornerRadius: Theme.S.radius).stroke(Theme.C.hairline, lineWidth: 1))
                }
                .buttonStyle(.plain)
                Button {
                    confirmation = model.applyVocabSeed(
                        pack: pack, confirmedChips: confirmedChips, freeform: freeform
                    )
                    onDismiss()
                } label: {
                    Text("DONE")
                        .font(Theme.F.ui(14, .bold))
                        .tracking(1.4)
                        .foregroundStyle(Theme.C.onOrange)
                        .frame(maxWidth: .infinity)
                        .frame(height: Theme.S.buttonHeight)
                        .background(RoundedRectangle(cornerRadius: Theme.S.radius).fill(Theme.C.orange))
                }
                .buttonStyle(.plain)
            }
            .padding(.top, 12)
            .padding(.bottom, 12)
        }
        .padding(.horizontal, Theme.S.screenPad)
        .background(Theme.C.paper.ignoresSafeArea())
    }
}

/// Default-on deselectable term chips. // sac: visual grammar is yours — this
/// is the plainest functional wrap (leading-aligned rows).
private struct WrapChips: View {
    let terms: [String]
    @Binding var deselected: Set<String>

    var body: some View {
        FlowLayout(spacing: 8) {
            ForEach(terms, id: \.self) { term in
                let on = !deselected.contains(term)
                Button {
                    if on { deselected.insert(term) } else { deselected.remove(term) }
                } label: {
                    Text(term)
                        .font(Theme.F.mono(11, .semibold))
                        .foregroundStyle(on ? Theme.C.ink : Theme.C.ink35)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 8)
                        .background(on ? Theme.C.orangeTint : Theme.C.paperDeep)
                        .overlay(Rectangle().stroke(on ? Theme.C.orange : Theme.C.hairline, lineWidth: on ? 1.5 : 1))
                }
                .buttonStyle(.plain)
            }
        }
    }
}

/// Minimal wrapping layout for the chips (iOS 16+ `Layout`).
private struct FlowLayout: Layout {
    var spacing: CGFloat = 8

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? .infinity
        var x: CGFloat = 0, y: CGFloat = 0, rowHeight: CGFloat = 0
        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x > 0, x + size.width > width {
                x = 0
                y += rowHeight + spacing
                rowHeight = 0
            }
            x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
        }
        return CGSize(width: width == .infinity ? x : width, height: y + rowHeight)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var x = bounds.minX
        var y = bounds.minY
        var rowHeight: CGFloat = 0
        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x > bounds.minX, x + size.width > bounds.maxX {
                x = bounds.minX
                y += rowHeight + spacing
                rowHeight = 0
            }
            subview.place(at: CGPoint(x: x, y: y), proposal: .unspecified)
            x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
        }
    }
}
