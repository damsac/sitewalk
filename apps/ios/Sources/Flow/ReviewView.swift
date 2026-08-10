import SwiftUI
import PhotosUI

// Document review — interactive: tap an amount to fix it, gaps fill the same
// way, total recomputes, SEND exports the PDF.

struct ReviewView: View {
    @Bindable var model: AppModel
    @FocusState private var amountFocused: Bool
    @FocusState private var titleFocused: Bool
    // Photos (Plan 11) — functional-plain capture entry point + gallery.
    // sac: placement, layout, thumbnails, empty state are yours; this just
    // wires PhotosPicker → bytes → engine.attachPhoto.
    @State private var photoPickerItem: PhotosPickerItem?

    // Back to the notes screen (the reported gap: review previously had only
    // Send / Discard). The doc-kind on the right reads what you're reviewing.
    private var header: some View {
        HStack(alignment: .center, spacing: 10) {
            Button { model.backToNotes() } label: {
                Text("‹ NOTES")
                    .font(Theme.F.mono(9, .semibold))
                    .tracking(1.0)
                    .foregroundStyle(Theme.C.ink60)
                    .padding(.vertical, 4)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            Spacer()
            Text(model.reviewKind.map { DocKinds.label(for: $0).uppercased() } ?? "REVIEW")
                .font(Theme.F.mono(9, .semibold))
                .tracking(2.0)
                .foregroundStyle(Theme.C.amberInk)
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 12)
        .padding(.bottom, 10)
        .background(Theme.C.paper)
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            ScrollView {
                if let doc = model.document {
                    VStack(alignment: .leading, spacing: 0) {
                        Letterhead(
                            biz: model.letterheadBiz,
                            bizSub: model.letterheadSub,
                            // The kind actually being built, not the trade's
                            // lead kind — a Work Order titled ESTIMATE with an
                            // EST- number is the document equivalent of a typo
                            // on company letterhead (#222). `docNo` is blank
                            // outside the fixture estimate rather than invented:
                            // core mints real numbers, and a fake one on a real
                            // document would be worse than none.
                            docKind: doc.docKindLabel.isEmpty
                                ? model.trade.docKind : doc.docKindLabel,
                            // The number core actually minted for THIS build.
                            // It used to fall back to the fixture's EST-0047
                            // for a trade's lead kind and to nothing at all
                            // for the other six — so every real invoice went
                            // out unnumbered. See `DocumentModel.docNumber`.
                            docNo: doc.docNumber.isEmpty ? model.trade.docNo : doc.docNumber,
                            docDate: model.letterheadDate,
                            branding: model.branding
                        )
                        // The authored blocks, above the itemized body — the
                        // order every trade document on paper uses (who and
                        // what this is about, then the lines, then the total).
                        ForEach(doc.sections) { section in
                            DocSectionView(section: section) { field in
                                model.beginFieldEdit(section: section, field: field)
                            }
                        }
                        if !doc.sections.isEmpty {
                            Spacer(minLength: 14)
                        }
                        ForEach(doc.rows) { row in
                            DocRowView(row: row)
                                .contentShape(Rectangle())
                                .onTapGesture { model.beginEdit(row) }
                                .accessibilityElement(children: .combine)
                                .accessibilityHint("Edit this line")
                        }
                        addLineButton
                        TotalRow(key: doc.totalKey, value: doc.totalValue, gaps: doc.gapCount)
                            .padding(.top, 2)
                        // Empty note = no bar. An empty amber block reads as a
                        // rendering failure, and a non-priced document has no
                        // pricing gap to talk about (#222).
                        if !doc.note.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                            RevNote(text: doc.note)
                                .padding(.top, 10)
                        }

                        // Document structure basics (DocumentLayout): operator
                        // terms + a client signature line, set in the PAPER tab.
                        if !model.documentLayout.termsText
                            .trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                            TermsBlock(text: model.documentLayout.termsText)
                        }
                        if model.documentLayout.showSignature {
                            SignatureRow()
                        }

                        // sac: functional-plain gallery + capture entry — yours to restyle.
                        photoGallery
                            .padding(.top, 14)
                    }
                    .padding(18)
                    .background(Theme.C.sheet)
                    .compositingGroup()
                    .shadow(color: Theme.C.ink.opacity(0.12), radius: 1, y: 1)
                    .shadow(color: Theme.C.ink.opacity(0.18), radius: 14, y: 10)
                    .padding(.horizontal, 14)
                    .padding(.top, 14)
                    .padding(.bottom, 20)
                }
            }
            .background(Theme.C.paperDeep)

            HStack(spacing: 10) {
                // Tier 2. DISCARD stays ink rather than going red: it abandons
                // an unsent draft and the walk's notes survive untouched, so
                // dressing it as destructive would overstate what it does.
                Button { model.discardDocument() } label: {
                    BlockLabel("DISCARD", size: 14)
                }
                .buttonStyle(RaisedBlockStyle(
                    face: Theme.C.sheet, lip: Theme.C.ink.opacity(0.32),
                    text: Theme.C.ink, border: Theme.C.ink, height: 58
                ))
                .frame(width: 124)
                // Tier 1. Was a hand-stacked ZStack of two rounded rectangles —
                // the same drawing, but as a View it could never see press
                // state. This is the screen where money gets confirmed, so it's
                // also the press that most needs to be felt.
                Button { model.makePDF() } label: {
                    BlockLabel(model.document?.send ?? "SEND")
                }
                .buttonStyle(RaisedBlockStyle(height: 58))
                .frame(maxWidth: .infinity)
            }
            .padding(.horizontal, Theme.S.screenPad)
            .padding(.top, 12)
            .padding(.bottom, 10)
            .background(Theme.C.paper)
            .overlay(alignment: .top) { Theme.C.hairline.frame(height: 1) }
        }
        .background(Theme.C.paperDeep.ignoresSafeArea())
        .toolbar(.hidden, for: .navigationBar)
        .navigationBarBackButtonHidden(true)
        .task {
            if let sessionId = model.currentSessionId {
                model.loadPhotos(sessionId: sessionId)
            }
        }
        .onChange(of: photoPickerItem) { _, newValue in
            guard let newValue else { return }
            Task {
                if let data = try? await newValue.loadTransferable(type: Data.self) {
                    model.capturePhoto(image: data, itemId: nil)
                }
                photoPickerItem = nil
            }
        }
        .sheet(isPresented: Binding(
            get: { model.editingRowID != nil },
            set: { if !$0 { model.commitEdit() } }
        )) {
            editSheet
        }
        .sheet(isPresented: Binding(
            get: { model.editingField != nil },
            set: { if !$0 { model.commitFieldEdit() } }
        )) {
            fieldSheet
        }
        .sheet(isPresented: Binding(
            get: { model.shareURL != nil },
            set: { if !$0 { model.shareURL = nil } }
        )) {
            if let url = model.shareURL {
                // Only a completed share finalizes the walk; cancelling the
                // sheet returns to review with the document intact (issue #155).
                ShareSheet(url: url) { completed in
                    if completed {
                        model.completeSend()
                    } else {
                        model.shareURL = nil
                    }
                }
            }
        }
    }

    // The gallery reads as a contact sheet ON the paper, not an iOS grid:
    // stamped label, ink-bordered thumbnails with PH-nn index stamps, square
    // ink ✕ remove, dashed empty state, errors in the red note bar.
    private var photoGallery: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                SectionLabel("PHOTOS")
                Text("× \(model.photos.count)")
                    .font(Theme.F.mono(9, .semibold))
                    .foregroundStyle(Theme.C.ink60)
                Spacer()
                PhotosPicker(selection: $photoPickerItem, matching: .images) {
                    Text("+ ADD")
                        .font(Theme.F.mono(9, .semibold))
                        .tracking(1.2)
                        .foregroundStyle(Theme.C.amberInk)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 5)
                        .overlay(
                            RoundedRectangle(cornerRadius: Theme.S.radiusCard)
                                .stroke(Theme.C.orangeDeep, lineWidth: 1.5)
                        )
                }
                .buttonStyle(.plain)
            }

            if let error = model.photoError {
                HStack(spacing: 0) {
                    Theme.C.redTag.frame(width: 3)
                    Text(error)
                        .font(Theme.F.ui(13, .medium))
                        .foregroundStyle(Theme.C.redTag)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.horizontal, 9)
                        .padding(.vertical, 6)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .background(Theme.C.redTint)
            }

            if model.photos.isEmpty {
                Text("No photos yet. Take them during a walk, or add some here.")
                    .font(Theme.F.mono(8))
                    .tracking(0.6)
                    .foregroundStyle(Theme.C.ink45)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 18)
                    .padding(.horizontal, 10)
                    .overlay(
                        RoundedRectangle(cornerRadius: Theme.S.radiusCard)
                            .stroke(style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
                            .foregroundStyle(Theme.C.ink45)
                    )
            } else {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 76), spacing: 10)], alignment: .leading, spacing: 10) {
                    ForEach(Array(model.photos.enumerated()), id: \.element.id) { index, photo in
                        photoThumbnail(photo, index: index)
                    }
                }
            }
        }
    }

    private func photoThumbnail(_ photo: PhotoModel, index: Int) -> some View {
        // Shared with the notes screen's strip (#224) — two screens display
        // photos now, so the path construction has exactly one home.
        let url = AppModel.photoURL(filename: photo.filename)
        return VStack(alignment: .leading, spacing: 3) {
            ZStack(alignment: .topTrailing) {
                Group {
                    if let uiImage = UIImage(contentsOfFile: url.path) {
                        Image(uiImage: uiImage)
                            .resizable()
                            .scaledToFill()
                    } else {
                        Rectangle().fill(Theme.C.paperDeep)
                    }
                }
                .frame(width: 76, height: 76)
                .clipped()
                .overlay(Rectangle().stroke(Theme.C.ink, lineWidth: 1.5))

                Button { model.removePhoto(photo) } label: {
                    Image(systemName: "xmark")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundStyle(Theme.C.paper)
                        .frame(width: 18, height: 18)
                        .background(Theme.C.ink)
                }
                .buttonStyle(.plain)
            }
            HStack(spacing: 4) {
                Text(String(format: "PH-%02d", index + 1))
                    .font(Theme.F.mono(7, .semibold))
                    .tracking(0.8)
                    .foregroundStyle(Theme.C.ink60)
                if photo.itemId != nil {
                    // pinned to a spoken item during the walk
                    Image(systemName: "link")
                        .font(.system(size: 6, weight: .bold))
                        .foregroundStyle(Theme.C.amberInk)
                }
            }
        }
    }

    /// Adding a line back, in the operator's own hand.
    ///
    /// Under the last line rather than beside the header: an estimate is read
    /// top to bottom and the next line goes at the bottom, so the control sits
    /// where the thing it makes will appear. Quiet by design — it is an
    /// occasional correction, not the screen's job, and it must not compete
    /// with SEND.
    private var addLineButton: some View {
        Button { model.addLine() } label: {
            HStack(spacing: 5) {
                Image(systemName: "plus")
                    .font(.system(size: 11, weight: .bold))
                Text("ADD LINE")
                    .font(Theme.F.mono(9, .semibold))
                    .tracking(1.2)
            }
            .foregroundStyle(Theme.C.amberInk)
            .padding(.horizontal, 9)
            .padding(.vertical, 7)
            .overlay(
                RoundedRectangle(cornerRadius: Theme.S.radiusCard)
                    .stroke(Theme.C.orangeDeep.opacity(0.45), lineWidth: 1)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.top, 10)
        .accessibilityLabel("Add a line")
    }

    /// The authored-field editor. Deliberately plainer than the line sheet:
    /// a field has no amount, no quantity and nothing to remove — it is one
    /// block of words — so offering those controls would be offering three
    /// dead buttons.
    private var fieldSheet: some View {
        let label = model.document?.sections
            .first { $0.key == model.editingField?.section }?
            .fields.first { $0.key == model.editingField?.key }?
            .label ?? "Field"
        return VStack(alignment: .leading, spacing: 16) {
            SectionLabel(label)
            TextField("What was said", text: $model.editTitle, axis: .vertical)
                .font(Theme.F.ui(16, .medium))
                .lineLimit(3...8)
                .focused($titleFocused)
                .padding(.bottom, 4)
                .overlay(alignment: .bottom) { Theme.C.hairline.frame(height: 1.5) }
            Text("LEAVE EMPTY TO PUT IT BACK TO A GAP")
                .font(Theme.F.mono(8, .semibold))
                .tracking(1.2)
                .foregroundStyle(Theme.C.ink45)
            Button { model.commitFieldEdit() } label: {
                BlockLabel("SET")
            }
            .buttonStyle(RaisedBlockStyle(height: Theme.S.buttonHeight, leadingDot: false))

            // Removing is not the same as clearing, and the copy has to say
            // so: clearing leaves a gap you owe a value to, this takes the
            // block off the document. Ink rather than red, and last —
            // the same call DISCARD makes on this screen.
            Button { model.removeEditingField() } label: {
                Text("Remove this block")
                    .font(Theme.F.ui(13, .semibold))
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.wellChip)
            Spacer(minLength: 0)
        }
        .padding(Theme.S.screenPad)
        .presentationDetents([.height(360)])
        .presentationBackground(Theme.C.paper)
        .onAppear { titleFocused = true }
    }

    /// One sheet for everything a line can need: its words, its number, and
    /// its removal.
    ///
    /// It used to be SET AMOUNT alone, which quietly said the only thing that
    /// could be wrong on a document was a price. The description is what the
    /// client actually reads, and speech-to-text mishears a word about as
    /// often as it mishears a number (Isaac, 2026-08-09).
    private var editSheet: some View {
        let priced = model.document?.pricesShown ?? false
        return VStack(alignment: .leading, spacing: 16) {
            SectionLabel(model.editingRowIsNew ? "NEW LINE" : "EDIT LINE")

            VStack(alignment: .leading, spacing: 5) {
                Text("DESCRIPTION")
                    .font(Theme.F.mono(8, .semibold))
                    .tracking(1.2)
                    .foregroundStyle(Theme.C.ink60)
                TextField("What the work is", text: $model.editTitle, axis: .vertical)
                    .font(Theme.F.ui(17, .semibold))
                    .lineLimit(1...3)
                    .focused($titleFocused)
                    .padding(.bottom, 4)
                    .overlay(alignment: .bottom) { Theme.C.hairline.frame(height: 1.5) }
            }

            if priced {
                VStack(alignment: .leading, spacing: 5) {
                    // "Leave it empty" belongs on the LABEL, not in the field:
                    // as placeholder text it set at the field's own 24pt and
                    // read as a value someone had typed.
                    Text("AMOUNT · LEAVE EMPTY FOR A GAP")
                        .font(Theme.F.mono(8, .semibold))
                        .tracking(1.2)
                        .foregroundStyle(Theme.C.ink60)
                    HStack(spacing: 4) {
                        Text("$")
                            .font(Theme.F.mono(24, .semibold))
                            .foregroundStyle(Theme.C.ink60)
                        TextField("0", text: $model.editText)
                            .font(Theme.F.mono(24, .semibold))
                            // Decimal, not number: a line can carry cents, so
                            // the keyboard has to be able to type them back.
                            .keyboardType(.decimalPad)
                            .focused($amountFocused)
                    }
                    .padding(.bottom, 4)
                    .overlay(alignment: .bottom) { Theme.C.orangeDeep.frame(height: 2) }
                }
            }

            Button { model.commitEdit() } label: {
                BlockLabel("SET")
            }
            .buttonStyle(RaisedBlockStyle(height: Theme.S.buttonHeight, leadingDot: false))

            // Ink, not red, and last: it removes a line from an unsent draft
            // that can be added straight back, so dressing it as destructive
            // would overstate it — the same call DISCARD makes on this screen.
            Button { model.removeEditingLine() } label: {
                Text(model.editingRowIsNew ? "Cancel" : "Remove this line")
                    .font(Theme.F.ui(13, .semibold))
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.wellChip)
        }
        .padding(Theme.S.screenPad)
        .presentationDetents([.height(priced ? 400 : 300)])
        .presentationBackground(Theme.C.paper)
        // A new line has nothing to say yet, so start in the words; an
        // existing one is most often opened to fix its number.
        .onAppear {
            if model.editingRowIsNew || !priced { titleFocused = true } else { amountFocused = true }
        }
    }
}

// Plan 12 review-time join. sac: layout/labels/empty-states/tap-to-scroll are
// yours — this is the join only. Photos group under the row whose itemId
// matches; everything else (nil itemId, demoted photos, photos on items with
// no row) falls to a session-level group.
extension ReviewView {
    private func photos(for row: DocRowFixture) -> [PhotoModel] {
        guard let itemId = row.itemId else { return [] }
        return model.photos.filter { $0.itemId == itemId }
    }

    private var sessionLevelPhotos: [PhotoModel] {
        let rowItemIds = Set((model.document?.rows ?? []).compactMap { $0.itemId })
        return model.photos.filter { photo in
            photo.itemId == nil || !rowItemIds.contains(photo.itemId!)
        }
    }
}
