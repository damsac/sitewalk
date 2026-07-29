import Foundation

/// The free-tier walk meter: 5 finished walks per calendar month, then Jefe Pro.
///
/// Split deliberately into pure decision logic (`WalkAllowance`) and keychain
/// persistence (`WalkMeter`). Everything that decides anything is a static
/// function over explicit inputs, so the rules — month rollover, what counts,
/// what happens at exactly the limit — are unit-testable without StoreKit, a
/// keychain, or a wall clock. Those are the parts that will be wrong.
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
    /// Finished walks per calendar month on the free tier (Isaac, 2026-07-26).
    static let freeMonthlyLimit = 5

    /// One month's usage. `month` is a "YYYY-MM" key, not a date, so a rollover
    /// is a string comparison rather than date arithmetic across time zones.
    struct Record: Codable, Equatable {
        var month: String
        var count: Int

        static let empty = Record(month: "", count: 0)
    }

    enum Decision: Equatable {
        /// Start the walk. `remaining` is what will be left AFTER this one, for
        /// the "1 walk left this month" nudge; nil for Pro (never counted).
        case allowed(remaining: Int?)
        /// Out of free walks this month — show the paywall instead of recording.
        case blocked(used: Int, limit: Int)
    }

    /// The month key a date falls in. Local calendar on purpose: an operator's
    /// month is the one on their wall, not UTC's.
    static func monthKey(for date: Date, calendar: Calendar = .current) -> String {
        let parts = calendar.dateComponents([.year, .month], from: date)
        guard let year = parts.year, let month = parts.month else { return "" }
        return String(format: "%04d-%02d", year, month)
    }

    /// Usage for `now`'s month, treating a record from any earlier month as
    /// zero. Rollover is therefore implicit — nothing has to run at midnight on
    /// the 1st, which is the kind of thing that silently never fires.
    static func usage(in record: Record, now: Date, calendar: Calendar = .current) -> Int {
        record.month == monthKey(for: now, calendar: calendar) ? record.count : 0
    }

    /// The gate. Pro is unmetered and short-circuits before any date handling.
    static func decide(
        isPro: Bool,
        record: Record,
        now: Date,
        calendar: Calendar = .current,
        limit: Int = freeMonthlyLimit
    ) -> Decision {
        if isPro { return .allowed(remaining: nil) }
        let used = usage(in: record, now: now, calendar: calendar)
        // `>=`, not `==`: a limit that drops (or a record written by a build
        // with a higher limit) must still block rather than wrap into an
        // unbounded free tier.
        if used >= limit { return .blocked(used: used, limit: limit) }
        return .allowed(remaining: limit - used - 1)
    }

    /// The record after a walk finishes. Rolls the month over on write as well
    /// as on read, so a stale month never accumulates.
    static func recordingFinish(
        in record: Record, now: Date, calendar: Calendar = .current
    ) -> Record {
        let key = monthKey(for: now, calendar: calendar)
        let base = record.month == key ? record.count : 0
        return Record(month: key, count: base + 1)
    }
}

/// Keychain-backed home for the meter. Keychain rather than `UserDefaults` so
/// delete-and-reinstall is not a one-tap allowance reset (same reasoning as
/// `InstallIdentity`, which shares the store).
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

    static func recordFinishedWalk(now: Date = Date()) {
        save(WalkAllowance.recordingFinish(in: load(), now: now))
    }
}
