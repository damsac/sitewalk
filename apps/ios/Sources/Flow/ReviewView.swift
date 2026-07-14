import SwiftUI
import PhotosUI

// Document review — interactive: tap an amount to fix it, gaps fill the same
// way, total recomputes, SEND exports the PDF.

struct ReviewView: View {
    @Bindable var model: AppModel
    @FocusState private var amountFocused: Bool
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
                .foregroundStyle(Theme.C.orangeDeep)
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
                            docKind: model.trade.docKind,
                            docNo: model.trade.docNo,
                            docDate: model.letterheadDate,
                            branding: model.branding
                        )
                        ForEach(doc.rows) { row in
                            DocRowView(row: row)
                                .contentShape(Rectangle())
                                .onTapGesture { model.beginEdit(row) }
                        }
                        TotalRow(key: doc.totalKey, value: doc.totalValue, gaps: doc.gapCount)
                            .padding(.top, 2)
                        RevNote(text: doc.note)
                            .padding(.top, 10)

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
                Button { model.discardDocument() } label: {
                    Text("DISCARD")
                        .font(Theme.F.ui(14, .bold))
                        .tracking(1.1)
                        .foregroundStyle(Theme.C.ink)
                        .frame(width: 124)
                        .frame(height: 58)
                        .overlay(
                            RoundedRectangle(cornerRadius: Theme.S.radius)
                                .stroke(Theme.C.ink, lineWidth: 2)
                        )
                }
                .buttonStyle(.plain)
                Button { model.makePDF() } label: {
                    ZStack {
                        RoundedRectangle(cornerRadius: Theme.S.radius)
                            .fill(Theme.C.orangeDeep)
                            .offset(y: 3)
                        RoundedRectangle(cornerRadius: Theme.S.radius)
                            .fill(Theme.C.orange)
                        Text(model.document?.send ?? "SEND")
                            .font(Theme.F.ui(15, .bold))
                            .tracking(1.4)
                            .foregroundStyle(Theme.C.onOrange)
                    }
                    .frame(height: 58)
                }
                .buttonStyle(.plain)
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
                        .foregroundStyle(Theme.C.orangeDeep)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 5)
                        .overlay(
                            RoundedRectangle(cornerRadius: 6)
                                .stroke(Theme.C.orangeDeep, lineWidth: 1.5)
                        )
                }
                .buttonStyle(.plain)
            }

            if let error = model.photoError {
                HStack(spacing: 0) {
                    Theme.C.redTag.frame(width: 3)
                    Text(error.uppercased())
                        .font(Theme.F.mono(8))
                        .tracking(0.4)
                        .foregroundStyle(Theme.C.redTag)
                        .padding(.horizontal, 9)
                        .padding(.vertical, 6)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .background(Theme.C.redTint)
            }

            if model.photos.isEmpty {
                Text("NO PHOTOS — USE THE PHOTO BUTTON DURING A WALK, OR ADD HERE")
                    .font(Theme.F.mono(8))
                    .tracking(0.6)
                    .foregroundStyle(Theme.C.ink35)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 18)
                    .padding(.horizontal, 10)
                    .overlay(
                        RoundedRectangle(cornerRadius: 4)
                            .stroke(style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
                            .foregroundStyle(Theme.C.ink35)
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
        let url = FileManager.default
            .urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("photos")
            .appendingPathComponent(photo.filename)
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
                        .foregroundStyle(Theme.C.orangeDeep)
                }
            }
        }
    }

    private var editSheet: some View {
        VStack(alignment: .leading, spacing: 16) {
            SectionLabel("SET AMOUNT")
            HStack(spacing: 4) {
                Text("$")
                    .font(Theme.F.mono(24, .semibold))
                    .foregroundStyle(Theme.C.ink60)
                TextField("0", text: $model.editText)
                    .font(Theme.F.mono(28, .semibold))
                    .keyboardType(.numberPad)
                    .focused($amountFocused)
            }
            .padding(.bottom, 4)
            .overlay(alignment: .bottom) { Theme.C.orangeDeep.frame(height: 2) }

            Button { model.commitEdit() } label: {
                ZStack {
                    RoundedRectangle(cornerRadius: Theme.S.radius)
                        .fill(Theme.C.orangeDeep)
                        .offset(y: 3)
                    RoundedRectangle(cornerRadius: Theme.S.radius)
                        .fill(Theme.C.orange)
                    Text("SET")
                        .font(Theme.F.ui(15, .bold))
                        .tracking(1.4)
                        .foregroundStyle(Theme.C.onOrange)
                }
                .frame(height: Theme.S.buttonHeight)
            }
            .buttonStyle(.plain)
        }
        .padding(Theme.S.screenPad)
        .presentationDetents([.height(220)])
        .presentationBackground(Theme.C.paper)
        .onAppear { amountFocused = true }
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
