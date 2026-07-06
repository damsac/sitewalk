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

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                if let doc = model.document {
                    VStack(alignment: .leading, spacing: 0) {
                        Letterhead(
                            biz: model.trade.biz,
                            bizSub: model.trade.bizSub,
                            docKind: model.trade.docKind,
                            docNo: model.trade.docNo,
                            docDate: model.trade.docDate
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

    // sac: functional-plain — capture button + bare grid, no thumbnails/empty
    // state polish. Restyle freely; the wiring (PhotosPicker → capturePhoto,
    // Image(contentsOfFile:) → removePhoto) is what matters here.
    private var photoGallery: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("PHOTOS")
                    .font(Theme.F.mono(11, .medium))
                    .tracking(1.2)
                    .foregroundStyle(Theme.C.ink60)
                Spacer()
                PhotosPicker(selection: $photoPickerItem, matching: .images) {
                    Text("+ ADD PHOTO")
                        .font(Theme.F.mono(11, .medium))
                        .foregroundStyle(Theme.C.orangeDeep)
                }
            }
            if let error = model.photoError {
                Text(error)
                    .font(Theme.F.mono(10))
                    .foregroundStyle(.red)
            }
            if !model.photos.isEmpty {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 72))], spacing: 8) {
                    ForEach(model.photos) { photo in
                        photoThumbnail(photo)
                    }
                }
            }
        }
    }

    private func photoThumbnail(_ photo: PhotoModel) -> some View {
        let url = FileManager.default
            .urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("photos")
            .appendingPathComponent(photo.filename)
        return ZStack(alignment: .topTrailing) {
            if let uiImage = UIImage(contentsOfFile: url.path) {
                Image(uiImage: uiImage)
                    .resizable()
                    .scaledToFill()
                    .frame(width: 72, height: 72)
                    .clipped()
            } else {
                Rectangle().fill(Theme.C.ink.opacity(0.08)).frame(width: 72, height: 72)
            }
            Button { model.removePhoto(photo) } label: {
                Image(systemName: "xmark.circle.fill")
                    .foregroundStyle(.white, .black.opacity(0.6))
            }
            .padding(2)
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
            .overlay(alignment: .bottom) { Theme.C.orange.frame(height: 2) }

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
