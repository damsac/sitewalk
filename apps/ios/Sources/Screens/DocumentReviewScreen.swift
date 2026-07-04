import SwiftUI

// Screen 04 — Document review. It looks like paper because paper is the
// pitch. Unheard values stay gaps, never guesses. Send is one thumb away.

struct DocumentReviewScreen: View {
    let trade: TradeFixture

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                DocumentSheet(trade: trade)
                    .padding(.horizontal, 14)
                    .padding(.top, 14)
                    .padding(.bottom, 20)
            }
            .background(Theme.C.paperDeep)

            ReviewBar(sendTitle: trade.send)
                .padding(.horizontal, Theme.S.screenPad)
                .padding(.top, 12)
                .padding(.bottom, 10)
                .background(Theme.C.paper)
                .overlay(alignment: .top) { Theme.C.hairline.frame(height: 1) }
        }
        .background(Theme.C.paperDeep.ignoresSafeArea())
        .toolbar(.hidden, for: .navigationBar)
    }
}
