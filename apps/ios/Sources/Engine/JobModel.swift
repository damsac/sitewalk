import Foundation

/// App-side mirror of the core `Job` record.
///
/// Mirrored rather than using the FFI type directly, for the same reason
/// `DocumentSchemaModel` and `NotesModel` are: a view referencing
/// `MurmurCoreFFI` compiles only in real-core mode, and the jobs board has to
/// be demoable in a clean checkout.
///
/// Jobs have existed in murmur-core since v1 — `sessions.job_id` has always
/// been a foreign key to them. Nothing was ever exposed to the app, so the
/// board stayed session-flat. This is the missing surface, not a new model.
struct JobModel: Identifiable, Hashable {
    let id: String
    /// What the operator calls it — the only field the create UI collects.
    /// Contractors name jobs however they think of them ("Alder Court", "the
    /// Hendersons", "412 Alder Ct"), and splitting that into client/site
    /// fields would impose a taxonomy nobody asked for.
    var name: String
    /// Carried by the model and round-tripped by sync, not yet collected.
    var client: String?
    var site: String?
    var scheduledAt: UInt64?
    var status: JobStatusModel
    var createdAt: UInt64
    var updatedAt: UInt64
}

/// Mirrors core's `JobStatus`. Crosses the FFI boundary as a string so an
/// unknown value surfaces as an allowlist error rather than being silently
/// coerced (R6) — but is an enum app-side, where exhaustive matching is what
/// we want.
enum JobStatusModel: String, CaseIterable, Hashable {
    case active
    case done
    case archived

    /// Unknown strings map to `.active` at the app boundary ONLY as a
    /// last-resort display fallback — core already rejects unknown values on
    /// write, so this is unreachable in practice. It exists so a future core
    /// status can't crash an older build.
    init(fromCore raw: String) {
        self = JobStatusModel(rawValue: raw) ?? .active
    }
}
