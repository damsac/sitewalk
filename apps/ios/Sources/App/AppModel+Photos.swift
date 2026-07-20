import Foundation
import os

// Photo attachments (Plan 11), split into its own file to keep AppModel.swift
// under the file/type-length lint budget. Capture writes bytes FIRST (D4
// write order), then calls the engine; deletion tombstones the row only —
// bytes are reclaimed by the reconciling sweep on next app-open, not here.
// sac: capture affordance placement, gallery layout/thumbnails, empty state,
// and per-item attach gesture are yours — this is functional-plain.
extension AppModel {
    private var photoLogger: Logger {
        Logger(subsystem: Bundle.main.bundleIdentifier ?? "sitewalk", category: "photos")
    }

    /// sac: the capture UX (camera vs picker, confirm, where the button
    /// lives) is yours. This just wires bytes → FFI.
    ///
    /// Off-main (PR #176 should-fix): both the disk write and the FFI
    /// `attachPhoto` call used to run synchronously on the main actor, and
    /// the FFI call blocks on a store lock shared with the Rust pump thread —
    /// enough to stall a tap during live extraction. The call itself stays a
    /// plain, non-async, fire-and-forget entry point (no call site changes),
    /// but the body now runs on a chained background `Task` (see
    /// `photoCaptureChain`): bytes-write and attach happen off the main
    /// actor, then the tail of the task hops back (implicitly — `AppModel`
    /// is `@MainActor`) to mutate `photos`/`photoError` and fire
    /// `onComplete`.
    ///
    /// `onComplete` is how `addPhoto` (AppModel.swift, sac's walk-time
    /// caller) applies its optimistic chip bump AFTER the attach actually
    /// succeeds, instead of racing it synchronously the way the old code
    /// implicitly could.
    ///
    /// Ordering under rapid taps: captures are chained onto
    /// `photoCaptureChain` (await the previous capture's Task before
    /// starting this one), so two quick taps run their bytes-write +
    /// attach + state mutation sequentially, in tap order — never
    /// interleaved. (Each capture's own UUID filename is independent either
    /// way, but chaining also keeps `photos` append order matching capture
    /// order, which is the nicer UX and avoids relitigating "is
    /// interleaving actually safe" every time this code changes.)
    func capturePhoto(image: Data, itemId: String?, onComplete: (@MainActor (Bool) -> Void)? = nil) {
        guard let sessionId = currentSessionId else {
            photoError = "no active session to attach a photo to"
            onComplete?(false)
            return
        }
        let name = "\(UUID().uuidString).jpg"
        let dir = photosDirectory // cheap URL/mkdir; fine on the main actor
        let engine = self.engine
        let previous = photoCaptureChain
        photoCaptureChain = Task { [weak self] in
            await previous?.value
            guard let self else { return }
            do {
                // bytes FIRST (Plan 11 D4) — off-main disk write.
                try await Task.detached(priority: .userInitiated) {
                    try image.write(to: dir.appendingPathComponent(name))
                }.value
                // attachPhoto is `async` (WalkEngine seam) so the FFI's
                // store-lock wait doesn't block the main actor either.
                let photo = try await engine.attachPhoto(
                    sessionId: sessionId, itemId: itemId, filename: name, capturedAt: nil
                )
                self.photos.append(photo)
                self.photoError = nil
                onComplete?(true)
            } catch {
                self.photoLogger.error("capturePhoto failed: \(error, privacy: .public)")
                // sac: how errors surface is a design call.
                self.photoError = "\(error)"
                onComplete?(false)
            }
        }
    }

    func removePhoto(_ photo: PhotoModel) {
        do {
            try engine.removePhoto(photoId: photo.id)
            photos.removeAll { $0.id == photo.id }
            photoError = nil
        } catch {
            photoLogger.error("removePhoto failed: \(error, privacy: .public)")
            photoError = "\(error)"
        }
        // bytes are reaped by sweepPhotoBytes() on next app-open, not here (D4)
    }

    func loadPhotos(sessionId: String) {
        photos = (try? engine.listPhotos(sessionId: sessionId)) ?? []
    }

    /// Both app-open sweeps, called together from `AppRoot.body`'s `.task`
    /// (GalleryApp.swift): reconcile photo bytes (Plan 11 D4) and fail any
    /// crash-orphaned `Recording` session. Both are app-open-ONLY, never
    /// background — a concurrent photo sweep could race an in-flight capture
    /// (bytes written, row not yet committed) and delete a just-captured
    /// photo; app-open is the one quiescent point where neither a capture nor
    /// a walk can be mid-flight.
    ///
    /// INVARIANT (sweep-vs-live-walk race — closed by ORDERING alone, and
    /// these three properties keep it closed):
    ///   1. Fully SYNCHRONOUS: no `await` before this call in the `.task`,
    ///      none inside it. A single suspension point could let a walk start
    ///      first, and the resumed sweep would Fail the LIVE session — its
    ///      next append/finish then throws InvalidState.
    ///   2. Runs BEFORE any start-walk path: the user's tap and the autoflow
    ///      script both come after the `.task` head has executed.
    ///   3. NEVER re-fired on background→foreground while a `WalkSession` is
    ///      live — `.task` runs once per view lifetime, and it must stay that
    ///      way (no scenePhase re-trigger).
    func runAppOpenSweeps() {
        sweepPhotoBytes()
        sweepZombieSessions()
        retryFailedSessionsInBackground()
        // Plan 20: BOTH tail calls sit strictly AFTER the synchronous sweeps
        // (#185 invariant — nothing async may run before them).
        warmSttInBackground()
        // Read-only walk-log hydrate (D5/R4): `listSessions` mutates nothing
        // and Recording rows are excluded, so it cannot surface or disturb a
        // walk. Runs here (the quiescent app-open point), never on a
        // scenePhase re-trigger — and it is once-per-process guarded anyway.
        hydrateWalkLog()
    }

    /// The offline banner ("SAVED OFFLINE — DOCUMENTS UNLOCK WHEN YOU
    /// RECONNECT") makes a promise; this is what keeps it. Fired as a
    /// separate, NOT-awaited `Task` after the two synchronous sweeps above
    /// return — it must never delay `runAppOpenSweeps()` itself (invariant 1
    /// on that function: fully synchronous, no suspension point before a
    /// walk can start) and never sit in front of the start-walk path, since
    /// `retryFailedSessions()` is slow (real LLM calls, one per Failed
    /// session).
    ///
    /// Not `Task.detached`: `WalkEngine` is `@MainActor`-isolated, so a
    /// literal detached task would just hop back to the main actor for the
    /// `await engine.retryFailedSessions()` call anyway — this follows the
    /// same plain, unstructured `Task { }` precedent already used for other
    /// fire-and-forget engine calls in this file (`capturePhoto`'s attach
    /// step). "Separate" is the property that matters: this task is never
    /// awaited by `runAppOpenSweeps()`, so it runs concurrently with
    /// whatever the user does next.
    ///
    /// Safe even if a retry completes mid-walk: the state machine guarantees
    /// a live walk's session is `Recording`, and `retry_failed_sessions`
    /// only ever queries and processes `Failed` sessions — it cannot touch
    /// the session the user is currently walking. Picks up zombies from
    /// `sweepZombieSessions()` above for free (crash-orphaned `Recording`
    /// rows just became `Failed`).
    ///
    /// sac: this is a good hook for a badge/history refresh once the count
    /// comes back, if a recovered walk should surface anywhere in the UI.
    private func retryFailedSessionsInBackground() {
        // Review #206 should-fix: `.task` can re-fire on scene re-appearance;
        // overlapping runs would double-spend LLM calls on the same Failed
        // sessions (second attempt loses at the state machine, but the tokens
        // are already burnt). One in-flight retry per app process.
        guard !isRetryingFailedSessions else { return }
        isRetryingFailedSessions = true
        Task {
            defer { isRetryingFailedSessions = false }
            let recovered = (try? await engine.retryFailedSessions()) ?? 0
            if recovered > 0 {
                photoLogger.notice("retried and recovered \(recovered, privacy: .public) failed session(s)")
            }
        }
    }

    /// Plan 20 D7: warm the whisper model at app open so the first START WALK
    /// tap doesn't pay the model read + Metal init (#228). Mirrors
    /// `retryFailedSessionsInBackground` exactly: a separate, NOT-awaited
    /// `Task` fired from the TAIL of `runAppOpenSweeps()` — it must NEVER run
    /// before the synchronous sweeps (#185 invariant: a suspension point ahead
    /// of them could let a walk start and get swept/Failed) and never delay
    /// them. Failure is SILENT-DEGRADE: log + swallow; the next `begin`
    /// cold-loads on demand (today's exact behavior — warm-up must never
    /// block or crash the app). Idempotent on the Rust side (path-keyed
    /// holder), so a `.task` re-fire is a cheap no-op.
    private func warmSttInBackground() {
        Task {
            do {
                try await engine.warmStt()
            } catch {
                photoLogger.error("stt warm-up failed (cold-load fallback): \(error, privacy: .public)")
            }
        }
    }

    /// Reconciling sweep (Plan 11 D4): delete every file in <Documents>/photos/
    /// whose name is NOT in the engine's live set. Idempotent, crash-safe;
    /// reaps tombstoned-row bytes AND never-committed capture orphans with one
    /// rule.
    func sweepPhotoBytes() {
        guard let live = try? Set(engine.liveLivePhotoFilenames()) else { return }
        for file in photoDirContents() where !live.contains(file) {
            deletePhotoFile(file)
        }
    }

    /// A crash/force-quit mid-walk leaves a `Recording` session that can
    /// never resume (there is no live `WalkSession` for it after relaunch).
    /// Best effort like the photo sweep — a failure here (e.g. store lock
    /// contention) just means the zombie waits for the next app-open; it
    /// never blocks launch or crashes the app.
    func sweepZombieSessions() {
        if let swept = try? engine.sweepZombieSessions(), swept > 0 {
            photoLogger.notice("swept \(swept, privacy: .public) zombie session(s)")
        }
    }

    // MARK: Photo byte storage — <Documents>/photos/. Core never touches
    // these; it only ever sees the relative `filename` (Plan 11 D4).

    private var photosDirectory: URL {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let dir = docs.appendingPathComponent("photos", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func photoDirContents() -> [String] {
        (try? FileManager.default.contentsOfDirectory(atPath: photosDirectory.path)) ?? []
    }

    private func deletePhotoFile(_ name: String) {
        try? FileManager.default.removeItem(at: photosDirectory.appendingPathComponent(name))
    }
}
