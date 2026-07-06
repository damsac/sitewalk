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
    func capturePhoto(image: Data, itemId: String?) {
        guard let sessionId = currentSessionId else {
            photoError = "no active session to attach a photo to"
            return
        }
        let name = "\(UUID().uuidString).jpg"
        do {
            try writePhotoBytes(image, name: name) // bytes FIRST (Plan 11 D4)
            let photo = try engine.attachPhoto(sessionId: sessionId, itemId: itemId, filename: name, capturedAt: nil)
            photos.append(photo)
            photoError = nil
        } catch {
            photoLogger.error("capturePhoto failed: \(error, privacy: .public)")
            // sac: how errors surface is a design call.
            photoError = "\(error)"
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

    /// Reconciling sweep (Plan 11 D4): delete every file in <Documents>/photos/
    /// whose name is NOT in the engine's live set. Idempotent, crash-safe;
    /// reaps tombstoned-row bytes AND never-committed capture orphans with one
    /// rule. Call on app launch ONLY (v1): a concurrent/background sweep could
    /// race an in-flight capture (bytes written, row not yet committed) and
    /// delete a just-captured photo. App-open is a quiescent point (no capture
    /// in flight).
    func sweepPhotoBytes() {
        guard let live = try? Set(engine.liveLivePhotoFilenames()) else { return }
        for file in photoDirContents() where !live.contains(file) {
            deletePhotoFile(file)
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

    private func writePhotoBytes(_ data: Data, name: String) throws {
        try data.write(to: photosDirectory.appendingPathComponent(name))
    }

    private func photoDirContents() -> [String] {
        (try? FileManager.default.contentsOfDirectory(atPath: photosDirectory.path)) ?? []
    }

    private func deletePhotoFile(_ name: String) {
        try? FileManager.default.removeItem(at: photosDirectory.appendingPathComponent(name))
    }
}
