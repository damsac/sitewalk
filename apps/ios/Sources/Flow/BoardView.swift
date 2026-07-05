import SwiftUI

// Live jobs board — the app's home. Trade is switchable from the business
// name (validation strategy: watch which template operators react to).

struct BoardView: View {
    @Bindable var model: AppModel

    var body: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 3) {
                Text(model.trade.dateLabel)
                    .font(Theme.F.mono(10, .semibold))
                    .tracking(2.0)
                    .foregroundStyle(Theme.C.orangeDeep)
                Text(model.trade.countTitle)
                    .font(Theme.F.ui(26, .bold))
                Menu {
                    ForEach(Fixtures.all, id: \.key) { trade in
                        Button(trade.biz) { model.switchTrade(trade) }
                    }
                } label: {
                    HStack(spacing: 6) {
                        Text(model.trade.bizCaps)
                            .font(Theme.F.mono(9.5))
                            .tracking(0.8)
                        Text("⌄")
                            .font(Theme.F.mono(9))
                    }
                    .foregroundStyle(Theme.C.ink60)
                }
                .padding(.top, 1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, Theme.S.screenPad)
            .padding(.top, 14)
            .padding(.bottom, 12)

            MetaStrip(left: model.trade.boardMeta, right: "SYNCED 07:58")

            SectionHead(
                left: "TODAY",
                right: "\(model.jobs.filter { !$0.done }.count) OPEN",
                rightColor: Theme.C.orangeDeep
            )

            ForEach(model.jobs) { job in
                JobRow(job: job)
            }

            Spacer(minLength: 0)

            Button {
                model.startWalk()
            } label: {
                WalkButton()
            }
            .buttonStyle(.plain)
            .padding(.horizontal, Theme.S.screenPad)
            .padding(.bottom, 10)
        }
        .background(Theme.C.paper.ignoresSafeArea())
        .toolbar(.hidden, for: .navigationBar)
    }
}
