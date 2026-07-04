import SwiftUI

// Screen 01 — Jobs board. One line per site, airport-board discipline.
// 62pt primary action in the thumb zone.

struct JobsBoardScreen: View {
    let trade: TradeFixture

    var body: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 3) {
                Text(trade.dateLabel)
                    .font(Theme.F.mono(10, .semibold))
                    .tracking(2.0)
                    .foregroundStyle(Theme.C.orangeDeep)
                Text(trade.countTitle)
                    .font(Theme.F.ui(26, .bold))
                Text(trade.bizCaps)
                    .font(Theme.F.mono(9.5))
                    .tracking(0.8)
                    .foregroundStyle(Theme.C.ink60)
                    .padding(.top, 1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, Theme.S.screenPad)
            .padding(.top, 14)
            .padding(.bottom, 12)

            MetaStrip(left: trade.boardMeta, right: "SYNCED 07:58")

            SectionHead(left: "TODAY", right: trade.openLabel, rightColor: Theme.C.orangeDeep)

            ForEach(trade.jobs) { job in
                JobRow(job: job)
            }

            Spacer(minLength: 0)

            WalkButton()
                .padding(.horizontal, Theme.S.screenPad)
                .padding(.bottom, 10)
        }
        .background(Theme.C.paper.ignoresSafeArea())
        .toolbar(.hidden, for: .navigationBar)
    }
}
