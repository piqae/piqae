import CryptoKit
import Foundation
import Security

public struct PiqaeGeneratedConnectorKey: Sendable {
    public let handle: Data
    public let publicKey: Data

    public init(handle: Data, publicKey: Data) {
        self.handle = handle
        self.publicKey = publicKey
    }
}

/// Non-exporting signing-key boundary used by the shared connector worker.
/// Handles are opaque; private key bytes never cross the callback ABI.
public protocol PiqaeConnectorKeyStore: Sendable {
    func generate(applicationScope: Data) throws -> PiqaeGeneratedConnectorKey
    func sign(handle: Data, message: Data) throws -> Data
    func delete(handle: Data) throws
}

public final class PiqaeKeychainConnectorKeyStore: @unchecked Sendable, PiqaeConnectorKeyStore {
    private let lock = NSLock()
    private let service: String

    public init(service: String = "com.piqae.nodekit.connector-signing.v1") {
        self.service = service
    }

    public func generate(applicationScope: Data) throws -> PiqaeGeneratedConnectorKey {
        try lock.withLock {
            guard (1 ... 255).contains(applicationScope.count) else {
                throw PiqaeNativeRuntimeError.keyUnavailable
            }
            let scopeDigest = SHA256.hash(data: applicationScope)
                .map { String(format: "%02x", $0) }.joined()
            let scopeAccount = "scope.\(scopeDigest)"
            if let handle = try load(account: scopeAccount),
                let privateBytes = try load(account: keyAccount(handle)),
                let key = try? Curve25519.Signing.PrivateKey(rawRepresentation: privateBytes)
            {
                return .init(handle: handle, publicKey: key.publicKey.rawRepresentation)
            }

            let key = Curve25519.Signing.PrivateKey()
            let handle = Data("ckh_\(UUID().uuidString.lowercased())".utf8)
            try store(key.rawRepresentation, account: keyAccount(handle))
            do {
                try store(handle, account: scopeAccount)
            } catch {
                try? remove(account: keyAccount(handle))
                throw error
            }
            return .init(handle: handle, publicKey: key.publicKey.rawRepresentation)
        }
    }

    public func sign(handle: Data, message: Data) throws -> Data {
        try lock.withLock {
            guard (1 ... 512).contains(handle.count), (1 ... 65_536).contains(message.count),
                let privateBytes = try load(account: keyAccount(handle))
            else { throw PiqaeNativeRuntimeError.keyUnavailable }
            let key = try Curve25519.Signing.PrivateKey(rawRepresentation: privateBytes)
            return try key.signature(for: message)
        }
    }

    public func delete(handle: Data) throws {
        try lock.withLock {
            guard (1 ... 512).contains(handle.count) else {
                throw PiqaeNativeRuntimeError.keyUnavailable
            }
            try remove(account: keyAccount(handle))
        }
    }

    private func keyAccount(_ handle: Data) -> String {
        "key.\(handle.base64EncodedString())"
    }

    private func baseQuery(account: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrSynchronizable as String: kCFBooleanFalse as Any,
        ]
    }

    private func load(account: String) throws -> Data? {
        var query = baseQuery(account: account)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data else {
            throw PiqaeNativeRuntimeError.keyUnavailable
        }
        return data
    }

    private func store(_ data: Data, account: String) throws {
        var query = baseQuery(account: account)
        query[kSecValueData as String] = data
        query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let status = SecItemAdd(query as CFDictionary, nil)
        if status == errSecSuccess { return }
        if status == errSecDuplicateItem {
            let updated = SecItemUpdate(
                baseQuery(account: account) as CFDictionary,
                [kSecValueData as String: data] as CFDictionary
            )
            guard updated == errSecSuccess else { throw PiqaeNativeRuntimeError.keyUnavailable }
            return
        }
        throw PiqaeNativeRuntimeError.keyUnavailable
    }

    private func remove(account: String) throws {
        let status = SecItemDelete(baseQuery(account: account) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw PiqaeNativeRuntimeError.keyUnavailable
        }
    }
}

final class PiqaeConnectorKeyCallbackContext: @unchecked Sendable {
    let store: any PiqaeConnectorKeyStore
    init(store: any PiqaeConnectorKeyStore) { self.store = store }
}

typealias PiqaeAppleGenerateConnectorKeyCallback = @convention(c) (
    UnsafeMutableRawPointer?, UnsafePointer<UInt8>?, Int, UnsafeMutablePointer<UInt8>?, Int,
    UnsafeMutablePointer<Int>?, UnsafeMutablePointer<UInt8>?, Int
) -> Int32
typealias PiqaeAppleSignConnectorCallback = @convention(c) (
    UnsafeMutableRawPointer?, UnsafePointer<UInt8>?, Int, UnsafePointer<UInt8>?, Int,
    UnsafeMutablePointer<UInt8>?, Int
) -> Int32
typealias PiqaeAppleDeleteConnectorKeyCallback = @convention(c) (
    UnsafeMutableRawPointer?, UnsafePointer<UInt8>?, Int
) -> Int32

let piqaeAppleGenerateConnectorKey: PiqaeAppleGenerateConnectorKeyCallback = {
    context, scope, scopeLength, handleOutput, handleCapacity, handleLength, publicOutput,
        publicLength in
    guard let context, let scope, let handleOutput, let handleLength, let publicOutput,
        (1 ... 255).contains(scopeLength), handleCapacity >= 1, handleCapacity <= 512,
        publicLength == 32
    else { return 1 }
    let callback = Unmanaged<PiqaeConnectorKeyCallbackContext>
        .fromOpaque(context).takeUnretainedValue()
    do {
        let generated = try callback.store.generate(
            applicationScope: Data(bytes: scope, count: scopeLength)
        )
        guard !generated.handle.isEmpty, generated.handle.count <= handleCapacity,
            generated.publicKey.count == 32
        else { return 1 }
        generated.handle.copyBytes(to: handleOutput, count: generated.handle.count)
        generated.publicKey.copyBytes(to: publicOutput, count: 32)
        handleLength.pointee = generated.handle.count
        return 0
    } catch { return 1 }
}

let piqaeAppleSignConnector: PiqaeAppleSignConnectorCallback = {
    context, handle, handleLength, message, messageLength, output, outputLength in
    guard let context, let handle, let message, let output,
        (1 ... 512).contains(handleLength), (1 ... 65_536).contains(messageLength),
        outputLength == 64
    else { return 1 }
    let callback = Unmanaged<PiqaeConnectorKeyCallbackContext>
        .fromOpaque(context).takeUnretainedValue()
    do {
        let signature = try callback.store.sign(
            handle: Data(bytes: handle, count: handleLength),
            message: Data(bytes: message, count: messageLength)
        )
        guard signature.count == 64 else { return 1 }
        signature.copyBytes(to: output, count: 64)
        return 0
    } catch { return 1 }
}

let piqaeAppleDeleteConnectorKey: PiqaeAppleDeleteConnectorKeyCallback = {
    context, handle, handleLength in
    guard let context, let handle, (1 ... 512).contains(handleLength) else { return 1 }
    let callback = Unmanaged<PiqaeConnectorKeyCallbackContext>
        .fromOpaque(context).takeUnretainedValue()
    do {
        try callback.store.delete(handle: Data(bytes: handle, count: handleLength))
        return 0
    } catch { return 1 }
}
