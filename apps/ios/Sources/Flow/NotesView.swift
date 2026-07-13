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

    private var emptyNotes: NotesModel { NotesModel(summary: "", items: [], docKind: "report", queued: false) }
    private var notes: NotesModel { model.notes ?? emptyNotes }
    private var kinds: [String] { DocKinds.legalKinds(for: model.trade.key) }

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
                        } else {
                            if !notes.items.isEmpty {
                                ForEach(grouped, id: \.0) { kind, items in
                                    SectionHead(left: sectionTitle(kind), right: "\(items.count)", heavyRule: false)
                                        .padding(.top, 4)
                                    ForEach(items) { CapturedRow(item: $0) }
                                }
                            }
                            transcriptRow
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
                .foregroundStyle(Theme.C.orangeDeep)
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
                        Text("SAVED OFFLINE — DOCUMENTS UNLOCK WHEN YOU RECONNECT")
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
            RoundedRectangle(cornerRadius: 2).fill(Theme.C.paperDeep)
                .frame(height: 78)
                .padding(.horizontal, Theme.S.screenPad).padding(.top, 14)
            ForEach(0..<3, id: \.self) { _ in
                HStack(spacing: 10) {
                    RoundedRectangle(cornerRadius: 2).fill(Theme.C.paperDeep).frame(width: 44, height: 16)
                    VStack(alignment: .leading, spacing: 5) {
                        RoundedRectangle(cornerRadius: 2).fill(Theme.C.paperDeep).frame(height: 12).frame(maxWidth: .infinity)
                        RoundedRectangle(cornerRadius: 2).fill(Theme.C.paperDeep).frame(height: 9).frame(maxWidth: 180)
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
            .foregroundStyle(Theme.C.ink35)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 22)
            .overlay(RoundedRectangle(cornerRadius: 4)
                .stroke(style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
                .foregroundStyle(Theme.C.ink35))
            .padding(Theme.S.screenPad)
    }

    // MARK: Transcript (collapsed)

    private var transcriptRow: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button { withAnimation(.easeOut(duration: 0.2)) { showTranscript.toggle() } } label: {
                HStack {
                    Text("SHOW WHAT I HEARD — FULL TRANSCRIPT")
                        .font(Theme.F.mono(8.5, .semibold)).tracking(0.8)
                        .foregroundStyle(Theme.C.ink60)
                    Spacer()
                    Text(showTranscript ? "▾" : "▸")
                        .font(Theme.F.mono(9)).foregroundStyle(Theme.C.orangeDeep)
                }
                .padding(9)
                .overlay(RoundedRectangle(cornerRadius: 3)
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

    // MARK: Action bar — the differentiation, made visible

    private var actionBar: some View {
        VStack(alignment: .leading, spacing: 9) {
            Text("TURN THESE NOTES INTO")
                .font(Theme.F.mono(8.5, .semibold)).tracking(1.8)
                .foregroundStyle(Theme.C.ink60)
            HStack(spacing: 8) {
                ForEach(Array(kinds.enumerated()), id: \.element) { i, kind in
                    docButton(kind, hero: i == 0)
                }
            }
            Button { exportNotes() } label: {
                Text("⇪  EXPORT NOTES")
                    .font(Theme.F.mono(9, .semibold)).tracking(1.0)
                    .foregroundStyle(Theme.C.ink60)
                    .frame(maxWidth: .infinity).frame(height: 40)
                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(Theme.C.hairline, lineWidth: 1.5))
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 11).padding(.bottom, 10)
        .background(Theme.C.paper)
        .overlay(alignment: .top) { Theme.C.ink.frame(height: 1.5) }
    }

    private func docButton(_ kind: String, hero: Bool) -> some View {
        let building = model.buildingKind == kind
        let disabled = notes.queued || (model.isBuildingDocument && !building)
        return Button { model.buildDocument(kind: kind) } label: {
            ZStack {
                if hero {
                    RoundedRectangle(cornerRadius: Theme.S.radius).fill(Theme.C.orangeDeep).offset(y: 3)
                    RoundedRectangle(cornerRadius: Theme.S.radius).fill(Theme.C.orange)
                } else {
                    RoundedRectangle(cornerRadius: Theme.S.radius).stroke(Theme.C.ink, lineWidth: 2)
                }
                if building {
                    ProgressView().tint(hero ? Theme.C.onOrange : Theme.C.ink)
                } else {
                    VStack(spacing: 2) {
                        Text(DocKinds.label(for: kind).uppercased())
                            .font(Theme.F.ui(12, .bold)).tracking(0.04)
                            .foregroundStyle(hero ? Theme.C.onOrange : Theme.C.ink)
                        Text(DocKinds.stamp(for: kind))
                            .font(Theme.F.mono(6.5, .semibold)).tracking(0.6)
                            .foregroundStyle(hero ? Theme.C.onOrange.opacity(0.85) : Theme.C.ink60)
                    }
                }
            }
            .frame(height: 54).frame(maxWidth: .infinity)
        }
        .buttonStyle(.plain)
        .disabled(disabled)
        .opacity(disabled ? 0.4 : 1)
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
