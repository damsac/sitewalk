import SwiftUI

// Field vocabulary (Plan 10, write half of the vocabulary → STT biasing loop).
//
// Designed as a field tool, not a settings page: the operator teaches the mic
// the words crews actually say — plant names, part numbers, client names —
// and whisper biases toward hearing them. Hand-rolled to the Theme; the
// onboarding interview that SEEDS this list is out of scope (Plan 10 note).

struct VocabularyView: View {
    @Bindable var model: AppModel
    @State private var newTerm = ""
    @FocusState private var fieldFocused: Bool

    private let cap = 100

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // Header
            VStack(alignment: .leading, spacing: 4) {
                SectionLabel("FIELD VOCABULARY", color: Theme.C.orangeDeep)
                Text("Teach the mic your jargon")
                    .font(Theme.F.ui(22, .bold))
                Text("NAMES, PLANTS, PART NUMBERS — WALKS HEAR THEM BETTER")
                    .font(Theme.F.mono(8.5))
                    .tracking(0.8)
                    .foregroundStyle(Theme.C.ink60)
                    .padding(.top, 1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, Theme.S.screenPad)
            .padding(.top, 18)
            .padding(.bottom, 14)

            MetaStrip(
                left: "TERMS \(model.vocabulary.count) / \(cap)",
                right: model.vocabulary.count >= cap ? "FULL — REMOVE TO ADD" : "BIASES ON-DEVICE STT",
                warn: model.vocabulary.count >= cap
            )

            if let error = model.vocabularyError {
                HStack(spacing: 0) {
                    Theme.C.yellowTag.frame(width: 3)
                    Text(error.uppercased())
                        .font(Theme.F.mono(8))
                        .tracking(0.4)
                        .foregroundStyle(Theme.C.ink60)
                        .padding(.horizontal, 9)
                        .padding(.vertical, 6)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .background(Theme.C.yellowTint)
                .padding(.horizontal, Theme.S.screenPad)
                .padding(.top, 10)
            }

            // Terms
            if model.vocabulary.isEmpty {
                Text("NO TERMS YET — ADD THE WORDS CREWS ACTUALLY SAY.\n\u{201C}BOXWOOD\u{201D} · \u{201C}GFCI\u{201D} · \u{201C}HOLLIS\u{201D}")
                    .font(Theme.F.mono(8.5))
                    .tracking(0.6)
                    .foregroundStyle(Theme.C.ink35)
                    .multilineTextAlignment(.center)
                    .lineSpacing(5)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 26)
                    .overlay(
                        RoundedRectangle(cornerRadius: 4)
                            .stroke(style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
                            .foregroundStyle(Theme.C.ink35)
                    )
                    .padding(.horizontal, Theme.S.screenPad)
                    .padding(.top, 16)
            } else {
                ScrollView {
                    VStack(spacing: 0) {
                        ForEach(model.vocabulary, id: \.self) { term in
                            HStack(spacing: 10) {
                                Text(term)
                                    .font(Theme.F.mono(12, .medium))
                                Spacer()
                                Button { model.removeVocabulary(term) } label: {
                                    Image(systemName: "xmark")
                                        .font(.system(size: 10, weight: .bold))
                                        .foregroundStyle(Theme.C.ink60)
                                        .frame(width: 30, height: 30)
                                        .overlay(
                                            RoundedRectangle(cornerRadius: 6)
                                                .stroke(Theme.C.hairline, lineWidth: 1)
                                        )
                                }
                                .buttonStyle(.plain)
                            }
                            .padding(.horizontal, Theme.S.screenPad)
                            .padding(.vertical, 9)
                            .overlay(alignment: .bottom) { Theme.C.hairlineSoft.frame(height: 1) }
                        }
                    }
                }
            }

            Spacer(minLength: 0)

            // Add bar
            VStack(spacing: 10) {
                HStack(spacing: 10) {
                    TextField("boxwood, zone 2, Hollis…", text: $newTerm)
                        .font(Theme.F.mono(14, .medium))
                        .autocorrectionDisabled()
                        .textInputAutocapitalization(.never)
                        .focused($fieldFocused)
                        .padding(.bottom, 6)
                        .overlay(alignment: .bottom) { Theme.C.orange.frame(height: 2) }
                    Button {
                        let term = newTerm.trimmingCharacters(in: .whitespaces)
                        newTerm = ""
                        if !term.isEmpty { model.addVocabulary(term) }
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
            }
            .padding(.horizontal, Theme.S.screenPad)
            .padding(.top, 12)
            .padding(.bottom, 12)
            .overlay(alignment: .top) { Theme.C.hairline.frame(height: 1) }
        }
        .background(Theme.C.paper.ignoresSafeArea())
        .onAppear { model.loadVocabulary() }
    }
}
