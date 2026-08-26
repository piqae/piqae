import Foundation
import Security

public enum PiqaeKeychainError: Error, LocalizedError, Sendable {
    case unexpectedData
    case operationFailed(OSStatus)

    public var errorDescription: String? {
        switch self {
        case .unexpectedData: "The installation identity in Keychain is invalid."
        case let .operationFailed(status): "Keychain operation failed with status \(status)."
        }
    }
}

public actor PiqaeKeychainInstallationIdentityStore: PiqaeInstallationIdentityStore {
    private let service: String
    private let account: String

    public init(
        service: String = "com.piqae.nodekit",
        account: String = "installation-id"
    ) {
        self.service = service
        self.account = account
    }

    public func loadOrCreateInstallationID() async throws -> PiqaeInstallationID {
        if let existing = try load() { return existing }

        let created = PiqaeInstallationID(rawValue: "ins_apple_\(UUID().uuidString.lowercased())")
        let data = Data(created.rawValue.utf8)
        var query = baseQuery
        query[kSecValueData as String] = data
        query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let status = SecItemAdd(query as CFDictionary, nil)

        if status == errSecSuccess { return created }
        if status == errSecDuplicateItem, let winner = try load() { return winner }
        throw PiqaeKeychainError.operationFailed(status)
    }

    private var baseQuery: [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }

    private func load() throws -> PiqaeInstallationID? {
        var query = baseQuery
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw PiqaeKeychainError.operationFailed(status) }
        guard
            let data = result as? Data,
            let rawValue = String(data: data, encoding: .utf8),
            rawValue.hasPrefix("ins_apple_")
        else {
            throw PiqaeKeychainError.unexpectedData
        }
        return PiqaeInstallationID(rawValue: rawValue)
    }
}
