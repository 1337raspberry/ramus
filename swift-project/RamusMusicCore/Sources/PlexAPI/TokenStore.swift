import CryptoKit
import Foundation
import IOKit
import os.log

private let log = Logger(subsystem: "com.raspsoft.ramus", category: "TokenStore")

/// Encrypted file-based token storage.
///
/// Tokens are AES-GCM encrypted with a key derived from the machine's
/// hardware UUID, so the file is inert if copied to another machine.
/// Stored in the app's Application Support directory (sandboxed container).
///
/// **Security note**: The encryption key is derived from `IOPlatformUUID`,
/// which is readable by any process on the machine. This provides
/// machine-binding (file useless if exfiltrated) but NOT same-machine
/// process isolation. Any non-sandboxed process running as the same user
/// could derive the key and decrypt the file. This is an accepted trade-off.
enum TokenStore: Sendable {

    private static let directory = "Ramus"
    private static let fileName = "tokens.enc"
    private static let lock = NSLock()

    // MARK: - Public API

    static func read(_ key: TokenKey) -> String? {
        lock.withLock {
            guard let tokens = loadAll() else { return nil }
            return tokens[key.rawValue]
        }
    }

    @discardableResult
    static func write(_ key: TokenKey, value: String) -> Bool {
        lock.withLock {
            var tokens = loadAll() ?? [:]
            tokens[key.rawValue] = value
            return saveAll(tokens)
        }
    }

    @discardableResult
    static func delete(_ key: TokenKey) -> Bool {
        lock.withLock {
            guard var tokens = loadAll() else { return true }
            tokens.removeValue(forKey: key.rawValue)
            return saveAll(tokens)
        }
    }

    // MARK: - Token Keys

    enum TokenKey: String, Sendable {
        case authToken = "plexAuthToken"
        case serverToken = "plexServerToken"
    }

    // MARK: - File Location

    private static func tokenFileURL() -> URL? {
        guard let appSupport = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first else {
            log.error("Could not locate Application Support directory")
            return nil
        }
        let dir = appSupport.appendingPathComponent(directory)
        do {
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        } catch {
            log.error("Failed to create token store directory: \(error.localizedDescription, privacy: .public)")
            return nil
        }
        return dir.appendingPathComponent(fileName)
    }

    // MARK: - Encryption Key

    private static func encryptionKey() -> SymmetricKey? {
        guard let uuid = hardwareUUID() else {
            log.error("Could not read hardware UUID — tokens will not persist")
            return nil
        }
        let hash = SHA256.hash(data: Data(uuid.utf8))
        return SymmetricKey(data: hash)
    }

    private static func hardwareUUID() -> String? {
        let service = IOServiceGetMatchingService(
            kIOMainPortDefault,
            IOServiceMatching("IOPlatformExpertDevice")
        )
        guard service != IO_OBJECT_NULL else { return nil }
        defer { IOObjectRelease(service) }
        let property = IORegistryEntryCreateCFProperty(
            service, "IOPlatformUUID" as CFString, kCFAllocatorDefault, 0
        )
        return property?.takeRetainedValue() as? String
    }

    // MARK: - Read / Write Encrypted File

    /// Caller must hold `lock`.
    private static func loadAll() -> [String: String]? {
        guard let url = tokenFileURL(),
              let key = encryptionKey() else { return nil }
        guard let data = try? Data(contentsOf: url) else { return nil }
        do {
            let box = try AES.GCM.SealedBox(combined: data)
            let decrypted = try AES.GCM.open(box, using: key)
            return try JSONDecoder().decode([String: String].self, from: decrypted)
        } catch {
            log.error("Failed to decrypt token store: \(error.localizedDescription, privacy: .public)")
            return nil
        }
    }

    /// Caller must hold `lock`.
    @discardableResult
    private static func saveAll(_ tokens: [String: String]) -> Bool {
        guard let url = tokenFileURL(),
              let key = encryptionKey() else { return false }
        do {
            let data = try JSONEncoder().encode(tokens)
            let sealed = try AES.GCM.seal(data, using: key)
            guard let combined = sealed.combined else {
                log.error("AES.GCM sealed box has no combined representation — tokens not persisted")
                return false
            }
            try combined.write(to: url, options: .atomic)
            try FileManager.default.setAttributes(
                [.posixPermissions: 0o600],
                ofItemAtPath: url.path
            )
            return true
        } catch {
            log.error("Failed to write token store: \(error.localizedDescription, privacy: .public)")
            return false
        }
    }
}
