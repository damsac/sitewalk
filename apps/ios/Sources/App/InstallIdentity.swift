import Foundation
import Security

/// A stable, anonymous per-install id, used to meter and rate-limit this
/// install at the proxy.
///
/// Keychain-backed rather than `UserDefaults` on purpose: keychain items
/// survive app deletion and reinstall, `UserDefaults` does not. Since the
/// proxy's free-tier allowance is keyed on this id, a `UserDefaults` id would
/// reset the allowance on every reinstall — a one-tap bypass.
///
/// This identifies an INSTALL, not a person. It is a random UUID with no
/// device identifier in it, it never leaves the device except as an opaque
/// string in the proxy credential, and it is not linkable to anything else.
///
/// It is also not a security boundary. Anyone can mint an arbitrary id and
/// send it; what makes that bounded is the proxy's per-install and global
/// spend caps, and — in Phase 2 — App Attest proving the caller is a genuine
/// unmodified build (see services/proxy/README.md).
enum InstallIdentity {
    private static let service = "app.jefe.install"
    private static let account = "install-id"

    /// The id for this install, minting and persisting one on first call.
    static func current() -> String {
        if let existing = read() { return existing }
        let fresh = UUID().uuidString.lowercased()
        write(fresh)
        // Return `fresh` even if the write failed. A keychain failure must not
        // block a walk: the caller still gets a usable id, metering still
        // works for this launch, and the only cost is that the id may not
        // persist. Failing the walk over a bookkeeping problem would be a far
        // worse trade for someone standing on a job site.
        return fresh
    }

    private static func read() -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data,
              let value = String(data: data, encoding: .utf8),
              !value.isEmpty
        else { return nil }
        return value
    }

    private static func write(_ value: String) {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        // Delete-then-add rather than update: simpler, and idempotent if a
        // partial item somehow exists.
        SecItemDelete(query as CFDictionary)

        var attributes = query
        attributes[kSecValueData as String] = Data(value.utf8)
        // AfterFirstUnlock, not WhenUnlocked: a walk can finish and process
        // with the screen locked in a pocket (the background-audio path), and
        // the id has to be readable then.
        attributes[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        SecItemAdd(attributes as CFDictionary, nil)
    }
}
