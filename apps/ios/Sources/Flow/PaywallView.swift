import StoreKit
import SwiftUI

/// The Jefe Pro paywall.
///
/// Shown when the free tier runs out at START WALK, and reachable deliberately
/// from the board. Written to be read by someone standing on a job site who
/// just wanted to record a walk: what happened, what it costs, one button.
///
/// Deliberately NOT a feature grid. Every paid feature is the same feature —
/// more walks — so a checklist would be padding, and padding at the moment
/// someone is blocked from working reads as a stall.
struct PaywallView: View {
    @Bindable var model: AppModel
    @Environment(\.dismiss) private var dismiss

    /// Usage that triggered this, when it was a refusal. Nil when the operator
    /// opened the paywall themselves — the copy changes accordingly, because
    /// "you've used all 5" is wrong and alarming if they came here by choice.
    var blocked: (used: Int, limit: Int)?

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
        .task { await entitlement.start() }
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
                Text("CLOSE")
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
            if let blocked {
                Text("You've used all \(blocked.limit) free walks this month.")
                    .font(Theme.F.serif(23, .bold))
                    .foregroundStyle(Theme.C.ink)
                    .fixedSize(horizontal: false, vertical: true)
                Text("They reset on the 1st. Jefe Pro removes the limit today.")
                    .font(Theme.F.ui(14, .medium))
                    .foregroundStyle(Theme.C.ink60)
                    .fixedSize(horizontal: false, vertical: true)
            } else {
                Text("Walk as many jobs as you want.")
                    .font(Theme.F.serif(23, .bold))
                    .foregroundStyle(Theme.C.ink)
                    .fixedSize(horizontal: false, vertical: true)
                Text("The free tier covers \(WalkAllowance.freeMonthlyLimit) walks a month. Pro removes the limit.")
                    .font(Theme.F.ui(14, .medium))
                    .foregroundStyle(Theme.C.ink60)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 22)
    }

    private var priceBlock: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let product = entitlement.proProduct {
                // The price comes from StoreKit, never hardcoded — it is
                // localized, and it is the number Apple will actually charge.
                // A hardcoded "$12.99" would be wrong in every other currency
                // and would silently lie after any price change.
                Text(product.displayPrice)
                    .font(Theme.F.serif(30, .bold))
                    .foregroundStyle(Theme.C.ink)
                Text("per month · cancel anytime")
                    .font(Theme.F.mono(10))
                    .tracking(0.6)
                    .foregroundStyle(Theme.C.ink60)
            } else {
                // No product: either still loading, or the ASC product id
                // doesn't match. Say nothing about price rather than invent one.
                Text("Loading…")
                    .font(Theme.F.mono(11))
                    .foregroundStyle(Theme.C.ink35)
            }
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 26)
    }

    private var reassurance: some View {
        VStack(alignment: .leading, spacing: 9) {
            row("Your notes and jobs stay yours — nothing is deleted if you cancel.")
            row("Audio never leaves your phone. Jefe transcribes it right here.")
            row("No account. Nothing to log into on a job site.")
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 26)
    }

    /// Apple's required subscription disclosure (App Review 3.1.2).
    ///
    /// An auto-renewable subscription must state, IN THE BINARY and on the
    /// purchase screen: the title, the length of the period, the price per
    /// period, and functional links to the Terms of Use (EULA) and Privacy
    /// Policy. Missing any of it is a hard rejection, not a note. Title,
    /// length, and price are above; this carries the renewal terms and the two
    /// links.
    ///
    /// The renewal sentence is Apple's own required substance, in plain words:
    /// billing is on the Apple ID, it renews unless cancelled, and cancellation
    /// is 24 hours before the period ends. Anyone paying $12.99 a month deserves
    /// to read that before paying rather than discover it after.
    private var legalBlock: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Payment is charged to your Apple ID at confirmation. It renews monthly unless you cancel at least 24 hours before the period ends. Manage or cancel it any time in your Apple ID settings.")
                .font(Theme.F.mono(9))
                .lineSpacing(2)
                .foregroundStyle(Theme.C.ink35)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 14) {
                Link("TERMS OF USE", destination: Self.termsURL)
                Link("PRIVACY POLICY", destination: Self.privacyURL)
            }
            .font(Theme.F.mono(9, .semibold))
            .tracking(0.8)
            .foregroundStyle(Theme.C.orangeDeep)
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.top, 24)
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
                Task { await entitlement.purchase() }
            } label: {
                WalkButton(title: entitlement.isWorking ? "…" : "GET JEFE PRO")
            }
            .buttonStyle(.plain)
            // No product means no purchase is possible; a live button would
            // just fail silently.
            .disabled(entitlement.isWorking || entitlement.proProduct == nil)
            .opacity(entitlement.proProduct == nil ? 0.4 : 1)

            // Restore is not optional garnish: Apple requires the path, and
            // with no accounts it is the ONLY way an operator recovers a
            // subscription on a new phone.
            Button {
                Task { await entitlement.restore() }
            } label: {
                Text("RESTORE PURCHASE")
                    .font(Theme.F.mono(9.5, .semibold))
                    .tracking(1.0)
                    .foregroundStyle(Theme.C.ink60)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 10)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(entitlement.isWorking)
        }
        .padding(.horizontal, Theme.S.screenPad)
        .padding(.bottom, 12)
        .padding(.top, 12)
        .overlay(alignment: .top) { Theme.C.hairline.frame(height: 1) }
    }
}
