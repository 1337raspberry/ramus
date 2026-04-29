import Foundation
import os
import Security

private let log = Logger(subsystem: "com.raspsoft.ramus", category: "keychain")

/// Thin wrapper over `SecItem*` for `kSecClassGenericPassword` items keyed by
/// a single service + account pair. `service` is the shared bundle-scoped
/// namespace (e.g. "com.raspsoft.ramus.tokens"); `account` is the token key
/// name ("plexAuthToken" / "plexServerToken") — matching the Rust-side
/// `TokenKey::as_str()` values.
///
/// Items are written with `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`
/// so playback can read the token after a reboot before the user unlocks
/// (required for background audio session restart). The `ThisDeviceOnly`
/// suffix opts the entry out of iCloud Keychain so a Plex auth token written
/// on one signed-in device does not replicate to the user's other Apple-ID
/// devices.
final class KeychainBridge {
    static let shared = KeychainBridge()

    private let service = "com.raspsoft.ramus.tokens"

    func read(account: String) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]

        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess,
              let data = item as? Data,
              let value = String(data: data, encoding: .utf8) else {
            return nil
        }
        return value
    }

    @discardableResult
    func write(account: String, value: String) -> Bool {
        guard let data = value.data(using: .utf8) else { return false }

        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]

        // Try an update first; if nothing exists, fall through to add.
        let updateAttrs: [String: Any] = [
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        let updateStatus = SecItemUpdate(query as CFDictionary, updateAttrs as CFDictionary)
        if updateStatus == errSecSuccess {
            return true
        }
        if updateStatus != errSecItemNotFound {
            log.warning("keychain update failed for '\(account, privacy: .public)': OSStatus \(updateStatus)")
            return false
        }

        var addQuery = query
        addQuery[kSecValueData as String] = data
        addQuery[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let addStatus = SecItemAdd(addQuery as CFDictionary, nil)
        if addStatus != errSecSuccess {
            log.warning("keychain add failed for '\(account, privacy: .public)': OSStatus \(addStatus)")
        }
        return addStatus == errSecSuccess
    }

    @discardableResult
    func delete(account: String) -> Bool {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        let status = SecItemDelete(query as CFDictionary)
        return status == errSecSuccess || status == errSecItemNotFound
    }
}
