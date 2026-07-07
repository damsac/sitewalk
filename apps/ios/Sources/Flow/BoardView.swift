import SwiftUI

// Live jobs board — the app's home. Trade is switchable from the business
// name (validation strategy: watch which template operators react to).

struct BoardView: View {
    @Bindable var model: AppModel
    // sac: entry point + presentation (sheet vs. a new AppModel.Phase) is your
    // call; a gear → .sheet is a functional default, not a design decision.
    @State private var showVocabulary = false

    var body: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 3) {
                HStack(alignment: .top) {
                    Text(model.trade.dateLabel)
                        .font(Theme.F.mono(10, .semibold))
                        .tracking(2.0)
                        .foregroundStyle(Theme.C.orangeDeep)
                    Spacer()
                    // Vocabulary entry: a stamped chip in the tag grammar —
                    // this is a field tool, not settings, so no gear. Padding
                    // widens the tap target without growing the stamp.
                    Button { showVocabulary = true } label: {
                        Text("VOCAB")
                            .font(Theme.F.mono(8, .semibold))
                            .tracking(1.0)
                            .foregroundStyle(Theme.C.ink60)
                            .padding(.horizontal, 6)
                            .padding(.top, 3)
                            .padding(.bottom, 2)
                            .background(Theme.C.paperDeep)
                            .padding(6)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .padding(-6)
                }
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
        .sheet(isPresented: $showVocabulary) {
            VocabularyView(model: model)
                .presentationDetents([.large])
                .presentationDragIndicator(.visible)
                .presentationBackground(Theme.C.paper)
        }
    }
}
