import SwiftUI
import PhotosUI

// Live capture — the walk. Everything readable at arm's length; controls
// glove-sized in the thumb zone.

struct WalkView: View {
    @Bindable var model: AppModel
    @State private var showCamera = false
    @State private var pickerItem: PhotosPickerItem?
    @State private var showDiscardConfirm = false
    // First-run DONE hint, one-shot (survives relaunch). Cleared by resetcoach=1.
    @AppStorage(CoachMarks.doneKey) private var coachDoneShown = false

    var body: some View {
        VStack(spacing: 0) {
            TimelineView(.periodic(from: .now, by: 1)) { _ in
                RecBanner(timer: model.isPaused ? "PAUSED" : model.elapsedLabel)
            }

            MetaStrip(
                left: model.trade.site,
                right: model.walkMode == .demo ? "DEMO WALK — SCRIPTED" : "REC — ON-DEVICE STT",
                warn: model.walkMode == .demo
            )

            ScrollView {
                VStack(alignment: .leading, spacing: 6) {
                    Text(model.transcript)
                        .font(Theme.F.mono(11.5))
                        .foregroundStyle(Theme.C.ink60)
                        .lineSpacing(7)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    Caret(height: 13)
                }
                .padding(.horizontal, Theme.S.screenPad)
                .padding(.vertical, 12)
            }
            .frame(height: 168)
            .defaultScrollAnchor(.bottom)

            SectionHead(left: "CAPTURED", right: "\(model.items.count) ITEMS")
                .overlay(alignment: .top) { Theme.C.ink.frame(height: 1.5) }

            ScrollView {
                VStack(spacing: 0) {
                    ForEach(model.items) { item in
                        CapturedRow(item: item)
                            .transition(.opacity.combined(with: .move(edge: .bottom)))
                    }
                }
            }

            // First-run DONE hint: appears once anything is captured (so it
            // shows exactly when DONE becomes tappable), pointing at DONE.
            if !coachDoneShown && !(model.transcript.isEmpty && model.items.isEmpty) {
                CoachCallout(text: "All done talking? Tap DONE — Jefe writes up your notes and paperwork.", pointer: .trailing) {
                    coachDoneShown = true
                }
                .padding(.horizontal, Theme.S.screenPad)
                .padding(.bottom, 2)
                .transition(.opacity)
            }

            VStack(spacing: 12) {
                if model.isPaused {
                    // A real outlined button (dam's note: the plain red text
                    // didn't read as tappable) + a confirm, since discard is
                    // destructive — it drops the whole walk, not just text.
                    Button(role: .destructive) { showDiscardConfirm = true } label: {
                        Text("DISCARD WALK")
                            .font(Theme.F.ui(13, .bold))
                            .tracking(1.0)
                            .foregroundStyle(Theme.C.redTag)
                            .frame(height: 44)
                            .frame(maxWidth: .infinity)
                            .overlay(RoundedRectangle(cornerRadius: Theme.S.radius)
                                .stroke(Theme.C.redTag, lineWidth: 2))
                    }
                    .buttonStyle(.plain)
                    .confirmationDialog("Discard this walk?", isPresented: $showDiscardConfirm, titleVisibility: .visible) {
                        Button("Discard — nothing will be saved", role: .destructive) { model.discardWalk() }
                        Button("Keep walking", role: .cancel) {}
                    } message: {
                        Text("The transcript and everything captured on this walk will be deleted.")
                    }
                } else {
                    Waveform()
                }
                HStack(spacing: 9) {
                    // One tap, zero confirm: camera on device, picker on sim.
                    // The shot pins to the item being spoken (see addPhoto).
                    if CameraCapture.isAvailable {
                        Button { showCamera = true } label: { PhotoSquareButton() }
                            .buttonStyle(.plain)
                    } else {
                        PhotosPicker(selection: $pickerItem, matching: .images) {
                            PhotoSquareButton()
                        }
                        .buttonStyle(.plain)
                    }
                    Button { model.togglePause() } label: {
                        Text(model.isPaused ? "RESUME" : "PAUSE")
                            .font(Theme.F.ui(13.5, .bold))
                            .tracking(1.1)
                            .foregroundStyle(Theme.C.ink)
                            .frame(width: 96)
                            .frame(height: Theme.S.buttonHeight)
                            .overlay(
                                RoundedRectangle(cornerRadius: Theme.S.radius)
                                    .stroke(Theme.C.ink, lineWidth: 2)
                            )
                    }
                    .buttonStyle(.plain)
                    // DONE is finishable once ANYTHING has been captured —
                    // transcript OR extracted items (issue #168). On a voice
                    // walk the live board lags the speech (batched extraction),
                    // and finish() runs a full extraction pass anyway, so
                    // gating on items alone stranded the user on a stuck screen
                    // whenever items hadn't landed yet. Transcript-or-items.
                    let nothingCaptured = model.transcript.isEmpty && model.items.isEmpty
                    Button { coachDoneShown = true; model.finishWalk() } label: { DoneButton() }
                        .buttonStyle(.plain)
                        .frame(maxWidth: .infinity)
                        .disabled(nothingCaptured)
                        .opacity(nothingCaptured ? 0.4 : 1)
                }
            }
            .padding(.horizontal, Theme.S.screenPad)
            .padding(.top, 10)
            .padding(.bottom, 10)
            .overlay(alignment: .top) { Theme.C.hairline.frame(height: 1) }
        }
        .animation(.easeOut(duration: 0.25), value: coachDoneShown)
        .background(Theme.C.paper.ignoresSafeArea())
        .toolbar(.hidden, for: .navigationBar)
        .navigationBarBackButtonHidden(true)
        .fullScreenCover(isPresented: $showCamera) {
            CameraCapture { data in model.addPhoto(data) }
                .ignoresSafeArea()
        }
        .onChange(of: pickerItem) { _, newValue in
            guard let newValue else { return }
            Task {
                if let data = try? await newValue.loadTransferable(type: Data.self) {
                    model.addPhoto(data)
                }
                pickerItem = nil
            }
        }
    }
}
