import StoreKit
import SwiftUI

/// The Jefe Pro paywall.
///
/// Shown when the free allowance runs out at START WALK, offered once after the
/// practice walk, and reachable deliberately from the board. Written to be read
/// by someone standing on a job site who just wanted to record a walk: what
/// happened, what it costs, one button.
///
/// Deliberately NOT a feature grid. Every paid feature is the same feature —
/// more walks — so a checklist would be padding, and padding at the moment
/// someone is blocked from working reads as a stall. The two plans are a *price*
/// choice, not a feature choice, and the layout says so.
struct PaywallView: View {
    /// Why this sheet is on screen. The copy changes with it, because "you've
    /// used all 5" is wrong and alarming for someone who opened it by choice,
    /// and a hard sell is wrong for someone who just finished a practice run.
    enum Reason: Equatable {
        /// Refused at START WALK. The only case that is a wall.
        case blocked(used: Int, limit: Int)
        /// The operator opened it themselves from the board.
        case chosen
        /// Offered once, right after the practice walk — the first moment they
        /// have watched the thing actually produce a document.
        case afterPractice
    }

    @Bindable var model: AppModel
    var reason: Reason = .chosen

    @Environment(\.dismiss) private var dismiss
    @State private var selected: Product?
    /// Flips once the load has had long enough that "loading" stops being a
    /// credible explanation. See `priceBlock`.
    @State private var loadTimedOut = false

    private var entitlement: Entitlement { model.entitlement }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    headline
                    priceBlock
                    reassurance
                    legalBlock
                    if let error = entitlement.purchaseError {
                        Text(error)
                            .font(Theme.F.mono(10))
                            .foregroundStyle(Theme.C.redTag)
                            .padding(.horizontal, Theme.S.screenPad)
                            .padding(.top, 14)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            Spacer(minLength: 0)
            actions
        }
        .background(Theme.C.paper.ignoresSafeArea())
        .task {
            await entitlement.start()
            // Select once products exist. Not in `init` — they aren't loaded yet.
            if selected == nil { selected = entitlement.defaultProduct }
        }
        .task {
            // A silent spinner is the worst failure mode this screen has: if the
            // ASC product ids don't match, "Loading…" is permanent, the button
            // stays dead, and nobody who hits the limit can pay.
            //
            // This timeout is the SECOND of two escapes, and it is the one that
            // matters. `didAttemptLoad` covers a load that finished and returned
            // nothing; it does not cover `Product.products(for:)` never
            // returning at all, which is what actually happens on a simulator
            // with no StoreKit configuration — and, on a device, on a job site
            // with a captive-portal wifi that accepts the connection and then
            // answers nothing. Gating the message on `didAttemptLoad` made the
            // timeout unreachable in exactly the case it exists for.
            try? await Task.sleep(for: .seconds(8))
            loadTimedOut = true
        }
        .onChange(of: entitlement.defaultProduct) { _, product in
            if selected == nil { selected = product }
        }
        // The sheet's only job is to sell Pro; once Pro exists it has no reason
        // to be on screen, and leaving it up would make a successful purchase
        // feel like it failed.
        .onChange(of: entitlement.isPro) { _, isPro in if isPro { dismiss() } }
    }

    private var header: some View {
        HStack {
            SectionLabel("JEFE PRO")
            Spacer()
            Button { dismiss() } label: {
                Text(reason == .afterPractice ? "NOT NOW" : "CLOSE")
                    .font(Theme.F.mono(9, .semibold))
                    .tracking(1.0)
                    .foregroundStyle(Theme.C.ink60)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 18)
        .padding(.bottom, 12)
        .overlay(alignment: .bottom) { Theme.C.hairline.frame(height: 1) }
    }

    private var headline: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title)
                .font(Theme.F.serif(23, .bold))
                .foregroundStyle(Theme.C.ink)
                .fixedSize(horizontal: false, vertical: true)
            Text(subtitle)
                .font(Theme.F.ui(14, .medium))
                .foregroundStyle(Theme.C.ink60)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 22)
    }

    private var title: String {
        switch reason {
        case .blocked(_, let limit):
            return "That was your \(limit)th free walk."
        case .afterPractice:
            return "That's the whole job, start to finish."
        case .chosen:
            return "Walk as many jobs as you want."
        }
    }

    private var subtitle: String {
        switch reason {
        case .blocked:
            // No "they reset on the 1st" any more — they don't. Saying plainly
            // that the free ones are spent beats implying a refill never comes.
            return "The free ones are used up. Pro takes the limit off for good."
        case .afterPractice:
            return "Your first \(WalkAllowance.freeWalkAllowance) real walks are free. Pro doesn't count them at all."
        case .chosen:
            return "Free gets you \(WalkAllowance.freeWalkAllowance) walks. Pro doesn't count them."
        }
    }

    // MARK: Plans

    @ViewBuilder
    private var priceBlock: some View {
        VStack(alignment: .leading, spacing: 10) {
            if entitlement.canSubscribe {
                if let annual = entitlement.annualProduct { planRow(annual) }
                if let monthly = entitlement.monthlyProduct { planRow(monthly) }
                if let note = perJobNote {
                    Text(note)
                        .font(Theme.F.mono(10))
                        .tracking(0.4)
                        .foregroundStyle(Theme.C.ink60)
                        .padding(.top, 2)
                }
            } else if entitlement.didAttemptLoad || loadTimedOut {
                // Loaded, and there is nothing to sell. Say so — and say the one
                // thing that is both true and useful: they are not blocked.
                VStack(alignment: .leading, spacing: 8) {
                    Text("Couldn't load pricing.")
                        .font(Theme.F.ui(14, .semibold))
                        .foregroundStyle(Theme.C.ink)
                    Text("The App Store isn't answering. Your walks aren't limited while this is down, so keep working and try again later.")
                        .font(Theme.F.ui(13, .medium))
                        .foregroundStyle(Theme.C.ink60)
                        .fixedSize(horizontal: false, vertical: true)
                }
            } else {
                Text("Loading…")
                    .font(Theme.F.mono(11))
                    .foregroundStyle(Theme.C.ink45)
            }
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 24)
    }

    private func planRow(_ product: Product) -> some View {
        let isSelected = selected?.id == product.id
        return Button {
            selected = product
        } label: {
            HStack(alignment: .center, spacing: 12) {
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 8) {
                        Text(planName(product))
                            .font(Theme.F.ui(15, .semibold))
                            .foregroundStyle(Theme.C.ink)
                        if let badge = savingsBadge(product) {
                            Text(badge)
                                .font(Theme.F.mono(9, .semibold))
                                .tracking(0.6)
                                .foregroundStyle(Theme.C.amberInk)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 3)
                                .background(Theme.C.orangeTint)
                        }
                    }
                    Text(planDetail(product))
                        .font(Theme.F.mono(10))
                        .foregroundStyle(Theme.C.ink60)
                }
                Spacer(minLength: 8)
                Text(product.displayPrice)
                    .font(Theme.F.serif(19, .bold))
                    .foregroundStyle(Theme.C.ink)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 13)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(isSelected ? Theme.C.orangeTint : Theme.C.paper)
            .overlay {
                Rectangle()
                    .strokeBorder(
                        isSelected ? Theme.C.orangeDeep : Theme.C.hairline,
                        lineWidth: isSelected ? 2 : 1
                    )
            }
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isSelected ? [.isSelected, .isButton] : .isButton)
    }

    private func planName(_ product: Product) -> String {
        product.id == Entitlement.proAnnualID ? "Yearly" : "Monthly"
    }

    /// The second line of a plan row: the trial if one is on offer, otherwise
    /// the monthly equivalent for annual — the number that makes the comparison
    /// fair — or the renewal period.
    private func planDetail(_ product: Product) -> String {
        if let trial = trialText(product) { return trial }
        if let each = monthlyEquivalent(product) { return "\(each) a month, billed yearly" }
        return "billed monthly"
    }

    private func monthlyEquivalent(_ product: Product) -> String? {
        guard product.id == Entitlement.proAnnualID else { return nil }
        return (product.price / 12).formatted(product.priceFormatStyle)
    }

    /// "SAVE $40" on the annual row, computed from the two real prices rather
    /// than hardcoded — a stale saving is a false advertising claim, and prices
    /// are localized and can differ per storefront.
    private func savingsBadge(_ product: Product) -> String? {
        guard product.id == Entitlement.proAnnualID,
              let monthly = entitlement.monthlyProduct else { return nil }
        let saved = (monthly.price * 12) - product.price
        guard saved > 0 else { return nil }
        return "SAVE \(saved.formatted(product.priceFormatStyle))"
    }

    /// The introductory offer, only when this Apple ID can still take it.
    /// Advertising a trial someone has already used is a promise Apple's own
    /// purchase sheet will visibly break a second later.
    private func trialText(_ product: Product) -> String? {
        guard entitlement.isEligibleForTrial,
              let offer = product.subscription?.introductoryOffer,
              offer.paymentMode == .freeTrial else { return nil }
        let period = offer.period.formatted(product.subscriptionPeriodFormatStyle)
        return "\(period) free, then \(product.displayPrice)"
    }

    /// The one line of ROI framing, and deliberately conditional rather than a
    /// claim: we do not measure how many walks anyone does, so "about a dollar a
    /// job" is only honest with the assumption attached.
    private var perJobNote: String? {
        guard let monthly = entitlement.monthlyProduct else { return nil }
        let each = (monthly.price / 20).formatted(monthly.priceFormatStyle)
        return "About \(each) a job if you walk 20 a month."
    }

    private var reassurance: some View {
        VStack(alignment: .leading, spacing: 9) {
            row("Cancel whenever. Your notes and jobs stay put.")
            row("Your audio never leaves your phone.")
            row("No account to log into.")
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 24)
    }

    /// Apple's required subscription disclosure (App Review 3.1.2).
    ///
    /// An auto-renewable subscription must state, IN THE BINARY and on the
    /// purchase screen: the title, the length of the period, the price per
    /// period, and functional links to the Terms of Use (EULA) and Privacy
    /// Policy. Missing any of it is a hard rejection, not a note. Title, length
    /// and price are in the plan rows; this carries the renewal terms and the
    /// two links.
    ///
    /// The renewal sentence tracks the SELECTED plan. It used to hardcode
    /// "every month", which becomes false the moment a yearly plan exists — and
    /// a wrong renewal period is exactly the disclosure Apple rejects for.
    private var legalBlock: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Payment goes to your Apple ID when you confirm. It renews \(renewalPhrase) unless you cancel at least 24 hours before it ends. You can cancel any time in your Apple ID settings.")
                .font(Theme.F.mono(9))
                .lineSpacing(2)
                .foregroundStyle(Theme.C.ink45)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 14) {
                Link("TERMS OF USE", destination: Self.termsURL)
                Link("PRIVACY POLICY", destination: Self.privacyURL)
            }
            .font(Theme.F.mono(9, .semibold))
            .tracking(0.8)
            .foregroundStyle(Theme.C.amberInk)
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 22)
        .padding(.bottom, 16)
    }

    private var renewalPhrase: String {
        (selected ?? entitlement.defaultProduct)?.id == Entitlement.proAnnualID
            ? "every year" : "every month"
    }

    // swiftlint:disable force_unwrapping
    // Both are literals verified to parse; a nil here would silently drop a
    // link Apple requires to be present and functional.
    static let termsURL = URL(string: "https://getjefe.netlify.app/terms.html")!
    static let privacyURL = URL(string: "https://getjefe.netlify.app/privacy.html")!
    // swiftlint:enable force_unwrapping

    private func row(_ text: String) -> some View {
        HStack(alignment: .top, spacing: 9) {
            Rectangle()
                .fill(Theme.C.orangeDeep)
                .frame(width: 4, height: 4)
                .padding(.top, 6)
            Text(text)
                .font(Theme.F.ui(13, .medium))
                .foregroundStyle(Theme.C.ink60)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var actions: some View {
        VStack(spacing: 10) {
            Button {
                Task { await entitlement.purchase(selected ?? entitlement.defaultProduct) }
            } label: {
                BlockLabel(entitlement.isWorking ? "…" : buyLabel)
            }
            // No product means no purchase is possible; a live button would
            // just fail silently. `.disabled` now carries the whole visual —
            // the style drops the lip and goes flat, so the control reads as
            // physically unpressable instead of merely faded.
            .buttonStyle(.primaryBlock)
            .disabled(entitlement.isWorking || !entitlement.canSubscribe)

            // Restore is not optional garnish: Apple requires the path, and
            // with no accounts it is the ONLY way an operator recovers a
            // subscription on a new phone.
            // Tier 3: a real 44pt target. Apple requires this path to exist,
            // so it must not be 9pt text with no bounds.
            Button {
                Task { await entitlement.restore() }
            } label: {
                Text("Restore purchase")
                    .font(Theme.F.ui(13, .semibold))
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.wellChip)
            .disabled(entitlement.isWorking)
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.bottom, 12)
        .padding(.top, 12)
        .overlay(alignment: .top) { Theme.C.hairline.frame(height: 1) }
    }

    /// "START FREE TRIAL" only when there is genuinely a trial to start.
    private var buyLabel: String {
        let product = selected ?? entitlement.defaultProduct
        if let product, trialText(product) != nil { return "START FREE TRIAL" }
        return "GET JEFE PRO"
    }
}
