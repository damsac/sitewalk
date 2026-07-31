import SwiftUI
import UIKit

// Plan 13 (notes-first): the walk's PRIMARY result. finish() lands here —
// summary + items, NOT a document. A document is built only when the operator
// taps an action button, via the engine-keyed buildDocument(kind:) call,
// landing in the existing ReviewView.
//
// Design: docs/design/notes-mockup.html. Notes are a smart field-log writeup —
// a summary card, findings grouped by kind (trade-aware headers), the raw
// transcript tucked away — with the "TURN THESE NOTES INTO" action row where
// the visible, trade-specific document buttons ARE the differentiation.

struct NotesView: View {
    @Bindable var model: AppModel
    @State private var showTranscript = false
    @State private var exportURL: URL?
    // Plan 16: tap a line to fix it, or add one. The walk is a first draft.
    @State private var itemEdit: NoteItemEdit?

    // Plan 15 D9-15: the vocab seed card shows ONCE, on the FIRST notes-screen
    // appearance — i.e. right after the user's first real walk, when they have
    // concrete context for "what did the mic get wrong?" (the CANON "walk
    // before the vocab card" intent; no onboarding demo-walk step exists).
    // The UserDefaults flag is a UX mirror only — CORE stays authoritative for
    // idempotency (a shown-but-skipped card leaves core cold; a re-seed of an
    // applied pack no-ops on the `_seeds` marker). // sac: exact placement in
    // the flow + presentation style (sheet vs inline card) are yours.
    @State private var vocabPack: VocabPack?
    static let vocabCardShownKey = "onboardingVocabCardShown"

    /// The document types actually available, read from the LIVE schemas
    /// rather than a hardcoded per-trade switch.
    ///
    /// `DocKinds.legalKinds` predates the DocumentSchema seam (#244) and only
    /// knows the seven built-ins, so a doc type authored in the Document
    /// Builder could be created and then never used — a dead end (Isaac,
    /// field report). Reading schemas here is what makes the Builder mean
    /// anything.
    @State private var docChoices: [DocChoice] = []
    /// Jobs, for filing this walk. R4: "the user corrects on the report" —
    /// this screen IS the report, and it's where the operator is standing
    /// right after the walk, still holding the context.
    @State private var jobs: [JobModel] = []
    @State private var fileError: String?
    @State private var showNewJob = false
    /// The photo open in the full-size viewer, if any (#224).
    @State private var zoomedPhoto: PhotoModel?
    @State private var newJobName = ""

    /// One offerable document type.
    struct DocChoice: Identifiable, Hashable {
        var id: String { kind }
        let kind: String
        let label: String
        let stamp: String
    }

    private var emptyNotes: NotesModel { NotesModel(summary: "", items: [], docKind: "report", queued: false) }
    private var notes: NotesModel { model.notes ?? emptyNotes }

    /// Live schemas, falling back to the built-in list.
    ///
    /// The fallback matters: the demo engine and any pre-schema path still
    /// need buttons, and an empty action row would look like a broken screen
    /// rather than a missing feature.
    private func loadDocChoices() {
        let schemas = (try? model.engine.listDocumentSchemas(tradeKey: model.trade.key)) ?? []
        if schemas.isEmpty {
            docChoices = DocKinds.legalKinds(for: model.trade.key).map {
                DocChoice(kind: $0, label: DocKinds.label(for: $0), stamp: DocKinds.stamp(for: $0))
            }
        } else {
            // Only what core will actually build. `buildable(from:tradeKey:)`
            // mirrors `resolve_active_schema` — exact trade match, one winner
            // per kind — so a button's label always describes a document that
            // really comes out.
            //
            // The trade filter is the field fix (Isaac, TestFlight 2026-07-28):
            // `listDocumentSchemas` returns nil-trade rows for every trade, so
            // the universal `report` built-in was offered to a landscape
            // operator and then refused by the build. Same for any doc type
            // duplicated from it.
            let winners = DocumentSchemaModel.buildable(from: schemas, tradeKey: model.trade.key)
            docChoices = winners.map {
                DocChoice(kind: $0.kind, label: $0.label, stamp: $0.numberPrefix.uppercased())
            }
            // Every schema filtered out — an operator with only nil-trade
            // customs would face an action row with no buttons, which reads as
            // a broken screen. Fall back to the built-in list for the trade,
            // which core's `doc_kinds_for_template` accepts unconditionally.
            if docChoices.isEmpty {
                docChoices = DocKinds.legalKinds(for: model.trade.key).map {
                    DocChoice(kind: $0, label: DocKinds.label(for: $0), stamp: DocKinds.stamp(for: $0))
                }
            }
        }
    }

    private func loadJobs() {
        jobs = ((try? model.engine.listJobs()) ?? []).filter { $0.status == .active }
    }

    /// The job this walk is currently filed under, if any.
    private var filedJob: JobModel? {
        guard let sessionId = model.currentSessionId,
              let walk = model.sessionWalks.first(where: { $0.sessionId == sessionId }),
              let jobId = walk.jobId
        else { return nil }
        return jobs.first { $0.id == jobId }
    }

    private func file(under jobId: String?) {
        guard let sessionId = model.currentSessionId else { return }
        do {
            try model.setWalkJob(sessionId: sessionId, jobId: jobId)
            fileError = nil
        } catch {
            fileError = EngineErrorText.readable(error)
        }
    }

    private func createAndFile() {
        let name = newJobName
        newJobName = ""
        do {
            let job = try model.engine.createJob(name: name)
            loadJobs()
            file(under: job.id)
        } catch {
            fileError = EngineErrorText.readable(error)
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            // Indeterminate top bar while finish() computes; everything below
            // stays a stable skeleton, so nothing shifts when the notes land
            // (dam's UX note: navigate once, fill in place).
            if model.notesLoading {
                ProgressView().progressViewStyle(.linear).tint(Theme.C.orangeDeep).frame(height: 2)
            } else {
                Theme.C.paper.frame(height: 2)
            }
            header
            MetaStrip(left: metaLeft, right: model.notesLoading ? "READING YOUR WALK…" : metaRight)
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    if model.notesLoading {
                        skeleton
                    } else {
                        summaryCard
                        // Plan 14: the comprehensive coordination buckets sit
                        // ABOVE the terse tag-grouped board (additive — the board
                        // still carries the priced line items).
                        bucketSections
                        if notes.items.isEmpty && notes.notes.isEmpty {
                            emptyState
                            if !notes.queued { addLineButton }
                            // A walk can capture nothing but photos — someone
                            // documenting damage without narrating it. Without
                            // this the empty state would claim the walk was
                            // empty while their photos sat invisible.
                            photoStrip
                        } else {
                            if !notes.items.isEmpty {
                                ForEach(grouped, id: \.0) { kind, items in
                                    SectionHead(left: sectionTitle(kind), right: "\(items.count)", heavyRule: false)
                                        .padding(.top, 4)
                                    ForEach(items) { item in
                                        // Edit gates on !queued (Plan 16 contract clause a):
                                        // a queued/Failed session's items are swept by the
                                        // retry reprocess — the engine refuses edits by
                                        // design, so the affordance must not render.
                                        CapturedRow(item: item)
                                            .contentShape(Rectangle())
                                            .onTapGesture {
                                                if !notes.queued { itemEdit = .edit(item) }
                                            }
                                    }
                                }
                            }
                            if !notes.queued { addLineButton }
                            photoStrip
                            transcriptRow
                        }
                        if let message = model.notesEditError {
                            errorBar(message)
                        }
                        if let error = model.documentBuildError {
                            errorBar(error)
                        }
                    }
                }
                .padding(.bottom, 18)
            }
            .background(Theme.C.paperDeep)
            actionBar
                .disabled(model.notesLoading)
                .opacity(model.notesLoading ? 0.4 : 1)
        }
        .background(Theme.C.paperDeep.ignoresSafeArea())
        .toolbar(.hidden, for: .navigationBar)
        .navigationBarBackButtonHidden(true)
        .sheet(isPresented: Binding(get: { exportURL != nil }, set: { if !$0 { exportURL = nil } })) {
            if let url = exportURL { ShareSheet(url: url) { _ in exportURL = nil } }
        }
        .fullScreenCover(isPresented: Binding(
            get: { zoomedPhoto != nil }, set: { if !$0 { zoomedPhoto = nil } }
        )) { photoViewer }
        .task {
            // Rehydrate the photo gallery from the store whenever the notes
            // screen appears (field fix, jefe-2026-07-24). `model.photos` is an
            // in-memory array; before this it was only reloaded in ReviewView,
            // so a reopened walk — and a walk finished after a relaunch — showed
            // ZERO photos even though the bytes + rows were safely persisted,
            // reading to the operator as "my photos disappeared." Mirrors
            // ReviewView.task; a cheap, idempotent store read.
            if let sessionId = model.currentSessionId {
                model.loadPhotos(sessionId: sessionId)
            }
        }
        .onAppear {
            // Read the LIVE schemas and jobs every appearance, not once: the
            // operator can author a doc type or add a job between walks, and a
            // stale list is exactly the dead end this screen had before.
            loadDocChoices()
            loadJobs()

            // F2 (dam's #238 review): never burn the one-shot vocab-seed on a
            // practice run — the card would pop mid-"nothing gets saved" (and
            // confirming writes REAL vocabulary), then the user's first real
            // walk would never see it.
            guard !model.isPracticeWalk,
                  !UserDefaults.standard.bool(forKey: Self.vocabCardShownKey),
                  let pack = VocabPack.bundled(for: model.trade.key) else { return }
            UserDefaults.standard.set(true, forKey: Self.vocabCardShownKey) // show once, ever
            vocabPack = pack
        }
        .sheet(isPresented: Binding(get: { vocabPack != nil }, set: { if !$0 { vocabPack = nil } })) {
            if let pack = vocabPack {
                VocabSeedCard(model: model, pack: pack) { vocabPack = nil }
            }
        }
        .sheet(item: $itemEdit) { target in
            NoteItemEditSheet(target: target, model: model)
                .presentationDetents([.medium])
                .presentationDragIndicator(.visible)
                .presentationBackground(Theme.C.paper)
        }
    }

    // Add a manual line — dashed, in the tag grammar, distinct from a captured row.
    /// The photos taken on this walk.
    ///
    /// Issue #224. Capture, persistence and rehydration all worked; the photos
    /// simply had no surface here — `model.photos` rendered only in
    /// `ReviewView`, which is reachable only after a document builds. So an
    /// operator who finished a walk and saved notes (Isaac: "they get an email
    /// months after the fact asking for details about a job") never saw the
    /// photos they took, on the walk or on reopen. The bytes were always safe.
    ///
    /// Renders nothing when there are no photos — no dashed empty box. This
    /// screen already has an empty state, and a second one saying "no photos"
    /// on every walk that didn't need photos is noise.
    @ViewBuilder
    private var photoStrip: some View {
        if !model.photos.isEmpty {
            SectionHead(left: "PHOTOS", right: "\(model.photos.count)", heavyRule: false)
                .padding(.top, 4)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(model.photos) { photo in
                        Button { zoomedPhoto = photo } label: {
                            photoThumb(photo)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.horizontal, Theme.S.screenPad)
                .padding(.vertical, 10)
            }
        }
    }

    private func photoThumb(_ photo: PhotoModel) -> some View {
        let url = AppModel.photoURL(filename: photo.filename)
        return Group {
            if let image = UIImage(contentsOfFile: url.path) {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFill()
            } else {
                // The row exists but the bytes don't — a sweep race, or a
                // restore that dropped the container. Say so quietly rather
                // than render a blank tile that looks like a broken layout.
                Text("MISSING")
                    .font(Theme.F.mono(7, .semibold))
                    .tracking(0.6)
                    .foregroundStyle(Theme.C.ink45)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(Theme.C.paperDeep)
            }
        }
        .frame(width: 78, height: 78)
        .clipped()
        .overlay(Rectangle().stroke(Theme.C.hairline, lineWidth: 1))
    }

    /// Full-size viewer. A 78pt thumbnail answers "did I take a photo?" but not
    /// "what did that crack look like?", which is the question someone opening a
    /// months-old walk actually has.
    @ViewBuilder
    private var photoViewer: some View {
        if let photo = zoomedPhoto {
            ZStack(alignment: .topTrailing) {
                Color.black.ignoresSafeArea()
                if let image = UIImage(contentsOfFile: AppModel.photoURL(filename: photo.filename).path) {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFit()
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
                Button { zoomedPhoto = nil } label: {
                    Text("CLOSE")
                        .font(Theme.F.mono(10, .semibold))
                        .tracking(1.0)
                        .foregroundStyle(.white)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 10)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .padding(.top, 8)
                .padding(.trailing, 8)
            }
        }
    }

    private var addLineButton: some View {
        Button { itemEdit = .add } label: {
            Text("＋ ADD LINE")
                .font(Theme.F.mono(9, .semibold)).tracking(1.0)
                .foregroundStyle(Theme.C.amberInk)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 11)
                .overlay(RoundedRectangle(cornerRadius: Theme.S.radiusCard)
                    .stroke(style: StrokeStyle(lineWidth: 1.5, dash: [4, 3]))
                    .foregroundStyle(Theme.C.orangeDeep.opacity(0.6)))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 12)
    }

    // MARK: Header

    private var header: some View {
        HStack(alignment: .center, spacing: 10) {
            Button { model.dismissNotes() } label: {
                Text("‹ CLOSE")
                    .font(Theme.F.mono(9, .semibold))
                    .tracking(1.0)
                    .foregroundStyle(Theme.C.ink60)
                    .padding(.vertical, 4)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            Spacer()
            Text("WALK NOTES")
                .font(Theme.F.mono(9, .semibold))
                .tracking(2.0)
                .foregroundStyle(Theme.C.amberInk)
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 12)
        .padding(.bottom, 10)
        .background(Theme.C.paper)
    }

    private var metaLeft: String {
        (BusinessProfile.current?.businessName ?? model.trade.biz).uppercased()
    }
    private var metaRight: String {
        let d = Date().formatted(.dateTime.weekday(.abbreviated).month(.abbreviated).day())
        return "\(d.uppercased()) · \(notes.items.count) ITEM\(notes.items.count == 1 ? "" : "S")"
    }

    // MARK: Summary

    private var summaryCard: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 0) {
                Rectangle().fill(Theme.C.orangeDeep).frame(width: 3)
                VStack(alignment: .leading, spacing: 6) {
                    Text("SUMMARY")
                        .font(Theme.F.mono(8, .semibold)).tracking(2.0)
                        .foregroundStyle(Theme.C.ink60)
                    Text(notes.summary.isEmpty ? "Nothing was captured on this walk." : notes.summary)
                        .font(Theme.F.serif(14))
                        .foregroundStyle(Theme.C.ink)
                        .lineSpacing(3)
                    if notes.queued {
                        // Plan 20 F5: a REOPENED still-queued walk must not
                        // reuse the reconnect promise — the app-open retry
                        // sweep may already have run and exhausted. Distinct
                        // string per banner reason.
                        // // sac: the reopened-Failed copy below is a
                        // // sac: placeholder — the wording is yours.
                        Text(model.notesBannerReason == .reopened
                             ? "COULDN’T FINISH THIS WALK — RETRYING AUTOMATICALLY"
                             : "SAVED OFFLINE — DOCUMENTS UNLOCK WHEN YOU RECONNECT")
                            .font(Theme.F.mono(8.5, .semibold)).tracking(0.4)
                            .foregroundStyle(Theme.C.yellowTag)
                            .padding(.top, 2)
                    }
                }
                .padding(.horizontal, 13).padding(.vertical, 11)
            }
            .background(Theme.C.sheet)
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 14)
        .padding(.bottom, 4)
    }

    // Stable placeholder while notes compute — same rough shape as the real
    // content so the swap-in doesn't move anything.
    private var skeleton: some View {
        VStack(alignment: .leading, spacing: 0) {
            RoundedRectangle(cornerRadius: Theme.S.radiusCard).fill(Theme.C.paperDeep)
                .frame(height: 78)
                .padding(.horizontal, Theme.S.screenPad).padding(.top, 14)
            ForEach(0..<3, id: \.self) { _ in
                HStack(spacing: 10) {
                    RoundedRectangle(cornerRadius: Theme.S.radiusCard).fill(Theme.C.paperDeep).frame(width: 44, height: 16)
                    VStack(alignment: .leading, spacing: 5) {
                        RoundedRectangle(cornerRadius: Theme.S.radiusCard).fill(Theme.C.paperDeep).frame(height: 12).frame(maxWidth: .infinity)
                        RoundedRectangle(cornerRadius: Theme.S.radiusCard).fill(Theme.C.paperDeep).frame(height: 9).frame(maxWidth: 180)
                    }
                }
                .padding(.horizontal, Theme.S.screenPad).padding(.vertical, 12)
                .overlay(alignment: .bottom) { Theme.C.hairline.frame(height: 1) }
            }
        }
        .opacity(0.7)
    }

    private var emptyState: some View {
        Text("NOTHING WAS CAPTURED ON THIS WALK")
            .font(Theme.F.mono(9)).tracking(0.6)
            .foregroundStyle(Theme.C.ink45)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 22)
            .overlay(RoundedRectangle(cornerRadius: Theme.S.radiusCard)
                .stroke(style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
                .foregroundStyle(Theme.C.ink45))
            .padding(Theme.S.screenPad)
    }

    // MARK: Transcript (collapsed)

    private var transcriptRow: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button { withAnimation(.easeOut(duration: 0.2)) { showTranscript.toggle() } } label: {
                HStack {
                    Text("Show the full transcript")
                        .font(Theme.F.mono(8.5, .semibold)).tracking(0.8)
                        .foregroundStyle(Theme.C.ink60)
                    Spacer()
                    Image(systemName: showTranscript ? "chevron.down" : "chevron.right")
                        .font(Theme.F.mono(9)).foregroundStyle(Theme.C.amberInk)
                }
                .padding(9)
                .overlay(RoundedRectangle(cornerRadius: Theme.S.radiusCard)
                    .stroke(style: StrokeStyle(lineWidth: 1, dash: [3, 3]))
                    .foregroundStyle(Theme.C.hairline))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if showTranscript {
                Text(model.transcript.isEmpty ? "—" : model.transcript)
                    .font(Theme.F.mono(10.5)).foregroundStyle(Theme.C.ink60)
                    .lineSpacing(5)
                    .padding(.top, 8)
            }
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 14)
    }

    private func errorBar(_ text: String) -> some View {
        HStack(spacing: 0) {
            Theme.C.redTag.frame(width: 3)
            Text(text)
                .font(Theme.F.mono(9)).foregroundStyle(Theme.C.redTag)
                .padding(.horizontal, 9).padding(.vertical, 7)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(Theme.C.redTint)
        .padding(.horizontal, Theme.S.screenPad).padding(.top, 12)
    }

    // MARK: File under a job

    /// Filing, at the moment the operator actually knows the answer.
    ///
    /// This is R4's "the user corrects on the report" half, and it fixes the
    /// bug Isaac hit in the field: he walked 117 Lexington, finished the walk,
    /// and it never landed under that job — because nothing files it and the
    /// only affordance was a chip back on the board. Expecting the walk to end
    /// up on the job you just walked is the CORRECT mental model; the flow
    /// simply didn't honor it.
    ///
    /// Still not automatic (R4 forbids pre-labeling, and silently filing under
    /// a guessed job is worse than not filing — the walk lands somewhere the
    /// operator won't think to look). Inferring a SUGGESTION is issue #265.
    private var fileUnderRow: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Text("FILE UNDER")
                    .font(Theme.F.mono(8.5, .semibold)).tracking(1.8)
                    .foregroundStyle(Theme.C.ink60)
                Spacer()
                Menu {
                    ForEach(jobs) { job in
                        Button {
                            file(under: job.id)
                        } label: {
                            Label(job.name, systemImage:
                                filedJob?.id == job.id ? "checkmark" : "folder")
                        }
                    }
                    Divider()
                    Button { showNewJob = true } label: {
                        Label("New job…", systemImage: "plus")
                    }
                    if filedJob != nil {
                        Button(role: .destructive) { file(under: nil) } label: {
                            Label("Unfile", systemImage: "xmark")
                        }
                    }
                } label: {
                    HStack(spacing: 5) {
                        Text(filedJob?.name ?? "Choose a job")
                            .font(Theme.F.mono(9, .semibold)).tracking(0.8)
                            .lineLimit(1).truncationMode(.tail)
                        Image(systemName: "chevron.down")
                            .font(.system(size: 8, weight: .bold))
                    }
                    .foregroundStyle(filedJob == nil ? Theme.C.ink60 : Theme.C.amberInk)
                    .padding(.horizontal, 9).padding(.vertical, 7)
                    .background(filedJob == nil ? Theme.C.paperDeep : Theme.C.orangeTint)
                    .contentShape(Rectangle())
                }
            }
            if let fileError {
                Text(fileError)
                    .font(Theme.F.mono(8.5)).foregroundStyle(Theme.C.redTag)
            } else if let auto = model.autoFiled,
                      auto.sessionId == model.currentSessionId,
                      filedJob?.name == auto.jobName {
                // Say it out loud. Auto-filing that happens silently is
                // indistinguishable from a bug the first time it guesses
                // wrong — and the operator needs to know a choice was made
                // on their behalf so they can correct it.
                Text("You said “\(auto.jobName)”, so it’s filed there. Tap to change.")
                    .font(Theme.F.ui(13, .medium))
                    .foregroundStyle(Theme.C.amberInk)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .alert("New job", isPresented: $showNewJob) {
            TextField("Name", text: $newJobName)
            Button("Cancel", role: .cancel) { newJobName = "" }
            Button("Add") { createAndFile() }
        } message: {
            Text("Call it whatever you call it on site.")
        }
    }

    // MARK: Action bar — the differentiation, made visible

    private var actionBar: some View {
        VStack(alignment: .leading, spacing: 9) {
            fileUnderRow

            Text("TURN THESE NOTES INTO")
                .font(Theme.F.mono(8.5, .semibold)).tracking(1.8)
                .foregroundStyle(Theme.C.ink60)
            // Scrolls: with custom document types there can be more than the
            // three built-ins, and a fixed HStack would squeeze them into
            // illegibility rather than overflowing.
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(Array(docChoices.enumerated()), id: \.element.kind) { i, choice in
                        docButton(choice, hero: i == 0)
                    }
                }
                .padding(.vertical, 3)
            }
            Button { exportNotes() } label: {
                Text("⇪  EXPORT NOTES")
                    .font(Theme.F.mono(9, .semibold)).tracking(1.0)
                    .foregroundStyle(Theme.C.ink60)
                    .frame(maxWidth: .infinity).frame(height: 40)
                    .overlay(RoundedRectangle(cornerRadius: Theme.S.radiusCard).stroke(Theme.C.hairline, lineWidth: 1.5))
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 11).padding(.bottom, 10)
        .background(Theme.C.paper)
        .overlay(alignment: .top) { Theme.C.ink.frame(height: 1.5) }
    }

    private func docButton(_ choice: DocChoice, hero: Bool) -> some View {
        let kind = choice.kind
        let building = model.buildingKind == kind
        let disabled = notes.queued || (model.isBuildingDocument && !building)
        return Button { model.buildDocument(kind: kind) } label: {
            if building {
                ProgressView().tint(hero ? Theme.C.onOrange : Theme.C.ink)
            } else {
                // ONE line. The stamp under the label ("Estimate" over "EST")
                // said the same thing twice, and the type ramp made it the
                // bigger problem: at 6.5pt it sat below the 11pt floor, so it
                // was forced up 69% while the label grew 30%.
                Text(choice.label)
                    .font(Theme.F.ui(12, .bold))
                    .tracking(0.04)
                    .lineLimit(1)
            }
        }
        // The control system, rather than a hand-rolled ZStack.
        //
        // The old version drew the hero's lip as a second rounded rectangle at
        // `.offset(y: 3)` — 3pt BELOW its own frame — and the outlined ones with
        // a 2pt stroke, which draws half outside the path. Inside a horizontal
        // ScrollView both got clipped at the frame edge, which is the shaved
        // shadow Isaac spotted under Estimate.
        //
        // `RaisedBlockStyle` builds the lip with padding INSIDE the frame, so
        // nothing overhangs and nothing can be clipped. These also pick up the
        // press travel and the haptic every other control already has.
        .buttonStyle(RaisedBlockStyle(
            face: hero ? Theme.C.orange : Theme.C.sheet,
            lip: hero ? Theme.C.orangeDeep : Theme.C.ink.opacity(0.32),
            text: hero ? Theme.C.onOrange : Theme.C.ink,
            border: hero ? nil : Theme.C.ink,
            height: 46,
            leadingDot: false,
            fillWidth: false
        ))
        .frame(minWidth: 88)
        .fixedSize(horizontal: true, vertical: false)
        .disabled(disabled)
    }

    // MARK: Comprehensive notes — Plan 14 coordination buckets

    // The rich client↔team detail behind the terse board: each entry is a
    // label + the spoken context. Buckets render in a fixed scope→constraints→
    // conditions order; empty ones are omitted. Rendered additively above the
    // tag-grouped board (dam's plumbing note: the board stays the priced items).
    private let bucketOrder: [NotesBucket] = [.scopeOfWork, .constraints, .conditionsAndIssues]

    private func bucketTitle(_ b: NotesBucket) -> String {
        switch b {
        case .scopeOfWork:         return "SCOPE OF WORK"
        case .constraints:         return "CONSTRAINTS"
        case .conditionsAndIssues: return "CONDITIONS & ISSUES"
        }
    }

    private var bucketed: [(NotesBucket, [NotesEntryFixture])] {
        bucketOrder.compactMap { b in
            let entries = notes.notes.filter { $0.bucket == b }
            return entries.isEmpty ? nil : (b, entries)
        }
    }

    // Empty `bucketed` emits nothing — no guard needed at the call site.
    @ViewBuilder private var bucketSections: some View {
        ForEach(bucketed, id: \.0) { bucket, entries in
            SectionHead(left: bucketTitle(bucket), right: "\(entries.count)", heavyRule: false)
                .padding(.top, 4)
            ForEach(entries) { notesEntryRow($0) }
        }
    }

    private func notesEntryRow(_ entry: NotesEntryFixture) -> some View {
        HStack(alignment: .top, spacing: 9) {
            Rectangle().fill(Theme.C.ink35).frame(width: 5, height: 5).padding(.top, 5)
            VStack(alignment: .leading, spacing: 3) {
                Text(entry.label)
                    .font(Theme.F.cond(13.5, .semibold))
                    .foregroundStyle(Theme.C.ink)
                    .fixedSize(horizontal: false, vertical: true)
                if !entry.detail.isEmpty {
                    Text(entry.detail)
                        .font(Theme.F.cond(11.5, .medium))
                        .foregroundStyle(Theme.C.ink60)
                        .lineSpacing(1.5)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.vertical, 8)
        .overlay(alignment: .bottom) { Theme.C.hairlineSoft.frame(height: 1) }
    }

    // MARK: Grouping (trade-aware headers, attention-first)

    private let order: [TagKind] = [.red, .yellow, .plain, .green]
    private var grouped: [(TagKind, [CapturedFixture])] {
        order.compactMap { k in
            let items = notes.items.filter { $0.tag.kind == k }
            return items.isEmpty ? nil : (k, items)
        }
    }

    private func sectionTitle(_ kind: TagKind) -> String {
        switch (model.trade.key, kind) {
        case ("inspection", .red): return "SAFETY"
        case ("inspection", .yellow): return "REPAIR"
        case ("inspection", .plain): return "OBSERVED"
        case ("inspection", .green): return "CHECKED — OK"
        case ("property", .red): return "DEDUCTIONS"
        case ("property", .yellow): return "FOLLOW-UP"
        case ("property", .plain): return "NOTED"
        case ("property", .green): return "CONDITION OK"
        default:
            switch kind {
            case .red: return "NEEDS ATTENTION"
            case .yellow: return "FOLLOW-UP"
            case .plain: return "SCOPE"
            case .green: return "LOOKS GOOD"
            }
        }
    }

    // MARK: Export — plain-text notes (Granola-style copy/paste) via share sheet

    private func exportNotes() {
        var lines: [String] = []
        lines.append(metaLeft)
        lines.append("Walk notes — \(Date().formatted(.dateTime.month().day().year()))")
        lines.append("")
        if !notes.summary.isEmpty { lines.append(notes.summary); lines.append("") }
        for (bucket, entries) in bucketed {
            lines.append(bucketTitle(bucket))
            for e in entries {
                lines.append("  • \(e.label)")
                if !e.detail.isEmpty { lines.append("      \(e.detail)") }
            }
            lines.append("")
        }
        for (kind, items) in grouped {
            lines.append(sectionTitle(kind))
            for it in items {
                let right = it.right.isEmpty ? "" : "  (\(it.right))"
                lines.append("  • \(it.text)\(right)")
            }
            lines.append("")
        }
        lines.append("Prepared with Sitewalk")
        let text = lines.joined(separator: "\n")
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("walk-notes.txt")
        try? text.data(using: .utf8)?.write(to: url)
        exportURL = url
    }
}

// MARK: - Item edit / add sheet (Plan 16)

/// What the tap-to-edit sheet is doing: fixing an existing line, or adding one.
private enum NoteItemEdit: Identifiable {
    case edit(CapturedFixture)
    case add
    var id: String { if case .edit(let item) = self { return item.id.uuidString } else { return "add" } }
}

/// Fix a captured line's text / quantity, remove it, or add a new one. Commits
/// through the core (AppModel → Plan 16 CRUD), so the correction reaches every
/// rebuilt document — not just this screen.
private struct NoteItemEditSheet: View {
    let target: NoteItemEdit
    let model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var text: String
    @State private var right: String

    init(target: NoteItemEdit, model: AppModel) {
        self.target = target
        self.model = model
        switch target {
        case .edit(let item):
            _text = State(initialValue: item.text)
            _right = State(initialValue: item.right)
        case .add:
            _text = State(initialValue: "")
            _right = State(initialValue: "")
        }
    }

    private var isEdit: Bool { if case .edit = target { return true } else { return false } }
    private var canSave: Bool { !text.trimmingCharacters(in: .whitespaces).isEmpty }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Button { dismiss() } label: {
                    Text("CANCEL")
                        .font(Theme.F.mono(9, .semibold)).tracking(1.0)
                        .foregroundStyle(Theme.C.ink60)
                }
                .buttonStyle(.plain)
                Spacer()
                Text(isEdit ? "EDIT LINE" : "ADD LINE")
                    .font(Theme.F.mono(9, .semibold)).tracking(2.0)
                    .foregroundStyle(Theme.C.amberInk)
            }
            .padding(.horizontal, Theme.S.screenPad).padding(.top, 18).padding(.bottom, 14)
            .overlay(alignment: .bottom) { Theme.C.ink.frame(height: 2) }

            VStack(alignment: .leading, spacing: 16) {
                field("DESCRIPTION", text: $text, placeholder: "Mower — front lawn")
                field("QUANTITY", text: $right, placeholder: "× 1 · 3 cu yd · optional")
            }
            .padding(.horizontal, Theme.S.screenPad).padding(.top, 20)

            Spacer(minLength: 12)

            HStack(spacing: 10) {
                if isEdit {
                    Button { commitRemove() } label: {
                        Text("REMOVE")
                            .font(Theme.F.ui(14, .bold)).tracking(1.1)
                            .foregroundStyle(Theme.C.redTag)
                            .frame(width: 118).frame(height: 54)
                            .overlay(RoundedRectangle(cornerRadius: Theme.S.radius)
                                .stroke(Theme.C.redTag, lineWidth: 2))
                    }
                    .buttonStyle(.plain)
                }
                Button { commitSave() } label: {
                    Text(isEdit ? "SAVE" : "ADD")
                        .font(Theme.F.ui(15, .bold)).tracking(1.4)
                        .foregroundStyle(Theme.C.onOrange)
                        .frame(maxWidth: .infinity).frame(height: 54)
                        .background(RoundedRectangle(cornerRadius: Theme.S.radius).fill(Theme.C.orange))
                }
                .buttonStyle(.plain)
                .disabled(!canSave)
                .opacity(canSave ? 1 : 0.4)
            }
            .padding(.horizontal, Theme.S.screenPad).padding(.top, 12).padding(.bottom, 14)
            .overlay(alignment: .top) { Theme.C.hairline.frame(height: 1) }
        }
        .background(Theme.C.paper.ignoresSafeArea())
    }

    private func field(_ label: String, text: Binding<String>, placeholder: String) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(label)
                .font(Theme.F.mono(8, .semibold)).tracking(1.4)
                .foregroundStyle(Theme.C.ink45)
            TextField(placeholder, text: text)
                .font(Theme.F.cond(15, .semibold))
                .autocorrectionDisabled()
                .padding(.bottom, 6)
                .overlay(alignment: .bottom) { Theme.C.orangeDeep.frame(height: 2) }
        }
    }

    private func commitSave() {
        let t = text.trimmingCharacters(in: .whitespaces)
        let r = right.trimmingCharacters(in: .whitespaces)
        guard !t.isEmpty else { return }
        switch target {
        case .edit(let item): model.editItem(item, text: t, right: r)
        case .add:            model.addNoteItem(text: t, right: r)
        }
        dismiss()
    }

    private func commitRemove() {
        if case .edit(let item) = target { model.removeNoteItem(item) }
        dismiss()
    }
}
