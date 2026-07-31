import Foundation
import StoreKit
import os

/// Jefe Pro — the subscription, and whether this install has it.
///
/// StoreKit 2 throughout. Entitlement is read from `Transaction.currentEntitlements`,
/// which StoreKit verifies cryptographically against Apple's signature on device;
/// we never hand-parse a receipt and never ask a server whether someone paid.
///
/// **No accounts, on purpose.** The tiers design chose a minimal proxy with no
/// user identity, so entitlement is per-Apple-ID via StoreKit and nothing else.
/// That is also why `restore()` is not optional garnish: an operator who gets a
/// new phone has no login to fall back on, and Apple requires a restore path in
/// any case.
///
/// **Unverified transactions are ignored, never trusted.** `VerificationResult`
/// carries a `.unverified` case; treating it as entitled would make the
/// signature pointless.
@MainActor
@Observable
final class Entitlement {
    /// Must match the App Store Connect subscription product exactly. If this
    /// string and ASC disagree, `Product.products(for:)` returns empty and the
    /// paywall renders with no price — the failure is silent, so it is worth
    /// checking first when the paywall looks wrong.
    static let proMonthlyID = "com.damsac.jefe.pro.monthly"

    /// Whether this install currently has Jefe Pro.
    ///
    /// Starts `false` and is corrected by `refresh()` on launch. Starting
    /// false-then-correcting rather than true-then-correcting means a
    /// subscriber briefly sees the free-tier count on a cold launch, which is
    /// harmless, instead of a lapsed user briefly getting Pro — the error that
    /// costs money.
    private(set) var isPro = false

    /// The subscription product, once loaded. `nil` while loading or if the
    /// lookup failed — the paywall must handle both without claiming a price.
    private(set) var proProduct: Product?

    /// Whether a subscription can actually be bought right now.
    ///
    /// False while the product is loading, if the App Store Connect product
    /// doesn't exist or its id doesn't match, if agreements aren't signed, or if
    /// StoreKit is simply unreachable. The free-tier gate reads this and
    /// declines to block when it's false — we do not refuse someone's money and
    /// their work at the same time. See `WalkAllowance.decide`.
    var canSubscribe: Bool { proProduct != nil }

    /// Set when a load or purchase failed in a way worth telling the operator
    /// about. `nil` for user-cancelled: cancelling is not an error.
    var purchaseError: String?

    /// True while a purchase or restore is in flight, so the paywall can
    /// disable its buttons rather than allow a double-purchase tap.
    private(set) var isWorking = false

    /// The `Transaction.updates` listener.
    ///
    /// `@ObservationIgnored` because it is plumbing, not UI state — and because
    /// without it the `@Observable` macro synthesizes main-actor-isolated
    /// accessors that `deinit` cannot call. `nonisolated(unsafe)` so `deinit`,
    /// which is not main-actor isolated, can cancel it; safe in fact, since it
    /// is written exactly once in `init` and read exactly once in `deinit` with
    /// no concurrent access in between.
    @ObservationIgnored
    private nonisolated(unsafe) var updatesTask: Task<Void, Never>?
    private let log = Logger(
        subsystem: Bundle.main.bundleIdentifier ?? "sitewalk", category: "billing"
    )

    init() {
        // Transaction.updates delivers renewals, refunds, revocations, Ask-to-Buy
        // approvals, and purchases made on another device. Without this listener
        // a refund or a lapsed renewal would leave `isPro` true until the next
        // cold launch.
        updatesTask = Task { [weak self] in
            for await update in Transaction.updates {
                guard let self else { return }
                if case .verified(let transaction) = update {
                    await transaction.finish()
                }
                await self.refresh()
            }
        }
    }

    deinit { updatesTask?.cancel() }

    /// Loads the product and re-reads entitlement. Safe to call repeatedly.
    func start() async {
        await loadProducts()
        await refresh()
    }

    func loadProducts() async {
        do {
            let products = try await Product.products(for: [Self.proMonthlyID])
            proProduct = products.first
            if proProduct == nil {
                // Not fatal and not the operator's problem — but it means the
                // paywall cannot show a price, so leave a breadcrumb.
                log.error("no product returned for \(Self.proMonthlyID, privacy: .public) — check the ASC product id and that agreements are signed")
            }
        } catch {
            log.error("product load failed: \(error, privacy: .public)")
        }
    }

    /// Re-reads entitlement from StoreKit. This is the only thing that may set
    /// `isPro`.
    func refresh() async {
        var entitled = false
        for await result in Transaction.currentEntitlements {
            // .unverified is deliberately NOT trusted — see the type doc.
            guard case .verified(let transaction) = result else { continue }
            guard transaction.productID == Self.proMonthlyID else { continue }
            if transaction.revocationDate == nil { entitled = true }
        }
        isPro = entitled
    }

    /// Buys Jefe Pro. Returns true only on a verified, completed purchase.
    @discardableResult
    func purchase() async -> Bool {
        guard let product = proProduct else {
            purchaseError = "Can't reach the App Store right now. Try again in a minute."
            return false
        }
        isWorking = true
        defer { isWorking = false }
        do {
            switch try await product.purchase() {
            case .success(let verification):
                guard case .verified(let transaction) = verification else {
                    // Signature check failed. Do not entitle; this is the case
                    // the verification exists for.
                    purchaseError = "That purchase didn't check out. You haven't been charged."
                    log.error("purchase returned an unverified transaction")
                    return false
                }
                await transaction.finish()
                await refresh()
                purchaseError = nil
                return isPro
            case .userCancelled:
                // Not an error. Saying nothing is the correct response.
                purchaseError = nil
                return false
            case .pending:
                // Ask-to-Buy / SCA. The transaction may complete later and will
                // arrive through Transaction.updates.
                purchaseError = "That purchase needs approval first."
                return false
            @unknown default:
                purchaseError = "That purchase didn't go through."
                return false
            }
        } catch {
            purchaseError = "That purchase didn't go through. \(error.localizedDescription)"
            log.error("purchase failed: \(error, privacy: .public)")
            return false
        }
    }

    /// Restores a subscription bought on another device or before a reinstall.
    /// Apple requires this path to exist; with no accounts it is also the only
    /// way an operator recovers on a new phone.
    func restore() async {
        isWorking = true
        defer { isWorking = false }
        do {
            try await AppStore.sync()
            await refresh()
            purchaseError = isPro ? nil : "No subscription found on this Apple ID."
        } catch {
            purchaseError = "Couldn't restore it. \(error.localizedDescription)"
            log.error("restore failed: \(error, privacy: .public)")
        }
    }
}
