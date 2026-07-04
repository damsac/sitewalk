import SwiftUI
import UIKit

// Component gallery — every piece of the kit in every state, plus a
// font-registration footer so a screenshot proves the brand faces loaded.

struct ComponentsPage: View {
    private let land = Fixtures.landscape
    private let prop = Fixtures.property
    private let insp = Fixtures.inspection

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                section("FIELD TAGS")
                HStack(spacing: 8) {
                    FieldTag(tag: TagFixture(kind: .red, label: "SAFETY"))
                    FieldTag(tag: TagFixture(kind: .yellow, label: "F/U"))
                    FieldTag(tag: TagFixture(kind: .green, label: "SENT"))
                    FieldTag(tag: TagFixture(kind: .plain, label: "ITEM"))
                    PhotoChip(count: 2)
                    GapChip(count: 1)
                }
                .padding(.horizontal, Theme.S.screenPad)
                .padding(.bottom, 18)

                section("JOB ROWS — OPEN / DONE")
                JobRow(job: land.jobs[1])
                JobRow(job: land.jobs[0])

                section("LIVE BOARD ROWS — TAG / PHOTO PIN")
                CapturedRow(item: prop.captured[0])
                CapturedRow(item: insp.captured[2])

                section("DOC ROWS — NORMAL / EDIT+HINT / GAP")
                VStack(spacing: 0) {
                    DocRowView(row: land.rows[0])
                    DocRowView(row: land.rows[2])
                    DocRowView(row: land.rows[4])
                    TotalRow(key: land.totalKey, value: land.totalValue, gaps: 1)
                        .padding(.top, 2)
                }
                .padding(.horizontal, Theme.S.screenPad)
                .padding(.bottom, 18)

                section("REVIEW NOTE")
                RevNote(text: land.note)
                    .padding(.horizontal, Theme.S.screenPad)
                    .padding(.bottom, 18)

                section("META STRIPS — SYNCED / OFFLINE")
                MetaStrip(left: land.boardMeta, right: "SYNCED 07:58")
                MetaStrip(left: land.site, right: "NO BARS — SAVED LOCAL", warn: true)
                    .padding(.bottom, 18)

                section("REC BANNER")
                RecBanner(timer: "00:47")
                    .padding(.bottom, 18)

                section("WAVEFORM")
                Waveform()
                    .padding(.horizontal, Theme.S.screenPad)
                    .padding(.bottom, 18)

                section("CONTROLS — 62PT GLOVE TARGETS")
                VStack(spacing: 10) {
                    WalkButton()
                    CaptureControls()
                    ReviewBar(sendTitle: land.send)
                }
                .padding(.horizontal, Theme.S.screenPad)
                .padding(.bottom, 24)

                fontFooter
            }
            .padding(.top, 8)
        }
        .background(Theme.C.paper.ignoresSafeArea())
        .toolbar(.hidden, for: .navigationBar)
    }

    private func section(_ title: String) -> some View {
        SectionHead(left: title, right: "", heavyRule: false)
            .padding(.bottom, 10)
    }

    // Proof-of-load: lists the registered brand faces. If a family is
    // missing here, the app silently fell back to system fonts.
    private var fontFooter: some View {
        let families = ["Barlow", "Barlow Semi Condensed", "IBM Plex Mono", "Source Serif 4"]
        let lines = families.map { fam -> String in
            let faces = UIFont.fontNames(forFamilyName: fam)
            return faces.isEmpty ? "\(fam): MISSING" : "\(fam): \(faces.count) faces"
        }
        return VStack(alignment: .leading, spacing: 3) {
            SectionLabel("FONTS LOADED")
            ForEach(lines, id: \.self) { line in
                Text(line)
                    .font(Theme.F.mono(8))
                    .foregroundStyle(Theme.C.ink60)
            }
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.bottom, 30)
    }
}
