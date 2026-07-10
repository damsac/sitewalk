import SwiftUI

// Plan 13 (notes-first): the walk's PRIMARY result. finish() lands here —
// items + summary — not a document. A document is only built when the
// operator taps the action row below, via the engine-keyed
// `buildDocument(sessionId:kind:)` call, landing in the EXISTING ReviewView.
//
// // sac: this whole screen is functional-plain plumbing — the real design
// (docs/design/notes-mockup.html) covers grouping by kind (SCOPE / NEEDS
// ATTENTION / …), the full per-trade action-button set, an export/utility
// row, and a collapsed "show what I heard" transcript row. This file only
// guarantees: summary renders, items render, ONE button wires to the
// template's primary doc kind and reaches ReviewView, and the empty/queued
// states don't look broken.

struct NotesView: View {
    @Bindable var model: AppModel

    private var emptyNotes: NotesModel { NotesModel(summary: "", items: [], docKind: "report", queued: false) }
    private var notes: NotesModel { model.notes ?? emptyNotes }
    private var isEmpty: Bool { notes.items.isEmpty }

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    summaryCard
                    if isEmpty {
                        emptyState
                    } else {
                        SectionHead(left: "CAPTURED", right: itemCountLabel)
                        // sac: grouping by item kind (SCOPE / NEEDS ATTENTION / …)
                        // is yours — this is a flat list, plumbing only.
                        ForEach(notes.items) { item in
                            CapturedRow(item: item)
                        }
                    }
                    if let error = model.documentBuildError {
                        Text(error)
                            .font(Theme.F.mono(11))
                            .foregroundStyle(Theme.C.redTag)
                            .padding(Theme.S.screenPad)
                    }
                }
                .padding(.bottom, 20)
            }
            .background(Theme.C.paperDeep)

            actionRow
        }
        .background(Theme.C.paperDeep.ignoresSafeArea())
        .toolbar(.hidden, for: .navigationBar)
        .navigationBarBackButtonHidden(true)
    }

    private var summaryCard: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("NOTES")
                .font(Theme.F.ui(11, .bold))
                .tracking(1.6)
                .foregroundStyle(Theme.C.ink60)
            Text(notes.summary.isEmpty ? "—" : notes.summary)
                .font(Theme.F.cond(15, .medium))
                .foregroundStyle(Theme.C.ink)
            if notes.queued {
                Text("SAVED OFFLINE — WILL FINISH WHEN YOU RECONNECT")
                    .font(Theme.F.mono(10.5, .semibold))
                    .foregroundStyle(Theme.C.orange)
                    .padding(.top, 2)
            }
        }
        .padding(Theme.S.screenPad)
    }

    private var emptyState: some View {
        Text("Nothing was captured on this walk.")
            .font(Theme.F.cond(13, .medium))
            .foregroundStyle(Theme.C.ink60)
            .padding(.horizontal, Theme.S.screenPad)
            .padding(.vertical, 12)
    }

    private var actionRow: some View {
        HStack(spacing: 10) {
            Button { model.dismissNotes() } label: {
                Text("NOT NOW")
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

            // sac: "TURN THESE NOTES INTO ___" is the design's action row —
            // this is the ONE wired button (Task 7), keyed off
            // DocKinds.primaryKind(for: model.trade.key), never off the
            // FFI payload's advisory doc_kind.
            Button { model.buildPrimaryDocument() } label: {
                ZStack {
                    RoundedRectangle(cornerRadius: Theme.S.radius)
                        .fill(Theme.C.orangeDeep)
                        .offset(y: 3)
                    RoundedRectangle(cornerRadius: Theme.S.radius)
                        .fill(Theme.C.orange)
                    if model.isBuildingDocument {
                        ProgressView()
                            .tint(Theme.C.onOrange)
                    } else {
                        Text(buildButtonLabel)
                            .font(Theme.F.ui(15, .bold))
                            .tracking(1.2)
                            .foregroundStyle(Theme.C.onOrange)
                    }
                }
                .frame(height: 58)
            }
            .buttonStyle(.plain)
            .disabled(notes.queued || model.isBuildingDocument)
            .opacity(notes.queued ? 0.5 : 1)
            .frame(maxWidth: .infinity)
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 12)
        .padding(.bottom, 10)
        .background(Theme.C.paper)
        .overlay(alignment: .top) { Theme.C.hairline.frame(height: 1) }
    }

    private var buildButtonLabel: String {
        "BUILD \(DocKinds.primaryKind(for: model.trade.key).uppercased())"
    }

    private var itemCountLabel: String {
        "\(notes.items.count) ITEM\(notes.items.count == 1 ? "" : "S")"
    }
}
