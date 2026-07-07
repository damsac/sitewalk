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
    func runAppOpenSweeps() {
        sweepPhotoBytes()
        sweepZombieSessions()
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
