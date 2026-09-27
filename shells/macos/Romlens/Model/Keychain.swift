import Foundation
import RomlensKit
import Security

/// Where the tutor's keys are kept (docs/24, decision 4): one generic
/// password per endpoint in the login keychain, never in the user defaults
/// or a project.
protocol KeyStore: Sendable {
    func key(for endpoint: String) -> String?
    func setKey(_ key: String?, for endpoint: String) throws
}

struct KeychainError: LocalizedError {
    let status: OSStatus
    var errorDescription: String? {
        (SecCopyErrorMessageString(status, nil) as String?) ?? "Keychain error \(status)"
    }
}

struct Keychain: KeyStore {
    static let service = "io.github.ayodehi.Romlens.tutor"

    private func query(_ endpoint: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: Self.service,
            kSecAttrAccount as String: endpoint,
        ]
    }

    func key(for endpoint: String) -> String? {
        var q = query(endpoint)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess,
              let data = out as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    func setKey(_ key: String?, for endpoint: String) throws {
        let q = query(endpoint)
        let status = SecItemDelete(q as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw KeychainError(status: status) }
        guard let key, !key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        var add = q
        add[kSecValueData as String] = Data(key.trimmingCharacters(in: .whitespacesAndNewlines).utf8)
        add[kSecAttrLabel as String] = "Romlens tutor: \(endpoint)"
        add[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlocked
        let added = SecItemAdd(add as CFDictionary, nil)
        guard added == errSecSuccess else { throw KeychainError(status: added) }
    }
}

/// Keys in memory, for tests.
final class MemoryKeyStore: KeyStore, @unchecked Sendable {
    private let lock = NSLock()
    private var keys: [String: String] = [:]
    func key(for endpoint: String) -> String? { lock.withLock { keys[endpoint] } }
    func setKey(_ key: String?, for endpoint: String) throws { lock.withLock { keys[endpoint] = key } }
}

/// What the core asks for a key through, from the turn's thread.
final class TutorCredentials: CredentialStore, Sendable {
    let store: any KeyStore
    init(store: any KeyStore) { self.store = store }
    func key(endpoint: String) -> String? { store.key(for: endpoint) }
}
