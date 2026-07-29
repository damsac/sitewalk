import Foundation
import Security

/// Minimal keychain read/write for the two things that must survive app
/// deletion: the install id and the free-tier walk meter.
///
/// Keychain rather than `UserDefaults` on purpose — keychain items survive
/// delete-and-reinstall, `UserDefaults` does not. Both callers key a usage
/// allowance off their value, so a `UserDefaults` home would make "reinstall the
/// app" a one-tap way to reset it.
///
/// Everything is `AfterFirstUnlock`: a walk can finish and process with the
/// screen locked in a pocket (the background-audio path), and both values have
/// to be readable then.
enum KeychainStore {
    static func read(service: String, account: String) -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data, !data.isEmpty
        else { return nil }
        return data
    }

    /// Writes, replacing any existing value. Returns whether it stuck — callers
    /// decide what a failure means; for both of ours it must never block a walk.
    @discardableResult
    static func write(_ value: Data, service: String, account: String) -> Bool {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        // Delete-then-add rather than update: simpler, and idempotent if a
        // partial item somehow exists.
        SecItemDelete(query as CFDictionary)

        var attributes = query
        attributes[kSecValueData as String] = value
        attributes[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        return SecItemAdd(attributes as CFDictionary, nil) == errSecSuccess
    }
}
