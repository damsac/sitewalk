import SwiftUI

// Screen 02 — Capture. State readable at arm's length: orange banner,
// live transcript, items ticking onto the board. Offline never loses audio.

struct CaptureScreen: View {
    let trade: TradeFixture

    var body: some View {
        VStack(spacing: 0) {
            RecBanner(timer: "00:47")

            MetaStrip(left: trade.site, right: "NO BARS — SAVED LOCAL", warn: true)

            VStack(alignment: .leading, spacing: 6) {
                Text(trade.transcript)
                    .font(Theme.F.mono(11.5))
                    .foregroundStyle(Theme.C.ink60)
                    .lineSpacing(7)
                Caret(height: 13)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, Theme.S.screenPad)
            .padding(.top, 14)
            .padding(.bottom, 10)

            SectionHead(left: "CAPTURED", right: trade.capturedCount)
                .overlay(alignment: .top) { Theme.C.ink.frame(height: 1.5) }

            ForEach(trade.captured) { item in
                CapturedRow(item: item)
            }

            Spacer(minLength: 0)

            VStack(spacing: 12) {
                Waveform()
                CaptureControls()
            }
            .padding(.horizontal, Theme.S.screenPad)
            .padding(.top, 10)
            .padding(.bottom, 10)
            .overlay(alignment: .top) { Theme.C.hairline.frame(height: 1) }
        }
        .background(Theme.C.paper.ignoresSafeArea())
        .toolbar(.hidden, for: .navigationBar)
    }
}
