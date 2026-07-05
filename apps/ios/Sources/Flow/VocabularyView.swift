import SwiftUI

// Vocabulary editor (Plan 10, write half of the vocabulary → STT biasing loop).
//
// sac: This whole screen is a functional placeholder — visual design is yours.
// A bare List is used only because the app has none yet; restyle to the Theme
// (Theme.C / Theme.F / Theme.S) or hand-roll rows like the rest of the app.
// The onboarding interview (which SEEDS this vocabulary) is out of scope here.
struct VocabularyView: View {
    @Bindable var model: AppModel
    @State private var newTerm = ""

    var body: some View {
        List {
            Section {                                    // sac: header/empty-state design
                ForEach(model.vocabulary, id: \.self) { term in
                    Text(term)
                }
                .onDelete { idx in
                    idx.map { model.vocabulary[$0] }.forEach(model.removeVocabulary)
                }
            }

            Section {                                    // sac: add affordance design (this is placeholder)
                HStack {
                    TextField("Add a term", text: $newTerm) // sac: styling / focus / placeholder copy
                    Button("Add") {
                        let term = newTerm
                        newTerm = ""
                        if !term.trimmingCharacters(in: .whitespaces).isEmpty {
                            model.addVocabulary(term)
                        }
                    }
                }
                // sac: the thrown-error surface (full-at-100, empty, too-long) is
                // yours to design; this is a bare placeholder.
                if let error = model.vocabularyError {
                    Text(error)
                        .font(.footnote)
                        .foregroundStyle(.red)
                }
            }
        }
        .onAppear { model.loadVocabulary() }
        // sac: title, chrome, navigation style are yours.
    }
}
