import SwiftUI
import UIKit

// First-run arc: WELCOME → YOUR BUSINESS → MIC → board. Shown full-screen
// when no BusinessProfile exists (AppRoot gates on it; `resetprofile=1`
// clears the stored profile for QA).
//
// Field Instrument language throughout: paper/ink, one safety orange, mono
// stamps, block buttons with the pressed shadow, targets ≥ 56pt. Three steps,
// flat — no page dots, no illustrations, no cleverness.

struct OnboardingFlow: View {
    /// Called after FINISH on the mic step — the profile is already persisted
    /// (SAVE on step 2); the caller reloads it and shows the board.
    var onComplete: () -> Void

    // welcome + 3 "how it works" beats (learn by seeing) → setup (business, mic).
    private enum Step: Int, CaseIterable { case welcome = 1, seeWalk, seeFix, seeDoc, business, mic }
    @State private var step: Step = .welcome
    private var isIntro: Bool { step.rawValue < Step.business.rawValue }

    // YOUR BUSINESS fields
    @State private var bizName = ""
    @State private var cityState = ""
    @State private var license = ""
    @State private var tradeKey = "landscape"
    private enum Field { case name, city, license }
    @FocusState private var focused: Field?

    // MIC
    private enum MicState { case idle, granted, denied }
    @State private var micState: MicState = .idle

    var body: some View {
        VStack(spacing: 0) {
            topBar
            switch step {
            case .welcome: welcome
            case .seeWalk: seeWalk
            case .seeFix: seeFix
            case .seeDoc: seeDoc
            case .business: business
            case .mic: mic
            }
        }
        .background(Theme.C.paper.ignoresSafeArea())
        .animation(.easeOut(duration: 0.25), value: step)
    }

    // MARK: - Chrome

    private var topBar: some View {
        HStack {
            HStack(spacing: 10) {
                Rectangle().fill(Theme.C.orangeDeep).frame(width: 13, height: 13)
                Text("JEFE")
                    .font(Theme.F.ui(20, .extraBold))
                    .tracking(3.0)
            }
            Spacer()
            if isIntro {
                // The intro teaches; a returning or impatient user skips it.
                Button { step = .business } label: {
                    Text("SKIP")
                        .font(Theme.F.mono(9, .semibold))
                        .tracking(1.5)
                        .foregroundStyle(Theme.C.ink35)
                }
                .buttonStyle(.plain)
            } else {
                Text(String(format: "%02d / 02", step.rawValue - Step.business.rawValue + 1))
                    .font(Theme.F.mono(9, .semibold))
                    .tracking(1.5)
                    .foregroundStyle(Theme.C.ink60)
            }
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 16)
        .padding(.bottom, 14)
        .overlay(alignment: .bottom) { Theme.C.ink.frame(height: 2) }
    }

    /// Pressed-block primary button (same grammar as START WALK / SEND).
    private func blockButton(_ title: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            ZStack {
                RoundedRectangle(cornerRadius: Theme.S.radius)
                    .fill(Theme.C.orangeDeep)
                    .offset(y: 3)
                RoundedRectangle(cornerRadius: Theme.S.radius)
                    .fill(Theme.C.orange)
                Text(title)
                    .font(Theme.F.ui(15, .bold))
                    .tracking(1.4)
                    .foregroundStyle(Theme.C.onOrange)
            }
            .frame(height: Theme.S.buttonHeight)
        }
        .buttonStyle(.plain)
    }

    // MARK: - Step 1 · WELCOME (payoff first — show the finished paperwork)

    private var welcome: some View {
        VStack(alignment: .leading, spacing: 0) {
            Spacer()
            SectionLabel("TALK YOUR WALK", color: Theme.C.orangeDeep)
            Text("Say it out loud.\nGet the paperwork.")
                .font(Theme.F.ui(30, .bold))
                .lineSpacing(2)
                .padding(.top, 8)
            Text("Walk the job and talk — like you’d tell a helper. Jefe turns it into a finished estimate you can send.")
                .font(Theme.F.serif(15))
                .foregroundStyle(Theme.C.ink60)
                .lineSpacing(3)
                .padding(.top, 14)
            miniEstimate
                .padding(.top, 20)
            Spacer()
            blockButton("GET STARTED") { step = .seeWalk }
                .padding(.bottom, 10)
        }
        .padding(.horizontal, Theme.S.screenPad)
    }

    // MARK: - Steps 2–4 · HOW IT WORKS (show, don't tell — plain words, a phone
    // doing the thing on every screen, and the trust beat up front)

    private var seeWalk: some View {
        beat(kick: "STEP 1 OF 3", title: "Walk & talk",
             lede: "Tap once, then just talk. Jefe listens the whole time and writes down what matters.",
             next: { step = .seeFix }) { walkPreview }
    }
    private var seeFix: some View {
        beat(kick: "STEP 2 OF 3", title: "Fix anything",
             lede: "It’s a first draft. Tap any line to fix a word or add a price. You’re always in control.",
             next: { step = .seeDoc }) { fixPreview }
    }
    private var seeDoc: some View {
        beat(kick: "STEP 3 OF 3", title: "One tap → paperwork",
             lede: "Turn your walk into an estimate, invoice, or report — with your logo on it. Then text or email it.",
             next: { step = .business }, cta: "SET UP MY BUSINESS") { docPreview }
    }

    private func beat<Preview: View>(
        kick: String, title: String, lede: String,
        next: @escaping () -> Void, cta: String = "NEXT",
        @ViewBuilder preview: () -> Preview
    ) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            SectionLabel(kick, color: Theme.C.orangeDeep).padding(.top, 20)
            Text(title).font(Theme.F.ui(28, .bold)).padding(.top, 6)
            Text(lede)
                .font(Theme.F.serif(15)).foregroundStyle(Theme.C.ink60)
                .lineSpacing(3).padding(.top, 12)
            preview().padding(.top, 18)
            Spacer()
            dots(step.rawValue - Step.seeWalk.rawValue, of: 3)
            blockButton(cta, action: next).padding(.bottom, 10)
        }
        .padding(.horizontal, Theme.S.screenPad)
    }

    private func dots(_ current: Int, of total: Int) -> some View {
        HStack(spacing: 5) {
            ForEach(0..<total, id: \.self) { i in
                Capsule()
                    .fill(i == current ? Theme.C.ink : Theme.C.hairline)
                    .frame(width: i == current ? 16 : 6, height: 6)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.bottom, 12)
    }

    // MARK: Mini previews — the "show" in show-don't-tell

    private var miniEstimate: some View {
        previewCard {
            HStack {
                Text("Alder Court Lawn").font(Theme.F.serif(13, .bold))
                Spacer()
                Text("ESTIMATE").font(Theme.F.mono(7, .semibold)).tracking(1.4)
                    .foregroundStyle(Theme.C.orangeDeep)
            }
            miniRow("Bark mulch — front beds", right: "$285")
            sendStamp
        }
    }
    private var walkPreview: some View {
        previewCard {
            HStack(spacing: 7) {
                Circle().fill(Theme.C.orangeDeep).frame(width: 7, height: 7)
                Text("LISTENING · 0:14").font(Theme.F.mono(8.5, .semibold)).tracking(1.2)
                    .foregroundStyle(Theme.C.orangeDeep)
            }
            miniRow("Mulch — front beds", sub: "JUST HEARD")
            miniRow("Trim the boxwoods", sub: "JUST HEARD")
        }
    }
    private var fixPreview: some View {
        previewCard {
            miniRow("Mower — front lawn  ✎", sub: "TAP TO EDIT", highlight: true)
            miniRow("Haul & disposal", right: "add $")
            miniRow("Bed edging — front", sub: "~60 LF")
        }
    }
    private var docPreview: some View {
        previewCard {
            HStack {
                Text("Alder Court Lawn").font(Theme.F.serif(13, .bold))
                Spacer()
                Text("EST-0047").font(Theme.F.mono(7, .semibold)).tracking(1.0)
                    .foregroundStyle(Theme.C.orangeDeep)
            }
            miniRow("Bark mulch — front beds", right: "$285")
            miniRow("TOTAL", right: "$680")
            sendStamp
        }
    }

    private var sendStamp: some View {
        Text("SEND ESTIMATE")
            .font(Theme.F.ui(10, .bold)).tracking(0.8)
            .foregroundStyle(Theme.C.onOrange)
            .frame(maxWidth: .infinity).padding(.vertical, 7)
            .background(Theme.C.orange)
    }

    private func previewCard<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 9) { content() }
            .padding(13)
            .background(Theme.C.paperDeep)
            .overlay(Rectangle().stroke(Theme.C.hairline, lineWidth: 1.5))
    }

    private func miniRow(_ text: String, sub: String? = nil, right: String? = nil,
                         highlight: Bool = false) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                Text(text).font(Theme.F.cond(13, .semibold)).foregroundStyle(Theme.C.ink)
                if let sub {
                    Text(sub).font(Theme.F.mono(7, .semibold)).tracking(0.6)
                        .foregroundStyle(Theme.C.ink60)
                }
            }
            Spacer(minLength: 6)
            if let right {
                Text(right).font(Theme.F.mono(11, .semibold)).foregroundStyle(Theme.C.orangeDeep)
            }
        }
        .padding(9)
        .background(Theme.C.sheet)
        .overlay {
            if highlight {
                RoundedRectangle(cornerRadius: 2).stroke(Theme.C.orange, lineWidth: 2)
            }
        }
    }

    // MARK: - Step 2 · YOUR BUSINESS

    private var trimmedName: String { bizName.trimmingCharacters(in: .whitespaces) }

    private var business: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    SectionLabel("YOUR BUSINESS", color: Theme.C.orangeDeep)
                    Text("The name on the paperwork")
                        .font(Theme.F.ui(23, .bold))
                        .padding(.top, 6)
                    Text("EVERY ESTIMATE AND REPORT GOES OUT UNDER THIS LETTERHEAD")
                        .font(Theme.F.mono(8.5))
                        .tracking(0.8)
                        .foregroundStyle(Theme.C.ink60)
                        .padding(.top, 4)

                    formField("BUSINESS NAME", text: $bizName,
                              placeholder: "Summit Lawn & Snow", field: .name)
                        .padding(.top, 24)
                    formField("CITY / STATE", text: $cityState,
                              placeholder: "Denver CO", field: .city)
                        .padding(.top, 20)
                    formField("LICENSE # — OPTIONAL", text: $license,
                              placeholder: "44-0781", field: .license)
                        .padding(.top, 20)

                    SectionLabel("TRADE")
                        .padding(.top, 26)
                    VStack(spacing: 8) {
                        tradeRow(key: "landscape", label: "LANDSCAPE", stamp: "ESTIMATES")
                        tradeRow(key: "property", label: "PROPERTY MGMT", stamp: "MOVE-OUT REPORTS")
                        tradeRow(key: "inspection", label: "INSPECTION", stamp: "INSPECTION REPORTS")
                    }
                    .padding(.top, 10)
                    .padding(.bottom, 18)
                }
                .padding(.horizontal, Theme.S.screenPad)
                .padding(.top, 18)
            }
            .scrollDismissesKeyboard(.interactively)

            blockButton("SAVE") {
                BusinessProfile.save(BusinessProfile(
                    businessName: trimmedName,
                    cityState: cityState.trimmingCharacters(in: .whitespaces),
                    licenseNumber: license.trimmingCharacters(in: .whitespaces).isEmpty
                        ? nil : license.trimmingCharacters(in: .whitespaces),
                    tradeKey: tradeKey
                ))
                focused = nil
                step = .mic
            }
            .opacity(trimmedName.isEmpty ? 0.35 : 1)
            .disabled(trimmedName.isEmpty)
            .padding(.horizontal, Theme.S.screenPad)
            .padding(.top, 12)
            .padding(.bottom, 10)
            .overlay(alignment: .top) { Theme.C.hairline.frame(height: 1) }
        }
    }

    /// Underlined text field in the app idiom (ReviewView editSheet /
    /// VocabularyView add bar): mono label stamp, mono entry, rule under —
    /// orange while focused.
    private func formField(
        _ label: String, text: Binding<String>, placeholder: String, field: Field
    ) -> some View {
        VStack(alignment: .leading, spacing: 9) {
            SectionLabel(label)
            TextField(placeholder, text: text)
                .font(Theme.F.mono(15, .medium))
                .autocorrectionDisabled()
                .focused($focused, equals: field)
                .frame(minHeight: 28)
                .padding(.bottom, 6)
                .overlay(alignment: .bottom) {
                    (focused == field ? Theme.C.orange : Theme.C.ink)
                        .frame(height: focused == field ? 2 : 1.5)
                }
        }
    }

    /// Stamped-chip trade row: square tick, mono trade name, document stamp.
    private func tradeRow(key: String, label: String, stamp: String) -> some View {
        let selected = tradeKey == key
        return Button { tradeKey = key } label: {
            HStack(spacing: 12) {
                Rectangle()
                    .fill(selected ? Theme.C.orange : Color.clear)
                    .frame(width: 12, height: 12)
                    .overlay(Rectangle().stroke(
                        selected ? Theme.C.orange : Theme.C.ink35, lineWidth: 1.5))
                Text(label)
                    .font(Theme.F.mono(11, .semibold))
                    .tracking(1.4)
                    .foregroundStyle(Theme.C.ink)
                Spacer(minLength: 8)
                Text(stamp)
                    .font(Theme.F.mono(8, .semibold))
                    .tracking(1.0)
                    .foregroundStyle(selected ? Theme.C.orangeDeep : Theme.C.ink60)
                    .padding(.horizontal, 6)
                    .padding(.top, 3)
                    .padding(.bottom, 2)
                    .background(selected ? Theme.C.orangeTint : Theme.C.paperDeep)
            }
            .padding(.horizontal, 14)
            .frame(height: Theme.S.minTarget)
            .background(selected ? Theme.C.sheet : Color.clear)
            .overlay(Rectangle().stroke(
                selected ? Theme.C.ink : Theme.C.hairline, lineWidth: selected ? 2 : 1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    // MARK: - Step 3 · MIC

    private var mic: some View {
        VStack(alignment: .leading, spacing: 0) {
            SectionLabel("MIC CHECK", color: Theme.C.orangeDeep)
                .padding(.top, 18)
            // The heading itself confirms the grant — no jarring banner.
            Text(micState == .granted ? "You’re ready to walk." : "Let Jefe hear your walk.")
                .font(Theme.F.ui(23, .bold))
                .padding(.top, 6)
            Text("YOUR AUDIO STAYS ON YOUR PHONE")
                .font(Theme.F.mono(8.5))
                .tracking(0.8)
                .foregroundStyle(Theme.C.ink60)
                .padding(.top, 4)

            // On grant, each row picks up a small green ON stamp — the
            // confirmation lives in the content, not a full-width block.
            VStack(spacing: 0) {
                micRow("MIC", "LISTENS ONLY WHILE YOU WALK")
                micRow("STT", "TRANSCRIBES ON-DEVICE")
                micRow("AUDIO", "NEVER LEAVES YOUR PHONE")
            }
            .padding(.top, 22)
            .overlay(alignment: .top) { Theme.C.hairline.frame(height: 1) }

            Spacer()

            // Vocabulary seeding deliberately does NOT live here (Plan 15
            // D9-15): the vocab card shows on the FIRST notes-screen
            // appearance — after the user's first real walk — see
            // NotesView.vocabCardShownKey / VocabSeedCard.swift.

            // Only the DENIED case still needs a bar — it's a real problem the
            // operator has to act on. Grant is confirmed inline above.
            if micState == .denied {
                noteBar("MIC IS OFF — YOU CAN CONTINUE AND ENABLE IT ANYTIME IN SETTINGS",
                        color: Theme.C.redTag, tint: Theme.C.redTint)
                    .padding(.bottom, 12)
            }

            switch micState {
            case .idle:
                blockButton("REQUEST MIC ACCESS") {
                    Task {
                        let granted = await AudioCaptureSource.requestPermissions()
                        micState = granted ? .granted : .denied
                    }
                }
                .padding(.bottom, 10)
            case .granted:
                blockButton("START WALKING") { onComplete() }
                    .padding(.bottom, 10)
            case .denied:
                blockButton("CONTINUE") { onComplete() }
                    .padding(.bottom, 10)
            }
        }
        .padding(.horizontal, Theme.S.screenPad)
    }

    /// Ledger row for the mic step — like `ledgerRow`, but gains a green ON
    /// stamp on the right once permission is granted.
    private func micRow(_ index: String, _ label: String) -> some View {
        HStack(spacing: 14) {
            Text(index)
                .font(Theme.F.mono(10, .semibold))
                .foregroundStyle(Theme.C.orangeDeep)
            Text(label)
                .font(Theme.F.mono(11, .semibold))
                .tracking(1.6)
                .foregroundStyle(Theme.C.ink)
            Spacer(minLength: 0)
            if micState == .granted {
                Text("ON")
                    .font(Theme.F.mono(8, .semibold)).tracking(1.0)
                    .foregroundStyle(Theme.C.greenTag)
                    .padding(.horizontal, 6).padding(.top, 3).padding(.bottom, 2)
                    .background(Theme.C.greenTint)
            }
        }
        .padding(.vertical, 13)
        .overlay(alignment: .bottom) { Theme.C.hairline.frame(height: 1) }
    }

    private func noteBar(_ text: String, color: Color, tint: Color) -> some View {
        HStack(spacing: 0) {
            color.frame(width: 3)
            Text(text)
                .font(Theme.F.mono(8, .semibold))
                .tracking(0.4)
                .foregroundStyle(color)
                .padding(.horizontal, 9)
                .padding(.vertical, 6)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(tint)
    }
}
