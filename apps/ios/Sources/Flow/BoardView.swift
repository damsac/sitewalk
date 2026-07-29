import SwiftUI
import UIKit

// Live jobs board — the app's home. Trade is switchable from the business
// name (validation strategy: watch which template operators react to).

struct BoardView: View {
    @Bindable var model: AppModel
    // sac: entry point + presentation (sheet vs. a new AppModel.Phase) is your
    // call; a gear → .sheet is a functional default, not a design decision.
    @State private var showCustomize = false
    // First-run coach mark, one-shot (survives relaunch). Cleared by resetcoach=1.
    @AppStorage(CoachMarks.startWalkKey) private var coachStartShown = false

    // Jobs. Held here rather than on AppModel because nothing outside the board
    // reads them yet; promote when walk-attachment lands and the notes screen
    // needs the list too.
    @State private var operatorJobs: [JobModel] = []
    @State private var jobsError: String?
    @State private var showNewJob = false
    @State private var newJobName = ""
    /// TODAY collapses so JOBS clears the fold. Expanding shows the rest.
    @State private var showAllWalks = false
    /// Apple's own manage-subscriptions sheet, raised by the PRO chip.
    @State private var showManageSubscription = false
    /// Enough to see today's work at a glance without pushing jobs off
    /// screen — the complaint that prompted this.
    private static let collapsedWalkCount = 3

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
                    // Practice-run marker: the armed dry run is visible on the
                    // board (the old mode chip carried this; the chip is gone —
                    // per Isaac, input mode is voice-only for users — so the
                    // marker survives as a non-interactive stamp).
                    if model.isPracticeWalk {
                        Text("PRACTICE")
                            .font(Theme.F.mono(8, .semibold))
                            .tracking(1.0)
                            .foregroundStyle(Theme.C.yellowTag)
                            .padding(.horizontal, 6)
                            .padding(.top, 3)
                            .padding(.bottom, 2)
                            .background(Theme.C.yellowTint)
                    }
                    // Input mode is voice-only for users; the DEMO toggle was a
                    // dev affordance (still reachable via the `demo=1` launch arg
                    // for QA/screenshots) — removed from the board per Isaac.
                    // One clear entry for making the app yours — replaces two
                    // cryptic chips (VOCAB + PAPER). A raised amber block (same
                    // pressed-block grammar as START WALK) so it reads as a
                    // button; opens the two-tab PAPERWORK / WORDS sheet.
                    planChip
                    Button { showCustomize = true } label: {
                        raisedChip(face: Theme.C.orange, edge: Theme.C.orangeDeep) {
                            Text("MY BUSINESS")
                                .font(Theme.F.ui(11, .bold))
                                .tracking(0.5)
                                .foregroundStyle(Theme.C.onOrange)
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
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

            // The board had NO ScrollView: a plain VStack that simply ran out
            // of room, which is why jobs ended up crammed against the bottom
            // (Isaac). Header and START WALK stay pinned — the button is the
            // one control that must never require scrolling to reach — and
            // everything between them scrolls.
            ScrollView {
              VStack(spacing: 0) {
                if model.profile != nil {
                // Operator mode: no fixture crew/sync strip, no fixture jobs.
                // The board logs the walks actually finished this session.
                //
                // Only UNFILED walks show here, grouped by day. Operator report
                // 2026-07-27 ("random walks"): the old top list was a single
                // hardcoded "TODAY" that actually showed the ENTIRE history
                // newest-first, AND filed walks appeared twice (loose up top and
                // under their job card). Now a walk lives in exactly one place —
                // loose here until filed, then under its job alone — and the
                // loose list is honestly labelled TODAY / EARLIER by date.
                if model.sessionWalks.isEmpty {
                    // Fresh board — no walks at all yet.
                    SectionHead(left: "TODAY", right: "0 WALKS", rightColor: Theme.C.orangeDeep)
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
                } else if model.looseWalks.isEmpty {
                    // Walks exist but every one is filed under a job below —
                    // nothing loose to list. Say so rather than showing an empty
                    // "TODAY"; this is also filing's payoff ("it moved under the
                    // job").
                    SectionHead(left: "UNFILED", right: "0 WALKS", rightColor: Theme.C.orangeDeep)
                    Text("ALL WALKS FILED — SEE JOBS BELOW")
                        .font(Theme.F.mono(8.5))
                        .tracking(0.8)
                        .foregroundStyle(Theme.C.ink35)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 22)
                        .overlay(
                            RoundedRectangle(cornerRadius: 4)
                                .stroke(style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
                                .foregroundStyle(Theme.C.ink35)
                        )
                        .padding(.horizontal, Theme.S.screenPad)
                        .padding(.top, 16)
                } else {
                    // Plan 20 D5: rows are tappable — reopen the walk's notes.
                    // // sac: the reopen affordance visuals (chevron? row
                    // // sac: press state?) + the reopened banner are yours.
                    // One SectionHead per non-empty date group (TODAY / EARLIER),
                    // over the CAPPED list: collapsing and date-grouping compose
                    // in that order, so the collapsed view shows the 3 newest
                    // unfiled walks (in practice all TODAY) and expanding reveals
                    // the older ones under their own head.
                    ForEach(visibleSections) { section in
                        SectionHead(
                            left: section.title,
                            right: "\(section.walks.count) \(section.walks.count == 1 ? "WALK" : "WALKS")",
                            rightColor: Theme.C.orangeDeep
                        )
                        ForEach(section.walks) { walk in
                            WalkLogRow(walk: walk) {
                                model.reopenWalk(sessionId: walk.sessionId)
                            } trailing: {
                                FileChip(walk: walk, jobs: activeJobs) { jobId in
                                    fileWalk(walk, to: jobId)
                                }
                            }
                        }
                    }
                    if model.looseWalks.count > Self.collapsedWalkCount {
                        Button { showAllWalks.toggle() } label: {
                            Text(showAllWalks
                                 ? "SHOW LESS"
                                 : "SHOW ALL \(model.looseWalks.count) WALKS")
                                .font(Theme.F.mono(8.5, .semibold))
                                .tracking(1.0)
                                .foregroundStyle(Theme.C.orangeDeep)
                                .padding(.horizontal, Theme.S.screenPad)
                                .padding(.vertical, 9)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .overlay(alignment: .bottom) {
                            Rectangle().fill(Theme.C.hairlineSoft).frame(height: 1)
                        }
                    }

                    if let reopenError = model.reopenError {
                        // F4 floor: the breadcrumb surfaces; chrome is sac's.
                        Text(reopenError.uppercased())
                            .font(Theme.F.mono(8.5))
                            .tracking(0.4)
                            .foregroundStyle(Theme.C.orangeDeep)
                            .padding(.horizontal, Theme.S.screenPad)
                            .padding(.top, 8)
                    }
                }

                // JOBS — the board's organizing unit, below the walk log on purpose.
                // Walk-first (Isaac, 07-26): START WALK stays instant and
                // unblocked at the truck door; jobs are where work accumulates,
                // not a gate in front of recording it.
                jobsSection
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

              }
            }
            .scrollBounceBehavior(.basedOnSize)

            // First-run coach mark: point a brand-new operator at the one thing
            // to do. Only on a fresh board (profile set, no walks yet); the
            // START button below stays fully tappable (non-blocking hint).
            if (!coachStartShown || model.isPracticeWalk) && model.profile != nil && model.sessionWalks.isEmpty {
                CoachCallout(text: model.isPracticeWalk
                    ? "This is a practice run — nothing gets saved. Tap START WALK and try talking through a job."
                    : "Ready? Tap START WALK and just talk — walk the job like you're telling a helper.") {
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
        .sheet(isPresented: $showCustomize) {
            CustomizeView(model: model)
                .presentationDetents([.large])
                .presentationDragIndicator(.visible)
                .presentationBackground(Theme.C.paper)
        }
        // Raised by `startWalk()` refusing at the free-tier limit, and by the
        // deliberate upgrade affordance in Customize. Never raised by a
        // background event, so it can't appear over a walk in progress.
        .sheet(isPresented: $model.showPaywall) {
            PaywallView(model: model, blocked: model.blockedUsage)
                .presentationDetents([.large])
                .presentationDragIndicator(.visible)
                .presentationBackground(Theme.C.paper)
        }
        .manageSubscriptionsSheet(isPresented: $showManageSubscription)
    }
}

// MARK: - Raised chip (clickable header buttons)

// MARK: - Jobs section

extension BoardView {
    /// The jobs list plus its create affordance.
    ///
    /// Only ACTIVE jobs show. A contractor juggling ten live jobs does not want
    /// last spring's finished work in the way; done/archived stay in the model
    /// (and sync) but are out of the daily view.
    var jobsSection: some View {
        VStack(alignment: .leading, spacing: 0) {
            // SectionHead's grammar, composed inline rather than used directly:
            // SectionHead draws its own full-width bottom rule, so putting the
            // + beside it as a sibling shrinks the head and truncates the rule.
            // Same padding, same labels, same heavy rule — one row.
            HStack(spacing: 10) {
                SectionLabel("JOBS")
                Spacer()
                SectionLabel("\(activeJobs.count) OPEN", color: Theme.C.orangeDeep)
                // Labeled, not a bare glyph: Isaac's field report was "the plus
                // sign to add a new job isn't apparent enough." A lone + next
                // to a count reads as decoration. Words and a border make it
                // read as a control — the same grammar the Document Builder's
                // "NEW DOCUMENT TYPE" already uses.
                Button { showNewJob = true } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "plus")
                            .font(.system(size: 10, weight: .bold))
                        Text("ADD JOB")
                            .font(Theme.F.mono(9, .semibold))
                            .tracking(0.8)
                    }
                    .foregroundStyle(Theme.C.orangeDeep)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 6)
                    .overlay(
                        RoundedRectangle(cornerRadius: 3)
                            .stroke(Theme.C.orangeDeep.opacity(0.45), lineWidth: 1)
                    )
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Add a new job")
            }
            .padding(.leading, Theme.S.screenPad)
            .padding(.trailing, Theme.S.screenPad - 8)
            .padding(.top, 10)
            .padding(.bottom, 4)
            .overlay(alignment: .bottom) {
                Theme.C.ink.frame(height: 1.5)
            }

            if let jobsError {
                Text(jobsError.uppercased())
                    .font(Theme.F.mono(8.5))
                    .tracking(0.4)
                    .foregroundStyle(Theme.C.redTag)
                    .padding(.horizontal, Theme.S.screenPad)
                    .padding(.top, 8)
            }

            if activeJobs.isEmpty {
                Text("NO JOBS YET — TAP + TO ADD ONE")
                    .font(Theme.F.mono(8.5))
                    .tracking(0.8)
                    .foregroundStyle(Theme.C.ink35)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 22)
                    .overlay(
                        RoundedRectangle(cornerRadius: 4)
                            .stroke(style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
                            .foregroundStyle(Theme.C.ink35)
                    )
                    .padding(.horizontal, Theme.S.screenPad)
                    .padding(.top, 12)
            } else {
                ForEach(activeJobs) { job in
                    OperatorJobCard(job: job, walks: walks(for: job.id)) { walk in
                        model.reopenWalk(sessionId: walk.sessionId)
                    }
                }
            }
        }
        .onAppear(perform: loadJobs)
        .alert("New job", isPresented: $showNewJob) {
            TextField("Name", text: $newJobName)
            Button("Cancel", role: .cancel) { newJobName = "" }
            Button("Add") { createJob() }
        } message: {
            Text("Call it whatever you call it on site.")
        }
    }

    var activeJobs: [JobModel] { operatorJobs.filter { $0.status == .active } }

    /// Plan status, and the only always-visible way in and out of billing.
    ///
    /// Free shows walks remaining rather than walks used — "2 LEFT" is what an
    /// operator actually needs to decide whether to start a walk, and it stays
    /// quiet (ink60, no fill) so it reads as a fact rather than a nag. Pro opens
    /// Apple's manage-subscriptions sheet: Apple requires that a subscriber can
    /// reach cancellation easily, and hiding it would be both a review risk and
    /// the kind of thing that makes people distrust a subscription.
    @ViewBuilder
    private var planChip: some View {
        if model.entitlement.isPro {
            Button { showManageSubscription = true } label: {
                Text("PRO")
                    .font(Theme.F.mono(8, .semibold))
                    .tracking(1.0)
                    .foregroundStyle(Theme.C.greenTag)
                    .padding(.horizontal, 6)
                    .padding(.top, 3)
                    .padding(.bottom, 2)
                    .background(Theme.C.greenTint)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        } else if model.entitlement.canSubscribe {
            // Only shown when the limit is actually being enforced. With no
            // purchasable product the gate declines to block (see
            // `WalkAllowance.decide`), so "2 LEFT" would be a lie — and a
            // discouraging one, since nothing runs out.
            let used = WalkAllowance.usage(in: WalkMeter.load(), now: Date())
            let left = max(0, WalkAllowance.freeMonthlyLimit - used)
            Button {
                model.blockedUsage = nil   // opened by choice, not refused
                model.showPaywall = true
            } label: {
                Text(left == 0 ? "0 LEFT" : "\(left) LEFT")
                    .font(Theme.F.mono(8, .semibold))
                    .tracking(1.0)
                    .foregroundStyle(left == 0 ? Theme.C.redTag : Theme.C.ink60)
                    .padding(.horizontal, 6)
                    .padding(.top, 3)
                    .padding(.bottom, 2)
                    .background(left == 0 ? Theme.C.redTint : Theme.C.paperDeep)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    /// The UNFILED walks, capped until expanded.
    ///
    /// Reads `looseWalks`, not `sessionWalks`: a filed walk lives under its job
    /// card and must not also sit loose up top, or it appears twice and the cap
    /// is spent showing a walk that is already visible elsewhere.
    var visibleWalks: [AppModel.WalkRecord] {
        showAllWalks
            ? model.looseWalks
            : Array(model.looseWalks.prefix(Self.collapsedWalkCount))
    }

    /// Those same walks split into honest date groups. Grouping the CAPPED list
    /// (rather than capping each group) keeps one rule — "at most N rows before
    /// you ask for more" — instead of a cap that silently multiplies by however
    /// many days the operator happens to have walked.
    var visibleSections: [AppModel.WalkSection] {
        AppModel.groupWalksByDay(visibleWalks, now: Date(), calendar: .current)
    }

    func loadJobs() {
        do {
            operatorJobs = try model.engine.listJobs()
            jobsError = nil
        } catch {
            // Leave whatever is on screen rather than blanking the list — the
            // vocabulary/schema editors' posture.
            jobsError = "Couldn't load jobs: \(error.localizedDescription)"
        }
    }

    /// Files (or unfiles) a walk, then re-reads the log so the job cards
    /// reflect it. Re-reading rather than mutating in place keeps core as the
    /// single source of truth for what is filed where.
    func fileWalk(_ walk: AppModel.WalkRecord, to jobId: String?) {
        guard !walk.sessionId.isEmpty else { return }  // legacy/demo row
        do {
            try model.setWalkJob(sessionId: walk.sessionId, jobId: jobId)
            jobsError = nil
        } catch {
            jobsError = "Couldn't file walk: \(error.localizedDescription)"
        }
    }

    /// The walks filed under a job, newest first — the "months later, what
    /// happened on this job?" surface.
    func walks(for jobId: String) -> [AppModel.WalkRecord] {
        model.sessionWalks.filter { $0.jobId == jobId }
    }

    func createJob() {
        let name = newJobName
        newJobName = ""
        do {
            // Core rejects an empty name rather than coercing it (R6), so the
            // error path is real and worth surfacing rather than pre-guarding
            // into silence.
            let created = try model.engine.createJob(name: name)
            operatorJobs.insert(created, at: 0)
            jobsError = nil
        } catch {
            jobsError = "Couldn't add job: \(error.localizedDescription)"
        }
    }
}

/// One job on the board. Deliberately quiet for now: this is where walks,
/// documents, and notes will hang once walk-attachment and document
/// persistence land, so it's built as a card rather than a row.
private struct OperatorJobCard: View {
    let job: JobModel
    let walks: [AppModel.WalkRecord]
    let onOpenWalk: (AppModel.WalkRecord) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text(job.name)
                    .font(Theme.F.serif(17, .semibold))
                    .foregroundStyle(Theme.C.ink)
                    .lineLimit(2)
                Spacer()
                Text(walkCountLabel)
                    .font(Theme.F.mono(8.5))
                    .tracking(0.8)
                    .foregroundStyle(walks.isEmpty ? Theme.C.ink35 : Theme.C.orangeDeep)
            }
            .padding(.horizontal, Theme.S.screenPad)
            .padding(.top, 13)
            .padding(.bottom, walks.isEmpty ? 13 : 8)

            // The walks themselves — the "an email lands months later asking
            // about this job" surface. Tapping reopens that walk's notes,
            // which are the durable record; the document is regenerated from
            // them on demand rather than stored.
            ForEach(walks) { walk in
                Button { onOpenWalk(walk) } label: {
                    HStack(spacing: 10) {
                        Rectangle()
                            .fill(Theme.C.hairline)
                            .frame(width: 1, height: 13)
                        Text(AppModel.walkDateLabel(epochSeconds: walk.startedAt))
                            .font(Theme.F.mono(9.5))
                            .foregroundStyle(Theme.C.ink60)
                        Text(walk.docKind)
                            .font(Theme.F.mono(9.5))
                            .tracking(0.6)
                            .foregroundStyle(Theme.C.ink35)
                            .lineLimit(1)
                        Spacer()
                        Image(systemName: "chevron.right")
                            .font(.system(size: 9, weight: .semibold))
                            .foregroundStyle(Theme.C.ink35)
                    }
                    .padding(.leading, Theme.S.screenPad + 2)
                    .padding(.trailing, Theme.S.screenPad)
                    .padding(.vertical, 7)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            .padding(.bottom, walks.isEmpty ? 0 : 8)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .bottom) {
            Rectangle().fill(Theme.C.hairlineSoft).frame(height: 1)
        }
    }

    private var walkCountLabel: String {
        switch walks.count {
        // Honest rather than a fake zero-state: an unfiled job genuinely has
        // no walks, and long-pressing a walk is how one gets here.
        case 0: return "NO WALKS YET"
        case 1: return "1 WALK"
        default: return "\(walks.count) WALKS"
        }
    }
}

/// A compact "pressed block" — the START WALK button's raised look, chip-sized:
/// a `face` cap sitting on a darker `edge` lip (3pt) so it reads as a button, not
/// a flat label. Same depth cue as the primary block button, scaled down.
@ViewBuilder
private func raisedChip<L: View>(face: Color, edge: Color, @ViewBuilder _ label: () -> L) -> some View {
    label()
        .padding(.horizontal, 11)
        .padding(.vertical, 6)
        .background(face)
        .padding(.bottom, 3)      // reveal the darker edge as a bottom lip
        .background(edge)
        .clipShape(RoundedRectangle(cornerRadius: 5))
}

// MARK: - My Business (customization sheet)

/// One sheet, two tabs — the single "MY BUSINESS" entry from the board (replaces
/// the two cryptic VOCAB / PAPER chips). PAPERWORK (logo / colors / letterhead —
/// the key differentiator) is shown first; WORDS is the former vocabulary editor.
/// Wraps the two existing screens in `embedded` mode so they drop their own
/// headers and share this one.
struct CustomizeView: View {
    @Bindable var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var tab: Tab = .paperwork
    private enum Tab { case paperwork, structure, words }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("MY BUSINESS")
                    .font(Theme.F.mono(9, .semibold)).tracking(2.0)
                    .foregroundStyle(Theme.C.orangeDeep)
                Spacer()
                Button { dismiss() } label: {
                    Text("CLOSE")
                        .font(Theme.F.mono(9, .semibold)).tracking(1.0)
                        .foregroundStyle(Theme.C.ink60)
                }
                .buttonStyle(.plain)
            }
            .padding(.horizontal, Theme.S.screenPad)
            .padding(.top, 16)
            .padding(.bottom, 12)

            HStack(spacing: 0) {
                tabButton("LOOK", .paperwork)
                tabButton("STRUCTURE", .structure)
                tabButton("WORDS", .words)
            }

            // Both tabs stay ALIVE (ZStack + opacity), not a `switch` that
            // tears the inactive branch down. LetterheadStudioView holds its
            // edits in @State draft (name/color/logo/terms) committed only on
            // SAVE — a `switch` would destroy that draft on every tab hop,
            // silently losing uncommitted letterhead edits (review #247). The
            // hidden tab is non-interactive so it can't steal touches.
            ZStack {
                LetterheadStudioView(model: model, embedded: true)
                    .opacity(tab == .paperwork ? 1 : 0)
                    .allowsHitTesting(tab == .paperwork)
                // Same keep-alive reasoning as the letterhead tab, and for the
                // same reason: the schema editor holds an uncommitted @State
                // draft, so tearing it down on a tab hop would silently discard
                // half-finished section and field edits.
                DocumentBuilderView(model: model, embedded: true)
                    .opacity(tab == .structure ? 1 : 0)
                    .allowsHitTesting(tab == .structure)
                VocabularyView(model: model, embedded: true)
                    .opacity(tab == .words ? 1 : 0)
                    .allowsHitTesting(tab == .words)
            }
        }
        .background(Theme.C.paper.ignoresSafeArea())
    }

    private func tabButton(_ label: String, _ t: Tab) -> some View {
        let on = tab == t
        return Button { tab = t } label: {
            Text(label)
                .font(Theme.F.ui(13.5, .bold)).tracking(0.8)
                .foregroundStyle(on ? Theme.C.ink : Theme.C.ink35)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 13)
                // Each tab draws its own bottom rule so the bar reads as one
                // line with the active tab picked out in amber.
                .overlay(alignment: .bottom) {
                    (on ? Theme.C.orange : Theme.C.ink).frame(height: on ? 3 : 2)
                }
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
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

/// One walk in the log.
///
/// Two independent targets, deliberately siblings rather than nested: tapping
/// the row reopens the walk's notes, and the trailing chip files it under a
/// job. A `Menu` inside the row's `Button` would fight it for taps.
private struct WalkLogRow<Trailing: View>: View {
    let walk: AppModel.WalkRecord
    let onOpen: () -> Void
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 8) {
            Button(action: onOpen) {
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
                    FieldTag(tag: walkTag(walk))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            trailing
        }
        .padding(.leading, Theme.S.screenPad)
        .padding(.trailing, Theme.S.screenPad - 6)
        .padding(.vertical, 13)
        // No dimming. A walk with no document is a normal, complete walk —
        // fading it implied "lesser" or "dead" and made a saved walk look lost.
        .overlay(alignment: .bottom) { Theme.C.hairline.frame(height: 1) }
    }
}

/// Honest disposition label. "DISCARDED" is deliberately gone: nothing in
/// WalkSummary records a discard, so it was always a guess — and the guess was
/// wrong for the common case of finishing a walk without building a document.
private func walkTag(_ walk: AppModel.WalkRecord) -> TagFixture {
    switch walk.disposition {
    case .saving:     return TagFixture(kind: .yellow, label: "SAVING")
    case .saved:      return TagFixture(kind: .plain, label: "NOTES SAVED")
    case .documented: return TagFixture(kind: .green, label: "SENT")
    }
}

/// The filing affordance: a visible, tap-activated chip that also DISPLAYS the
/// current filing.
///
/// Doing double duty is the point. A bare "FILE" button would say nothing about
/// where a walk already sits, so the operator would have to open the menu to
/// find out. Showing the job name instead means the common case — "which job is
/// this under?" — is answered without any interaction at all.
private struct FileChip: View {
    let walk: AppModel.WalkRecord
    let jobs: [JobModel]
    let onFile: (String?) -> Void

    private var filedJob: JobModel? { jobs.first { $0.id == walk.jobId } }

    var body: some View {
        Menu {
            if jobs.isEmpty {
                Text("No jobs yet — add one below")
            } else {
                ForEach(jobs) { job in
                    Button {
                        onFile(job.id)
                    } label: {
                        Label(job.name, systemImage:
                            walk.jobId == job.id ? "checkmark" : "folder")
                    }
                }
                if walk.jobId != nil {
                    Divider()
                    Button(role: .destructive) { onFile(nil) } label: {
                        Label("Unfile", systemImage: "xmark")
                    }
                }
            }
        } label: {
            if let filedJob {
                Text(filedJob.name.uppercased())
                    .font(Theme.F.mono(8, .semibold))
                    .tracking(0.8)
                    .foregroundStyle(Theme.C.orangeDeep)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: 88, alignment: .trailing)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 8)
                    .background(Theme.C.orangeTint)
                    .contentShape(Rectangle())
            } else {
                // Unfiled reads as an invitation, not an error — most walks
                // will sit unfiled for a while and that's fine.
                Text("FILE")
                    .font(Theme.F.mono(8, .semibold))
                    .tracking(0.8)
                    .foregroundStyle(Theme.C.ink35)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 8)
                    .overlay(
                        RoundedRectangle(cornerRadius: 2)
                            .stroke(style: StrokeStyle(lineWidth: 1, dash: [3, 2]))
                            .foregroundStyle(Theme.C.ink35)
                    )
                    .contentShape(Rectangle())
            }
        }
        .accessibilityLabel(filedJob.map { "Filed under \($0.name). Change." } ?? "File this walk")
    }
}
