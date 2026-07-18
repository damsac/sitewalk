import SwiftUI
import UIKit

// Live jobs board — the app's home. Trade is switchable from the business
// name (validation strategy: watch which template operators react to).

struct BoardView: View {
    @Bindable var model: AppModel
    // sac: entry point + presentation (sheet vs. a new AppModel.Phase) is your
    // call; a gear → .sheet is a functional default, not a design decision.
    @State private var showVocabulary = false
    @State private var showLetterhead = false
    // First-run coach mark, one-shot (survives relaunch). Cleared by resetcoach=1.
    @AppStorage(CoachMarks.startWalkKey) private var coachStartShown = false

    var body: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 3) {
                HStack(alignment: .top) {
                    // Real current date once a profile exists; the frozen
                    // fixture date only survives on the no-profile demo path.
                    Text(model.boardDateLabel)
                        .font(Theme.F.mono(10, .semibold))
                        .tracking(2.0)
                        .foregroundStyle(Theme.C.orangeDeep)
                    Spacer()
                    // Input-mode chip: VOICE (the product) vs DEMO (the canned
                    // walk — graduates into onboarding). Tap to toggle;
                    // launch-arg-forced modes lock it.
                    Button { model.toggleMode() } label: {
                        Text(model.walkMode == .voice ? "MIC · VOICE" : "DEMO WALK")
                            .font(Theme.F.mono(8, .semibold))
                            .tracking(1.0)
                            .foregroundStyle(model.walkMode == .voice ? Theme.C.greenTag : Theme.C.yellowTag)
                            .padding(.horizontal, 6)
                            .padding(.top, 3)
                            .padding(.bottom, 2)
                            .background(model.walkMode == .voice ? Theme.C.greenTint : Theme.C.yellowTint)
                            .padding(6)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .padding(-6)
                    .opacity(model.modeLocked ? 0.5 : 1)
                    .disabled(model.modeLocked)
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
                    // Letterhead / paperwork branding — same stamp grammar.
                    Button { showLetterhead = true } label: {
                        Text("PAPER")
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
                if let profile = model.profile {
                    // Operator mode: the board carries THEIR business. Trade
                    // comes from the profile, so no switcher — plain text.
                    Text(model.sessionTitle)
                        .font(Theme.F.ui(26, .bold))
                    Text(profile.businessName.uppercased())
                        .font(Theme.F.mono(9.5))
                        .tracking(0.8)
                        .foregroundStyle(Theme.C.ink60)
                        .lineLimit(1)
                        .padding(.top, 1)
                } else {
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
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, Theme.S.screenPad)
            .padding(.top, 14)
            .padding(.bottom, 12)

            if model.micDenied {
                // A voice walk was attempted with mic permission denied.
                // Same red-note grammar as the photo error bar; tapping goes
                // straight to the app's Settings pane.
                Button {
                    if let url = URL(string: UIApplication.openSettingsURLString) {
                        UIApplication.shared.open(url)
                    }
                } label: {
                    HStack(spacing: 0) {
                        Theme.C.redTag.frame(width: 3)
                        Text("MIC IS OFF — SITEWALK CAN'T HEAR YOUR WALK. TAP TO ENABLE IN SETTINGS")
                            .font(Theme.F.mono(8, .semibold))
                            .tracking(0.4)
                            .foregroundStyle(Theme.C.redTag)
                            .padding(.horizontal, 9)
                            .padding(.vertical, 6)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .background(Theme.C.redTint)
                }
                .buttonStyle(.plain)
                .padding(.horizontal, Theme.S.screenPad)
                .padding(.bottom, 10)
            }

            if model.profile != nil {
                // Operator mode: no fixture crew/sync strip, no fixture jobs.
                // The board logs the walks actually finished this session.
                SectionHead(
                    left: "TODAY",
                    right: "\(model.sessionWalks.count) \(model.sessionWalks.count == 1 ? "WALK" : "WALKS")",
                    rightColor: Theme.C.orangeDeep
                )

                if model.sessionWalks.isEmpty {
                    // Honest empty state, same dashed-box idiom as the
                    // vocabulary editor's.
                    Text("NO WALKS YET — TAP START WALK")
                        .font(Theme.F.mono(8.5))
                        .tracking(0.8)
                        .foregroundStyle(Theme.C.ink35)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 26)
                        .overlay(
                            RoundedRectangle(cornerRadius: 4)
                                .stroke(style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
                                .foregroundStyle(Theme.C.ink35)
                        )
                        .padding(.horizontal, Theme.S.screenPad)
                        .padding(.top, 16)
                } else {
                    ForEach(model.sessionWalks) { walk in
                        WalkLogRow(walk: walk)
                    }
                }
            } else {
                MetaStrip(left: model.trade.boardMeta, right: "SYNCED 07:58")

                SectionHead(
                    left: "TODAY",
                    right: "\(model.jobs.filter { !$0.done }.count) OPEN",
                    rightColor: Theme.C.orangeDeep
                )

                ForEach(model.jobs) { job in
                    JobRow(job: job)
                }
            }

            Spacer(minLength: 0)

            // First-run coach mark: point a brand-new operator at the one thing
            // to do. Only on a fresh board (profile set, no walks yet); the
            // START button below stays fully tappable (non-blocking hint).
            if !coachStartShown && model.profile != nil && model.sessionWalks.isEmpty {
                CoachCallout(text: "Ready? Tap START WALK and just talk — walk the job like you're telling a helper.") {
                    coachStartShown = true
                }
                .padding(.horizontal, Theme.S.screenPad)
                .padding(.bottom, 4)
                .transition(.opacity)
            }

            Button {
                coachStartShown = true
                model.startWalk()
            } label: {
                WalkButton()
            }
            .buttonStyle(.plain)
            .padding(.horizontal, Theme.S.screenPad)
            .padding(.bottom, 10)
        }
        .animation(.easeOut(duration: 0.25), value: coachStartShown)
        .background(Theme.C.paper.ignoresSafeArea())
        .toolbar(.hidden, for: .navigationBar)
        .sheet(isPresented: $showVocabulary) {
            VocabularyView(model: model)
                .presentationDetents([.large])
                .presentationDragIndicator(.visible)
                .presentationBackground(Theme.C.paper)
        }
        .sheet(isPresented: $showLetterhead) {
            LetterheadStudioView(model: model)
                .presentationDetents([.large])
                .presentationDragIndicator(.visible)
                .presentationBackground(Theme.C.paper)
        }
    }
}

// MARK: - Coach marks (first-run hints)

/// Persisted one-shot flags for the first-run coach marks. Centralized so the
/// GalleryApp QA hooks (resetcoach / autoflow) and the call sites agree.
enum CoachMarks {
    static let startWalkKey = "coach.startWalk.shown"
    static let doneKey = "coach.done.shown"
    static let allKeys = [startWalkKey, doneKey]
}

/// A soft amber callout that points at the button directly beneath it. Chosen
/// over a dark spotlight overlay on purpose: it stays in the field-instrument
/// grammar (paper + amber, not a flashy tour) and it's non-blocking — the
/// target button underneath stays tappable, so it never traps the flow. One-
/// shot gating lives at the call site (an @AppStorage flag).
struct CoachCallout: View {
    let text: String
    /// Where the downward caret sits, so it aims at the real target (a full-
    /// width button → .center; DONE in a control row → .trailing).
    var pointer: Alignment = .center
    var dismiss: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Text(text)
                    .font(Theme.F.cond(13.5, .semibold))
                    .foregroundStyle(Theme.C.ink)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Button(action: dismiss) {
                    Text("GOT IT")
                        .font(Theme.F.mono(9, .semibold))
                        .tracking(1.0)
                        .foregroundStyle(Theme.C.orangeDeep)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            .padding(.horizontal, 13)
            .padding(.vertical, 11)
            .background(Theme.C.orangeTint)
            .overlay(Rectangle().stroke(Theme.C.orange, lineWidth: 1.5))

            Text("▾")
                .font(Theme.F.ui(15, .bold))
                .foregroundStyle(Theme.C.orange)
                .frame(maxWidth: .infinity, alignment: pointer)
                .padding(.horizontal, 34)
                .offset(y: -2)
        }
    }
}

// MARK: - Session walk row (airport-board discipline, JobRow's bones)

private struct WalkLogRow: View {
    let walk: AppModel.WalkRecord

    var body: some View {
        HStack(spacing: 12) {
            Text(walk.time)
                .font(Theme.F.mono(11, .medium))
                .foregroundStyle(Theme.C.ink)
                .frame(width: 46, alignment: .leading)
            VStack(alignment: .leading, spacing: 1) {
                Text(walk.docNo)
                    .font(Theme.F.ui(14.5, .semibold))
                    .lineLimit(1)
                Text(walk.docKind.capitalized)
                    .font(Theme.F.cond(11.5, .medium))
                    .foregroundStyle(Theme.C.ink60)
                    .lineLimit(1)
            }
            Spacer(minLength: 8)
            FieldTag(tag: TagFixture(
                kind: walk.sent ? .green : .plain,
                label: walk.sent ? "SENT" : "DISCARDED"
            ))
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.vertical, 13)
        .opacity(walk.sent ? 1 : 0.55)
        .overlay(alignment: .bottom) { Theme.C.hairline.frame(height: 1) }
    }
}
