import Foundation

/// The free-tier walk meter: 5 finished walks, once, then Jefe Pro.
///
/// Split deliberately into pure decision logic (`WalkAllowance`) and keychain
/// persistence (`WalkMeter`). Everything that decides anything is a static
/// function over explicit inputs, so the rules — what counts, what happens at
/// exactly the limit — are unit-testable without StoreKit or a keychain. Those
/// are the parts that will be wrong.
///
/// ## Why the allowance is lifetime, not monthly
///
/// It used to reset every calendar month (Isaac, 2026-07-26). Changed 2026-08-08
/// because a recurring free tier leaks the middle of our own market: an operator
/// doing five jobs a month gets the whole product free, forever, and that is not
/// a light user we are nurturing — for a lot of small operators that *is* the
/// job. Worse, free walks are billed to the same proxy spend cap as paying
/// subscribers, so a permanently free tier spends paying customers' capacity in
/// perpetuity rather than once.
///
/// What the free tier is *for* is evaluation: prove it works before paying. That
/// job does not need to recur. Keeping the first five free preserves the thing
/// that actually matters — someone reaches a finished document, and therefore
/// the review prompt, without ever entering a card.
///
/// The close is the introductory offer on the paywall, not a monthly refill.
///
/// ## What counts as a walk
///
/// A walk that FINISHED and produced notes. Not a walk that was started and
/// discarded, and never a practice walk. You are metered on output, not on
/// tapping a button — someone who starts a walk, realizes they are at the wrong
/// address, and discards it has received nothing and must not be charged for it.
///
/// ## Where it is enforced
///
/// At `startWalk()`, never at finish. Refusing after the operator has already
/// walked the site and talked for ten minutes would destroy the recording they
/// just made, at the worst possible moment. R6 says reject rather than coerce —
/// but the rejection has to arrive BEFORE the work, not after it.
///
/// ## What this is not
///
/// It is not a security boundary. The meter lives on the device and the
/// entitlement check is StoreKit's local (cryptographically signed, but local)
/// `currentEntitlements`. Someone who patches the binary can walk for free. What
/// bounds that is the proxy's per-install and global daily spend caps, and — in
/// Phase 2 — App Attest. Treating an honest customer well matters more here than
/// making a determined one impossible.
enum WalkAllowance {
    /// Free finished walks per install, for the life of the install.
    static let freeWalkAllowance = 5

    /// Lifetime usage.
    ///
    /// Records written before 2026-08-08 also carried a `"month"` key. It is
    /// deliberately absent here rather than decoded and discarded: `JSONDecoder`
    /// ignores unknown keys, so an existing install's `count` **carries forward**
    /// instead of resetting. That is the conservative direction — nobody is
    /// silently handed a second free allowance by the migration — and it is why
    /// this struct must never gain a non-optional field without a default.
    struct Record: Codable, Equatable {
        var count: Int

        static let empty = Record(count: 0)
    }

    enum Decision: Equatable {
        /// Start the walk. `remaining` is what will be left AFTER this one, for
        /// the "1 free walk left" nudge; nil for Pro (never counted).
        case allowed(remaining: Int?)
        /// Out of free walks — show the paywall instead of recording.
        case blocked(used: Int, limit: Int)
    }

    /// The gate. Pro is unmetered and short-circuits immediately.
    ///
    /// `canSubscribe` is whether a purchasable product actually loaded. **If we
    /// cannot sell someone a way out, we do not block them.** Refusing to take
    /// somebody's money and refusing to let them work is indefensible — and it
    /// is not hypothetical: until the App Store Connect product exists, no
    /// install can subscribe at all, so a hard gate would brick every TestFlight
    /// tester at walk six with no recourse *and no month rollover to wait for*.
    /// That last clause is new: under the old monthly reset, failing closed cost
    /// someone days. Under a lifetime allowance it would cost them the app.
    ///
    /// It also covers the ordinary failures — StoreKit unreachable, a network
    /// blip on a job site with no signal, agreements lapsing. The cost of
    /// failing open is a handful of unmetered walks, bounded by the proxy's
    /// per-install and global daily spend caps. The cost of failing closed is a
    /// contractor standing in front of a client unable to record. Those are not
    /// close.
    static func decide(
        isPro: Bool,
        record: Record,
        limit: Int = freeWalkAllowance,
        canSubscribe: Bool = true
    ) -> Decision {
        if isPro { return .allowed(remaining: nil) }
        if !canSubscribe { return .allowed(remaining: nil) }
        let used = max(0, record.count)
        // `>=`, not `==`: a limit that drops (or a record written by a build
        // with a higher limit) must still block rather than wrap into an
        // unbounded free tier.
        if used >= limit { return .blocked(used: used, limit: limit) }
        return .allowed(remaining: limit - used - 1)
    }

    /// Whether a finished walk spends one of the free five.
    ///
    /// `canSubscribe` gates the COUNT for the same reason `decide` uses it to
    /// gate the BLOCK: **an allowance we are not willing to enforce is not an
    /// allowance we may spend.** Without that clause the meter runs through
    /// the entire pre-launch window — every TestFlight tester silently burning
    /// a lifetime allowance against a product that cannot be bought — and on
    /// the day the App Store Connect product goes live they are all at zero,
    /// having never had the free evaluation the tier exists to give them.
    /// (Isaac, 2026-08-09, on his first walk after a reinstall: *"it said I
    /// have 0 free walks left. I dont think this is correct."*)
    ///
    /// The other direction costs a handful of uncounted walks whenever
    /// StoreKit is unreachable — the same fail-open trade `decide` makes, and
    /// the cheaper one.
    ///
    /// `isMeteredWalk` carries `decide`'s two exemptions, restated at the
    /// other end of the walk: a practice run is onboarding, and demo mode
    /// makes no model calls at all, so neither costs anything the free tier
    /// exists to bound. The caller computes it BEFORE the finish work, since
    /// the exit paths clear the practice flag first.
    static func shouldCount(isPro: Bool, canSubscribe: Bool, isMeteredWalk: Bool) -> Bool {
        if isPro || !isMeteredWalk { return false }
        return canSubscribe
    }

    /// The record after a walk finishes.
    static func recordingFinish(in record: Record) -> Record {
        Record(count: max(0, record.count) + 1)
    }

    /// Free walks left, floored at zero. Used for the board's "2 LEFT" chip.
    static func remaining(in record: Record, limit: Int = freeWalkAllowance) -> Int {
        max(0, limit - max(0, record.count))
    }
}

/// Keychain-backed home for the meter. Keychain rather than `UserDefaults` so
/// delete-and-reinstall is not a one-tap allowance reset (same reasoning as
/// `InstallIdentity`, which shares the store).
///
/// This matters more than it used to. Under a monthly reset, wiping the meter
/// bought someone at most the rest of the month. Under a lifetime allowance it
/// buys them the product, so the keychain is now the only thing between
/// reinstall-looping and a permanently free tier.
enum WalkMeter {
    private static let service = "app.jefe.meter"
    private static let account = "free-walks"

    static func load() -> WalkAllowance.Record {
        guard let data = KeychainStore.read(service: service, account: account),
              let record = try? JSONDecoder().decode(WalkAllowance.Record.self, from: data)
        else { return .empty }
        return record
    }

    /// Persists the record. Deliberately returns nothing: a keychain failure
    /// must not block or fail a walk that already happened — the same trade
    /// `InstallIdentity` makes. The cost of a failed write is one uncounted
    /// walk, which is a far better outcome for someone standing on a job site
    /// than an error over bookkeeping.
    static func save(_ record: WalkAllowance.Record) {
        guard let data = try? JSONEncoder().encode(record) else { return }
        KeychainStore.write(data, service: service, account: account)
    }

    static func recordFinishedWalk() {
        save(WalkAllowance.recordingFinish(in: load()))
    }
}
