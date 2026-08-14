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
    // `newjob=1` opens the sheet on launch. simctl can't tap, and shipping a
    // brand-new UI surface unseen is how the inline FILE chip ended up 130pt
    // tall. Same precedent as `paywall=1`.
    @State private var showNewJob = ProcessInfo.processInfo.arguments.contains("newjob=1")
    @State private var newJobName = ""
    /// TODAY collapses so JOBS clears the fold. Expanding shows the rest.
    /// Which job the board is filtered to; `nil` = All.
    ///
    /// Deliberately NOT persisted. A filter set yesterday and forgotten is
    /// indistinguishable from missing walks — and the people using this will
    /// not think "a filter is active", they will think the app lost their
    /// work. Every launch opens on All.
    @State private var selectedJobId: String?
    @State private var showAllWalks = false
    /// Apple's own manage-subscriptions sheet, raised by the PRO chip.
    @State private var showManageSubscription = false
    /// Enough to see today's work at a glance without pushing jobs off
    /// screen — the complaint that prompted this.
    private static let collapsedWalkCount = 3

    /// Maps the model's two flags onto the sheet's reason. Computed here rather
    /// than on `AppModel` so the model never has to name a view type.
    private var paywallReason: PaywallView.Reason {
        if let blocked = model.blockedUsage {
            return .blocked(used: blocked.used, limit: blocked.limit)
        }
        if let freeLeft = model.paywallOfferFreeLeft {
            return .afterFirstWalk(freeLeft: freeLeft)
        }
        return .chosen
    }

    var body: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 3) {
                HStack(alignment: .top) {
                    // THE MASTHEAD. The business name is the headline — set
                    // in the same serif that tops every document this app
                    // sends, so the board opens the way the paperwork opens.
                    //
                    // It replaces a status readout ("4 walks today"), and that
                    // is the point rather than a side effect. A count at the
                    // top has to answer "which count?" forever — walks, walks
                    // today, to file, documents, money — and every answer is
                    // wrong on some ordinary day. A name is true on all of
                    // them. The counts still exist, in the section headers,
                    // which is where you look when you actually want one.
                    VStack(alignment: .leading, spacing: 5) {
                        Text(model.letterheadBiz)
                            .font(Theme.F.serif(28))
                            .lineLimit(2)
                            .minimumScaleFactor(0.7)
                        // Real current date once a profile exists; the frozen
                        // fixture date only survives on the no-profile demo path.
                        Text(model.boardDateLabel)
                            .font(Theme.F.mono(10, .semibold))
                            .tracking(2.0)
                            .foregroundStyle(Theme.C.amberInk)
                    }
                    Spacer(minLength: 12)
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
                    // Demoted from a raised GOLD chip to Tier 2 ink. The brief
                    // reserves amber for the live state and the primary action;
                    // a gold "MY BUSINESS" read as important as START WALK.
                    // Still obviously a button — which was the real complaint —
                    // and now 44pt instead of ~29.
                    Button { showCustomize = true } label: {
                        Text("My business")
                            .font(Theme.F.ui(13, .semibold))
                            .lineLimit(1)
                    }
                    .buttonStyle(.secondaryChip)
                    // A header chip cannot grow without truncating its own
                    // label; capped so the words survive at accessibility sizes.
                    .dynamicTypeSize(...DynamicTypeSize.accessibility1)
                }
                // No profile = the fixture/demo path, which still needs its
                // trade switcher. With the name now in the masthead above,
                // only the switcher itself survives here.
                if model.profile == nil {
                    Menu {
                        ForEach(Fixtures.all, id: \.key) { trade in
                            Button(trade.biz) { model.switchTrade(trade) }
                        }
                    } label: {
                        HStack(spacing: 6) {
                            Text(model.trade.bizCaps)
                                .font(Theme.F.mono(9.5))
                                .tracking(0.8)
                            // SF Symbol, not a typed glyph: correct optical
                            // alignment, free Dynamic Type, free VoiceOver.
                            Image(systemName: "chevron.down")
                                .font(.system(size: 11, weight: .semibold))
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
                        Text("Your mic is off, so Jefe can't hear you. Tap to turn it on.")
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
                jobChips
                if model.sessionWalks.isEmpty {
                    // Fresh board — no walks at all yet.
                    SectionHead(left: "TODAY", right: "0 WALKS", rightColor: Theme.C.amberInk)
                    // One empty-state idiom, shared with JOBS directly below
                    // — see `EmptyPanel`.
                    EmptyPanel("No walks yet. Tap Start walk and talk through the job.")
                        .padding(.horizontal, Theme.S.screenPad)
                        .padding(.top, 16)
                } else if model.looseWalks.isEmpty {
                    // Nothing outstanding: the card is simply absent. An empty
                    // "UNFILED — 0 WALKS" head used to sit here announcing the
                    // absence of work, which is a heading over nothing — the
                    // same defect the authored sections and the total row were
                    // already fixed for. Being caught up should LOOK like being
                    // caught up.
                    EmptyView()
                } else {
                    // Plan 20 D5: rows are tappable — reopen the walk's notes.
                    // // sac: the reopen affordance visuals (chevron? row
                    // // sac: press state?) + the reopened banner are yours.
                    // One SectionHead per non-empty date group (TODAY / EARLIER),
                    // over the CAPPED list: collapsing and date-grouping compose
                    // in that order, so the collapsed view shows the 3 newest
                    // unfiled walks (in practice all TODAY) and expanding reveals
                    // the older ones under their own head.
                    // TO FILE — one bordered zone for the walks that still
                    // want something, replacing a per-date run of section heads.
                    //
                    // The old shape put a File chip on EVERY row, filed or not,
                    // which is how a secondary action ended up repeated down the
                    // screen with the same weight as the walk itself (Isaac,
                    // 2026-08-13: "a bit cluttered"). Filing is only ever
                    // pending for unfiled walks, so the action belongs to a zone
                    // that exists only while there is filing to do — and the
                    // count belongs on that zone rather than on each date.
                    //
                    // No TODAY/EARLIER split inside the card: this is a triage
                    // pile, newest first. Dates still group the walks that have
                    // been filed, where "when" is the question being asked.
                    ToFileCard(count: model.looseWalks.count) {
                        ForEach(Array(visibleWalks.enumerated()), id: \.element.id) { index, walk in
                            if index > 0 {
                                Rectangle().fill(Theme.C.hairlineSoft).frame(height: 1)
                            }
                            ToFileRow(walk: walk) {
                                model.reopenWalk(sessionId: walk.sessionId)
                            } trailing: {
                                FileChip(walk: walk, jobs: activeJobs) { jobId in
                                    fileWalk(walk, to: jobId)
                                }
                            }
                        }
                    }
                    .padding(.horizontal, Theme.S.screenPad)
                    .padding(.top, 16)
                    if model.looseWalks.count > Self.collapsedWalkCount {
                        // Tier 3: was 8.5pt amber text with no bounds at all
                        // (~27pt tall). Amber also goes — a disclosure toggle
                        // is not the screen's primary action.
                        // Sized to its label, not the screen. Full-width gave a
                        // disclosure toggle the visual weight of a primary
                        // action (Isaac's on-device shot: a large grey slab).
                        HStack {
                            Button { showAllWalks.toggle() } label: {
                                Text(showAllWalks
                                     ? "Show less"
                                     : "Show all \(model.looseWalks.count) walks")
                                    .font(Theme.F.ui(13, .semibold))
                            }
                            .buttonStyle(WellChipStyle(minHeight: 34))
                            Spacer()
                        }
                        .padding(.horizontal, Theme.S.screenPad)
                        .padding(.vertical, 8)
                        .overlay(alignment: .bottom) {
                            Rectangle().fill(Theme.C.hairlineSoft).frame(height: 1)
                        }
                    }

                    if let reopenError = model.reopenError {
                        // F4 floor: the breadcrumb surfaces; chrome is sac's.
                        Text(reopenError)
                            .font(Theme.F.ui(13, .medium))
                            .foregroundStyle(Theme.C.amberInk)
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
                    rightColor: Theme.C.amberInk
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
                BlockLabel("START WALK")
            }
            .buttonStyle(.primaryBlock)
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
            PaywallView(model: model, reason: paywallReason)
                .presentationDetents([.large])
                .presentationDragIndicator(.visible)
                .presentationBackground(Theme.C.paper)
        }
        .manageSubscriptionsSheet(isPresented: $showManageSubscription)
    }
}


// MARK: - New job

/// Creating a job, as a sheet.
///
/// The `.alert` this replaces gave a ~30pt field and no room for a hint — the
/// two things this screen needs most, because it is used one-handed standing at
/// a truck. A sheet affords a 56pt field, a trade-shaped example, and a real
/// 62pt Save.
private struct NewJobSheet: View {
    @Binding var name: String
    let onCancel: () -> Void
    let onSave: () -> Void
    /// Focus the field on appear: this sheet exists to take one short string,
    /// so making the operator tap again first is a wasted step.
    @FocusState private var focused: Bool

    private var trimmed: String {
        name.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                SectionLabel("NEW JOB")
                Spacer()
                Button("Cancel", action: onCancel)
                    .font(Theme.F.ui(13, .semibold))
                    .foregroundStyle(Theme.C.ink60)
            }
            .padding(.horizontal, Theme.S.screenPad)
            .padding(.top, 18)
            .padding(.bottom, 14)

            Text("Call it whatever you call it on site.")
                .font(Theme.F.ui(14, .medium))
                .foregroundStyle(Theme.C.ink60)
                .padding(.horizontal, Theme.S.screenPad)
                .padding(.bottom, 12)

            // 56pt and recessed, matching the Tier 3 well: a field is something
            // you type INTO, so it is cut into the sheet rather than raised off
            // it.
            TextField("", text: $name, prompt: Text("14 Oakfield, back beds")
                .foregroundColor(Theme.C.ink45))
                .font(Theme.F.ui(17, .medium))
                .foregroundStyle(Theme.C.ink)
                .textInputAutocapitalization(.words)
                .autocorrectionDisabled()
                .submitLabel(.done)
                .onSubmit { if !trimmed.isEmpty { onSave() } }
                .focused($focused)
                .padding(.horizontal, 14)
                .frame(height: 56)
                .background(Theme.C.paperDeep)
                .overlay(alignment: .top) { Theme.C.hairline.frame(height: 1.5) }
                .overlay(
                    RoundedRectangle(cornerRadius: Theme.S.radiusCard)
                        .stroke(Theme.C.hairline)
                )
                .clipShape(RoundedRectangle(cornerRadius: Theme.S.radiusCard))
                .padding(.horizontal, Theme.S.screenPad)

            Spacer(minLength: 0)

            Button(action: onSave) { BlockLabel("SAVE JOB") }
                .buttonStyle(RaisedBlockStyle(leadingDot: false))
                // Core rejects an empty name rather than coercing it (R6), so
                // the button simply does not offer to send one — the style goes
                // flat, which is now what unpressable looks like.
                .disabled(trimmed.isEmpty)
                .padding(.horizontal, Theme.S.screenPad)
                .padding(.bottom, 14)
        }
        .background(Theme.C.paper)
        .onAppear { focused = true }
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
    /// Jobs as a filter strip, plus the walks already filed under them.
    ///
    /// Replaces a second full list — its own head, count, create button and a
    /// stack of job cards — sitting under the walk log. Two lists with two
    /// hierarchies is what made this screen read as cluttered (Isaac,
    /// 2026-08-13): a job is a LENS on the walks, not a parallel thing to
    /// browse. The walks that were nested inside each job card now sit in one
    /// list the chips narrow.
    var jobsSection: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let jobsError {
                // Sentence case: caps for a stamped LABEL is correct, caps for
                // a whole sentence destroys word shapes, and this has to land.
                Text(jobsError)
                    .font(Theme.F.ui(13, .medium))
                    .foregroundStyle(Theme.C.redTag)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, Theme.S.screenPad)
                    .padding(.top, 8)
            }
            filedWalksList
        }
        .onAppear(perform: loadJobs)
        // A SHEET, not an `.alert` (design review P1 #9). A TextField inside an
        // alert is a ~30pt target with no room for a real placeholder — the two
        // things this needs most, since it is used one-handed on a job site.
        .sheet(isPresented: $showNewJob) {
            NewJobSheet(
                name: $newJobName,
                onCancel: { newJobName = ""; showNewJob = false },
                onSave: { createJob(); showNewJob = false }
            )
            .presentationDetents([.height(300)])
            .presentationBackground(Theme.C.paper)
        }
    }

    /// The filter row. `+ Job` rides inside it rather than owning a full-width
    /// slot: creating a job is rare, and a dashed box reads as an empty
    /// placeholder rather than a control (see `WellChipStyle`).
    private var jobChips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 7) {
                jobChip(title: "All", selected: selectedJobId == nil) { selectedJobId = nil }
                ForEach(activeJobs) { job in
                    jobChip(title: job.name, selected: selectedJobId == job.id) {
                        selectedJobId = job.id
                    }
                }
                // Same capsule as its neighbours. It used to be a rounded-rect
                // well chip, which read as a different KIND of control sitting
                // in a row of pills (Isaac, 2026-08-13: "it looks out of
                // place"). It belongs to this row and should be shaped like it
                // — amber ink is enough to say it makes something rather than
                // filters something.
                Button { showNewJob = true } label: {
                    HStack(spacing: 5) {
                        Image(systemName: "plus").font(.system(size: 11, weight: .bold))
                        Text("Job").font(Theme.F.ui(13.5, .semibold))
                    }
                    .foregroundStyle(Theme.C.amberInk)
                    .padding(.horizontal, 15)
                    .frame(minHeight: 36)
                    .background(Capsule().fill(Theme.C.orangeTint))
                }
                .buttonStyle(.bareTap)
                .accessibilityLabel("Add a new job")
            }
            .padding(.horizontal, Theme.S.screenPad)
            .padding(.vertical, 12)
        }
    }

    /// No tick and no travel on purpose: selecting a filter changes what you
    /// SEE, and the pill filling with ink is that feedback, immediately. The
    /// tick belongs to taps that commit something (`BareTapStyle`).
    private func jobChip(
        title: String, selected: Bool, tap: @escaping () -> Void
    ) -> some View {
        Button(action: tap) {
            Text(title)
                .font(Theme.F.ui(13.5, .semibold))
                .lineLimit(1)
                .foregroundStyle(selected ? Theme.C.paper : Theme.C.ink60)
                .padding(.horizontal, 15)
                .frame(minHeight: 36)
                .background(Capsule().fill(selected ? Theme.C.ink : Theme.C.paperDeep))
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? [.isSelected] : [])
    }

    /// Walks already filed, narrowed by the selected chip. Grouped by day —
    /// unlike the TO FILE pile, where the question is "what still needs me?",
    /// the question here is "when did I do this?".
    private var filedWalksList: some View {
        let walks = filedWalks
        return Group {
            if walks.isEmpty {
                if let id = selectedJobId {
                    // A filter that hides everything has to explain itself and
                    // offer the way out, or it reads as lost work.
                    VStack(spacing: 10) {
                        EmptyPanel("No walks filed at \(jobName(id)) yet.")
                        Button("Show all walks") { selectedJobId = nil }
                            .font(Theme.F.ui(13, .semibold))
                            .buttonStyle(WellChipStyle(minHeight: 34))
                    }
                    .padding(.horizontal, Theme.S.screenPad)
                    .padding(.vertical, 14)
                } else if activeJobs.isEmpty {
                    EmptyPanel("No jobs yet. Add one and your walks will file under it.")
                        .padding(.horizontal, Theme.S.screenPad)
                        .padding(.bottom, 14)
                }
            } else {
                ForEach(
                    AppModel.groupWalksByDay(walks, now: Date(), calendar: .current)
                ) { section in
                    SectionHead(
                        left: section.title,
                        right: "\(section.walks.count) \(section.walks.count == 1 ? "WALK" : "WALKS")",
                        rightColor: Theme.C.amberInk
                    )
                    ForEach(section.walks) { walk in
                        WalkLogRow(walk: walk) {
                            model.reopenWalk(sessionId: walk.sessionId)
                        } trailing: {
                            // Where it went, in the slot an unfiled row uses
                            // for what it needs. One trailing column, two
                            // states, never both.
                            Text(jobName(walk.jobId))
                                .font(Theme.F.ui(12, .semibold))
                                .foregroundStyle(Theme.C.ink45)
                                .lineLimit(1)
                                .frame(maxWidth: 108, alignment: .trailing)
                        }
                    }
                }
            }
        }
    }

    private var filedWalks: [AppModel.WalkRecord] {
        let filed = model.sessionWalks.filter { $0.jobId != nil }
        guard let selectedJobId else { return filed }
        return filed.filter { $0.jobId == selectedJobId }
    }

    private func jobName(_ id: String?) -> String {
        guard let id else { return "" }
        return operatorJobs.first { $0.id == id }?.name ?? "Filed"
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
            // Tier 3. These OPEN BILLING but read as stamps at ~16pt — the
            // exact "looks like a label, is actually a button" failure.
            Button { showManageSubscription = true } label: {
                Text("Pro")
                    .font(Theme.F.ui(12, .semibold))
                    .foregroundStyle(Theme.C.greenTag)
            }
            .buttonStyle(WellChipStyle(tint: Theme.C.greenTint, text: Theme.C.greenTag, minHeight: 36))
        } else if model.entitlement.canSubscribe {
            // Only shown when the limit is actually being enforced. With no
            // purchasable product the gate declines to block (see
            // `WalkAllowance.decide`), so "2 LEFT" would be a lie — and a
            // discouraging one, since nothing runs out.
            let left = WalkAllowance.remaining(in: WalkMeter.load())
            Button {
                model.blockedUsage = nil   // opened by choice, not refused
                model.paywallOfferFreeLeft = nil   // an offer, not this
                model.showPaywall = true
            } label: {
                Text(left == 0 ? "0 left" : "\(left) left")
                    .font(Theme.F.ui(12, .semibold))
                    .foregroundStyle(left == 0 ? Theme.C.redTag : Theme.C.ink60)
            }
            .buttonStyle(WellChipStyle(
                tint: left == 0 ? Theme.C.redTint : Theme.C.paperDeep,
                text: left == 0 ? Theme.C.redTag : Theme.C.ink60,
                minHeight: 36
            ))
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
            jobsError = "Couldn't load jobs: \(EngineErrorText.readable(error))"
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
            jobsError = "Couldn't file walk: \(EngineErrorText.readable(error))"
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
            jobsError = "Couldn't add job: \(EngineErrorText.readable(error))"
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
        // An actual CARD. The type's own comment used to say "built as a card"
        // while drawing a hairline underline — and a rule cannot group a job
        // with its walks, so the board read as one undifferentiated ledger. A
        // ledger rules its lines because ink can't be raised; an app builds
        // containers because things can be touched.
        //
        // Sheet white on paperDeep with a 4pt radius and a hairline: exactly the
        // move the document preview already makes, applied one level up. The
        // design review calls this the single biggest "now it's an app" change
        // on this screen.
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text(job.name)
                    .font(Theme.F.serif(17, .semibold))
                    .foregroundStyle(Theme.C.ink)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 8)
                Text(walkCountLabel)
                    .font(Theme.F.ui(12, .medium))
                    .foregroundStyle(walks.isEmpty ? Theme.C.ink60 : Theme.C.amberInk)
                    .lineLimit(1)
                    .fixedSize()
            }
            .padding(.horizontal, 14)
            .padding(.top, 13)
            .padding(.bottom, walks.isEmpty ? 13 : 9)

            // The walks themselves — the "an email lands months later asking
            // about this job" surface. Tapping reopens that walk's notes,
            // which are the durable record; the document is regenerated from
            // them on demand rather than stored.
            ForEach(walks) { walk in
                Button { onOpenWalk(walk) } label: {
                    HStack(spacing: 10) {
                        Text(AppModel.walkDateLabel(epochSeconds: walk.startedAt))
                            .font(Theme.F.mono(9.5))
                            .foregroundStyle(Theme.C.ink)
                            .lineLimit(1)
                            .fixedSize()
                        Text(walk.subtitle)
                            .font(Theme.F.cond(11.5, .medium))
                            .foregroundStyle(Theme.C.ink60)
                            .lineLimit(1)
                        Spacer(minLength: 8)
                        Image(systemName: "chevron.right")
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundStyle(Theme.C.ink60)
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                }
                // ONE style. This carried `.buttonStyle(FieldRowStyle(...))`
                // followed by `.buttonStyle(.plain)`, and the second silently
                // won — so these rows never got the press fill the first one
                // was added for.
                .buttonStyle(FieldRowStyle(minHeight: 48))
                .overlay(alignment: .top) {
                    Theme.C.hairlineSoft.frame(height: 1)
                }
            }
            .padding(.bottom, walks.isEmpty ? 0 : 5)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.C.sheet)
        .clipShape(RoundedRectangle(cornerRadius: Theme.S.radiusCard))
        .overlay(
            RoundedRectangle(cornerRadius: Theme.S.radiusCard)
                .stroke(Theme.C.hairline, lineWidth: 1)
        )
    }

    /// Sentence case, not caps. Caps for a stamped LABEL is correct; this is a
    /// phrase, and caps sentences cost reading speed by destroying word shapes.
    private var walkCountLabel: String {
        switch walks.count {
        // Honest rather than a fake zero-state: an unfiled job genuinely has
        // no walks, and filing one from the board is how it gets any.
        case 0: return "No walks yet"
        case 1: return "1 walk"
        default: return "\(walks.count) walks"
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
        .clipShape(RoundedRectangle(cornerRadius: Theme.S.radiusCard))
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
                    .foregroundStyle(Theme.C.amberInk)
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
                .foregroundStyle(on ? Theme.C.ink : Theme.C.ink45)
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
                        .foregroundStyle(Theme.C.amberInk)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            .padding(.horizontal, 13)
            .padding(.vertical, 11)
            .background(Theme.C.orangeTint)
            .overlay(Rectangle().stroke(Theme.C.orange, lineWidth: 1.5))

            Image(systemName: "arrowtriangle.down.fill")
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
/// The bordered zone for walks that still need filing.
///
/// Amber hairline and label only — the fill stays paper. Isaac's call on the
/// mockup (direction B): amber marks the one thing you press, and START WALK
/// owns it. A card that competed with the button for the eye would make the
/// screen busier, which is the opposite of the brief.
private struct ToFileCard<Content: View>: View {
    let count: Int
    @ViewBuilder var content: Content

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("TO FILE")
                    .font(Theme.F.mono(9.5, .semibold))
                    .tracking(1.4)
                Spacer()
                Text("\(count)")
                    .font(Theme.F.mono(9.5, .semibold))
                    .tracking(1.0)
            }
            .foregroundStyle(Theme.C.amberInk)
            .padding(.horizontal, 13)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity)
            .background(Theme.C.orangeTint)
            content
        }
        .background(Theme.C.sheet)
        .clipShape(RoundedRectangle(cornerRadius: Theme.S.radiusCard))
        .overlay(
            RoundedRectangle(cornerRadius: Theme.S.radiusCard)
                .stroke(Theme.C.orangeDeep, lineWidth: 1.5)
        )
    }
}

/// A row inside TO FILE, which is a different row from the walk log's.
///
/// The card tidied the container and left the contents busy — Isaac's device
/// shot, 2026-08-13: *"it still looks really busy."* Each row was carrying a
/// timestamp, a chevron, a SENT tag and the File button around a title that
/// truncated at about 60% width, three rows deep. Four decorations winning
/// space from the one thing being read.
///
/// So this row drops all four:
///
/// - **No time.** TO FILE is a pile, not a log. "When" is the question the
///   FILED list answers, which is why that one keeps its date heads.
/// - **No chevron.** The text area is the tap target and the File button is
///   the only other one; a third mark pointing at the first is decoration.
/// - **No SENT tag.** Sent and unfiled are different axes, and side by side
///   they read as contradictory. It becomes a word on the metadata line, where
///   "REPORT SENT" is a phrase rather than a badge fighting a button.
/// - **No truncated title.** Two lines, full width, because the sentence is
///   the only part of the row that is content.
private struct ToFileRow<Trailing: View>: View {
    let walk: AppModel.WalkRecord
    let onOpen: () -> Void
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            Button(action: onOpen) {
                VStack(alignment: .leading, spacing: 5) {
                    Text(walk.title)
                        .font(Theme.F.ui(16, .semibold))
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)
                        .fixedSize(horizontal: false, vertical: true)
                    Text(meta)
                        .font(Theme.F.mono(9.5, .medium))
                        .tracking(0.9)
                        .foregroundStyle(Theme.C.ink45)
                        .lineLimit(1)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            trailing
        }
        .padding(.horizontal, 13)
        .padding(.vertical, 13)
    }

    /// "6 ITEMS · ESTIMATE SENT" — one quiet line instead of a line plus a tag.
    private var meta: String {
        let base = walk.subtitle.uppercased()
        guard walk.sent, !base.isEmpty else { return base }
        return "\(base) SENT"
    }
}

private struct WalkLogRow<Trailing: View>: View {
    let walk: AppModel.WalkRecord
    let onOpen: () -> Void
    @ViewBuilder var trailing: Trailing

    var body: some View {
        // TWO LINES, not one (Isaac, on-device 2026-07-29: "this section looks
        // cramped… I can't read the full sentence on each walk").
        //
        // The single-line version crammed five things across one row — time,
        // title, status tag, chevron and the FILE chip — so the title, the only
        // part that is actually CONTENT, got squeezed to "Field session t…".
        // Everything decorative was winning space from the one thing an
        // operator is reading. The title now owns the full width and gets two
        // lines; the small stuff moves to a quiet metadata line beneath it.
        Button(action: onOpen) {
            HStack(alignment: .top, spacing: 12) {
                // Capped, and sized to content rather than a fixed 46pt.
                //
                // #294 adopted Dynamic Type in the font helpers but did not
                // audit the layouts for it, so at accessibility sizes this
                // column wrapped mid-timestamp — "9:" over "41". A stamped scan
                // column has to stay on one line; the review names this exact
                // pattern as the place to cap growth.
                Text(walk.time)
                    .font(Theme.F.mono(11, .medium))
                    .foregroundStyle(Theme.C.ink)
                    .lineLimit(1)
                    .fixedSize()
                    .dynamicTypeSize(...DynamicTypeSize.accessibility1)
                    .frame(minWidth: 46, alignment: .leading)

                VStack(alignment: .leading, spacing: 4) {
                    // Two lines, and the full remaining width. A walk summary
                    // is a sentence; one truncated line of it is useless for
                    // "what happened here?", which is the whole reason the
                    // summary is the title (#221).
                    Text(walk.title)
                        .font(Theme.F.ui(14.5, .semibold))
                        .foregroundStyle(Theme.C.ink)
                        // ONE line. The title is now a single condensed
                        // sentence (`AppModel.firstSentence`), so two lines is
                        // room the row does not need — and Isaac's report was
                        // that the extra height read as cramped, not generous.
                        .lineLimit(1)

                    HStack(spacing: 8) {
                        Text(walk.subtitle)
                            .font(Theme.F.cond(11.5, .medium))
                            .foregroundStyle(Theme.C.ink60)
                            .lineLimit(1)
                        // Only NOTABLE states get a tag. "Notes saved" is what
                        // nearly every walk is, so a chip saying it carried
                        // almost no information while eating the width the
                        // title needed — and sitting it beside a chevron and a
                        // FILE button made three controls compete for the same
                        // corner ("that looks weird").
                        if let tag = notableTag(walk) {
                            FieldTag(tag: tag)
                                .lineLimit(1)
                                .fixedSize()
                        }
                        Spacer(minLength: 8)
                        // Filing lives on the metadata line now, right-aligned
                        // under the chevron rather than fighting it for the
                        // same corner.
                        trailing
                    }
                }

                // The row's own affordance, alone in the trailing position so
                // it unambiguously belongs to the row.
                Image(systemName: "chevron.right")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(Theme.C.ink60)
                    .padding(.top, 2)
            }
            .padding(.leading, Theme.S.screenPad)
            .padding(.trailing, Theme.S.screenPad)
            .padding(.vertical, 12)
        }
        .buttonStyle(FieldRowStyle(minHeight: 62))
        // No dimming. A walk with no document is a normal, complete walk —
        // fading it implied "lesser" or "dead" and made a saved walk look lost.
        .overlay(alignment: .bottom) { Theme.C.hairline.frame(height: 1) }
    }
}

/// The tag a row shows — nil when there is nothing worth saying.
///
/// "NOTES SAVED" is the state of nearly every finished walk, so rendering it on
/// every row is pure furniture: it competes with the title for width and tells
/// the operator something they already assume. SENT and SAVING are genuinely
/// worth a glance, so those keep a chip.
private func notableTag(_ walk: AppModel.WalkRecord) -> TagFixture? {
    switch walk.disposition {
    case .saved:      return nil
    case .saving:     return TagFixture(kind: .yellow, label: "SAVING")
    case .documented: return TagFixture(kind: .green, label: "SENT")
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
                Text("No jobs yet. Add one below.")
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
            // Tier 3 recessed well, both states. The unfiled state used to be
            // a 1px DASHED box at ~30pt — dashed reads "empty placeholder" in
            // app language, which is the exact opposite of "tap me", and Isaac
            // reported it as not obviously tappable. A well says "press here"
            // and catches a glove at 44pt.
            if let filedJob {
                Text(filedJob.name)
                    .font(Theme.F.ui(12, .semibold))
                    .foregroundStyle(Theme.C.amberInk)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: 96, alignment: .trailing)
            } else {
                // Unfiled reads as an invitation, not an error — most walks
                // will sit unfiled for a while and that's fine.
                Text("File")
                    .font(Theme.F.ui(12, .semibold))
                    .foregroundStyle(Theme.C.ink60)
            }
        }
        // Compact because it now sits INLINE on the metadata line. The 8pt
        // hit slop in `WellChrome` keeps the real target near 44pt, so it
        // reads small and still catches a glove.
        .wellChrome(
            tint: filedJob == nil ? Theme.C.paperDeep : Theme.C.orangeTint,
            minHeight: 28
        )
        .accessibilityLabel(filedJob.map { "Filed under \($0.name). Change." } ?? "File this walk")
    }
}
