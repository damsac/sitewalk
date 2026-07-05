import SwiftUI

// The transformation beat — speech dims, the paper rises. Deliberately a
// restrained version; the full showpiece motion pass comes later.

struct BuildView: View {
    @Bindable var model: AppModel
    @State private var progress = 0
    @State private var risen = false

    private let blocks = 8

    var body: some View {
        VStack(spacing: 0) {
            Text(model.transcript)
                .font(Theme.F.mono(10.5))
                .foregroundStyle(Theme.C.ink35)
                .lineSpacing(6)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, Theme.S.screenPad)
                .padding(.vertical, 14)
                .frame(height: 120, alignment: .bottom)
                .clipped()

            HStack {
                Text("BUILDING \(model.trade.docKind)")
                    .font(Theme.F.mono(10, .semibold))
                    .tracking(1.4)
                    .foregroundStyle(Theme.C.paper)
                Spacer()
                Text(String(repeating: "▓", count: progress) + String(repeating: "░", count: blocks - progress))
                    .font(Theme.F.mono(10, .semibold))
                    .foregroundStyle(Theme.C.orange)
            }
            .padding(.horizontal, Theme.S.screenPad)
            .padding(.vertical, 11)
            .background(Theme.C.ink)

            ZStack(alignment: .top) {
                Theme.C.paperDeep
                VStack(alignment: .leading, spacing: 0) {
                    Letterhead(
                        biz: model.trade.biz,
                        bizSub: model.trade.bizSub,
                        docKind: model.trade.docKind,
                        docNo: model.trade.docNo,
                        docDate: model.trade.docDate
                    )
                    ForEach(Array(model.trade.rows.prefix(3).enumerated()), id: \.element.id) { index, row in
                        DocRowView(row: row)
                            .opacity(progress > (index + 2) ? 1 : 0)
                    }
                }
                .padding(18)
                .background(Theme.C.sheet)
                .compositingGroup()
                .shadow(color: Theme.C.ink.opacity(0.15), radius: 12, y: 8)
                .padding(.horizontal, 16)
                .padding(.top, risen ? 22 : 90)
                .animation(.easeOut(duration: 0.7), value: risen)
            }
        }
        .background(Theme.C.paper.ignoresSafeArea())
        .toolbar(.hidden, for: .navigationBar)
        .navigationBarBackButtonHidden(true)
        .task {
            risen = true
            for step in 1...blocks {
                try? await Task.sleep(for: .milliseconds(190))
                progress = step
            }
        }
    }
}
