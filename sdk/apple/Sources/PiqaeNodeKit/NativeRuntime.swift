import CPiqaeNodeABI
import CryptoKit
import Darwin
import Foundation
import Security

public enum PiqaeNativeRuntimeError: Error, LocalizedError, Equatable, Sendable {
    case libraryUnavailable
    case incompatibleABI
    case invalidResponse
    case rejected(code: String, message: String)
    case keyUnavailable

    public var errorDescription: String? {
        switch self {
        case .libraryUnavailable: "The Piqae native runtime library is unavailable."
        case .incompatibleABI: "The Piqae native runtime ABI is incompatible with this SDK."
        case .invalidResponse: "The Piqae native runtime returned an invalid response."
        case let .rejected(_, message): message
        case .keyUnavailable: "The Piqae installation key is unavailable."
        }
    }
}

public struct PiqaeNativeRuntimeConfiguration: Sendable {
    public let applicationID: String
    public let dataDirectory: String
    public let hostMode: PiqaeNodeHostMode
    public let availability: PiqaeNodeAvailabilityClass
    public let localOnly: Bool
    public let libraryURL: URL?

    public init(
        applicationID: String,
        dataDirectory: String = "node-runtime",
        hostMode: PiqaeNodeHostMode = .embeddedApplication,
        availability: PiqaeNodeAvailabilityClass,
        localOnly: Bool,
        libraryURL: URL? = nil
    ) {
        self.applicationID = applicationID
        self.dataDirectory = dataDirectory
        self.hostMode = hostMode
        self.availability = availability
        self.localOnly = localOnly
        self.libraryURL = libraryURL
    }
}

/// Real allocator-neutral binding to `piqae-node-ffi`. This object owns the
/// native handle and Keychain callback context until `stop()` destroys it.
public actor PiqaeNativeRuntime: PiqaeHostLifecycleReporter, PiqaeOpaqueIdentityProvider {
    private let configuration: PiqaeNativeRuntimeConfiguration
    private let keyStore: any PiqaeHostKeyStore
    private var library: PiqaeNativeLibrary?
    private var handle: UInt64?
    private var keyContext: PiqaeHostKeyCallbackContext?

    public init(
        configuration: PiqaeNativeRuntimeConfiguration,
        keyStore: (any PiqaeHostKeyStore)? = nil
    ) {
        self.configuration = configuration
        self.keyStore = keyStore ?? PiqaeKeychainHostKeyStore(
            account: "host-hmac-sha256-v1.\(configuration.applicationID)"
        )
    }

    public func start() throws {
        guard handle == nil else { return }
        let library = try PiqaeNativeLibrary(url: configuration.libraryURL)
        let descriptor = library.abiDescriptor()
        guard descriptor.abi_version == 1, descriptor.contract_min <= 1,
            descriptor.contract_max >= 1
        else { throw PiqaeNativeRuntimeError.incompatibleABI }

        let request = NativeConfiguration(
            contract: 1,
            hostMode: configuration.hostMode,
            availability: configuration.availability,
            localOnly: configuration.localOnly,
            applicationID: configuration.applicationID,
            dataDirectory: configuration.dataDirectory
        )
        let created = library.create(try JSONEncoder().encode(request))
        let createdData: HandleData = try Self.unwrap(created)
        do {
            let key = try keyStore.loadOrCreateKey()
            let keyContext = PiqaeHostKeyCallbackContext(key: key)
            let provider = PiqaeHostKeyProvider(
                context: Unmanaged.passUnretained(keyContext).toOpaque(),
                hmac_sha256: piqaeAppleHMACSHA256
            )
            _ = try Self.unwrap(
                library.setHostKeyProvider(createdData.handle, provider)
            ) as HandleData
            _ = try Self.unwrap(library.start(createdData.handle)) as NativeSnapshot
            self.library = library
            handle = createdData.handle
            self.keyContext = keyContext
        } catch {
            _ = try? Self.unwrap(library.destroy(createdData.handle)) as DestroyData
            throw error
        }
    }

    public func stop() throws {
        guard let library, let handle else { return }
        let stopError: Error?
        do {
            _ = try Self.unwrap(library.stop(handle)) as NativeSnapshot
            stopError = nil
        } catch {
            stopError = error
        }
        let destroyError: Error?
        do {
            _ = try Self.unwrap(library.destroy(handle)) as DestroyData
            destroyError = nil
        } catch {
            destroyError = error
        }
        self.handle = nil
        keyContext = nil
        self.library = nil
        if let destroyError { throw destroyError }
        if let stopError { throw stopError }
    }

    public func report(_ event: PiqaeHostLifecycleEvent) async throws {
        let command = LifecycleCommand(type: "apply_lifecycle", event: event)
        _ = try commandResponse(command, as: NativeSnapshot.self)
    }

    public func deriveOpaqueID(
        namespace: String,
        canonicalIdentity: Data
    ) async throws -> String {
        guard canonicalIdentity.count <= 4_096,
            let canonical = String(data: canonicalIdentity, encoding: .utf8)
        else { throw PiqaeNativeRuntimeError.invalidResponse }
        let command = OpaqueEvidenceCommand(
            type: "derive_opaque_evidence",
            namespace: namespace,
            canonicalIdentity: canonical
        )
        let response = try commandResponse(command, as: OpaqueEvidenceData.self)
        return response.opaqueEvidence
    }

    private func commandResponse<Request: Encodable, Response: Decodable>(
        _ request: Request,
        as type: Response.Type
    ) throws -> Response {
        guard let library, let handle else {
            throw PiqaeNativeRuntimeError.rejected(
                code: "runtime_not_started",
                message: "The native runtime has not started."
            )
        }
        return try Self.unwrap(library.command(handle, try JSONEncoder().encode(request)))
    }

    private static func unwrap<Response: Decodable>(_ data: Data) throws -> Response {
        let envelope: NativeEnvelope<Response>
        do { envelope = try JSONDecoder().decode(NativeEnvelope<Response>.self, from: data) }
        catch { throw PiqaeNativeRuntimeError.invalidResponse }
        if envelope.ok, let value = envelope.data { return value }
        if let error = envelope.error {
            throw PiqaeNativeRuntimeError.rejected(code: error.code, message: error.message)
        }
        throw PiqaeNativeRuntimeError.invalidResponse
    }
}

public protocol PiqaeHostKeyStore: Sendable {
    func loadOrCreateKey() throws -> Data
}

public final class PiqaeKeychainHostKeyStore: @unchecked Sendable, PiqaeHostKeyStore {
    private let lock = NSLock()
    private let service: String
    private let account: String

    public init(
        service: String = "com.piqae.nodekit",
        account: String = "host-hmac-sha256-v1"
    ) {
        self.service = service
        self.account = account
    }

    public func loadOrCreateKey() throws -> Data {
        try lock.withLock {
            if let existing = try load() { return existing }
            var bytes = Data(count: 32)
            let status = bytes.withUnsafeMutableBytes { buffer in
                SecRandomCopyBytes(kSecRandomDefault, buffer.count, buffer.baseAddress!)
            }
            guard status == errSecSuccess else { throw PiqaeNativeRuntimeError.keyUnavailable }
            var query = baseQuery
            query[kSecValueData as String] = bytes
            query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            let addStatus = SecItemAdd(query as CFDictionary, nil)
            if addStatus == errSecSuccess { return bytes }
            if addStatus == errSecDuplicateItem, let winner = try load() { return winner }
            throw PiqaeNativeRuntimeError.keyUnavailable
        }
    }

    private var baseQuery: [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }

    private func load() throws -> Data? {
        var query = baseQuery
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data, data.count == 32 else {
            throw PiqaeNativeRuntimeError.keyUnavailable
        }
        return data
    }
}

private final class PiqaeHostKeyCallbackContext: @unchecked Sendable {
    let key: SymmetricKey
    init(key: Data) { self.key = SymmetricKey(data: key) }
}

private let piqaeAppleHMACSHA256: PiqaeHmacSha256Callback = {
    context, scope, scopeLength, message, messageLength, output, outputLength in
    guard
        let context, let scope, let message, let output,
        (1 ... 128).contains(scopeLength),
        (1 ... 4_096).contains(messageLength),
        outputLength == 32
    else { return 1 }
    let callback = Unmanaged<PiqaeHostKeyCallbackContext>
        .fromOpaque(context).takeUnretainedValue()
    let scopeData = Data(bytes: scope, count: scopeLength)
    let messageData = Data(bytes: message, count: messageLength)
    var input = Data("piqae-host-key-scope-v1\0".utf8)
    input.append(scopeData)
    input.append(0)
    input.append(messageData)
    let digest = HMAC<SHA256>.authenticationCode(for: input, using: callback.key)
    Data(digest).copyBytes(to: output, count: 32)
    return 0
}

private struct NativeConfiguration: Encodable {
    let contract: UInt16
    let hostMode: PiqaeNodeHostMode
    let availability: PiqaeNodeAvailabilityClass
    let localOnly: Bool
    let applicationID: String
    let dataDirectory: String

    enum CodingKeys: String, CodingKey {
        case contract
        case hostMode = "host_mode"
        case availability
        case localOnly = "local_only"
        case applicationID = "application_id"
        case dataDirectory = "data_directory"
    }
}

private struct LifecycleCommand: Encodable {
    let type: String
    let event: PiqaeHostLifecycleEvent
}

private struct OpaqueEvidenceCommand: Encodable {
    let type: String
    let namespace: String
    let canonicalIdentity: String

    enum CodingKeys: String, CodingKey {
        case type
        case namespace
        case canonicalIdentity = "canonical_identity"
    }
}

private struct NativeEnvelope<Value: Decodable>: Decodable {
    let ok: Bool
    let data: Value?
    let error: NativeErrorData?
}

private struct NativeErrorData: Decodable {
    let code: String
    let message: String
}

private struct HandleData: Decodable { let handle: UInt64 }
private struct DestroyData: Decodable { let destroyed: Bool }
private struct NativeSnapshot: Decodable { let handle: UInt64 }
private struct OpaqueEvidenceData: Decodable {
    let opaqueEvidence: String
    enum CodingKeys: String, CodingKey { case opaqueEvidence = "opaque_evidence" }
}

private final class PiqaeNativeLibrary: @unchecked Sendable {
    private typealias AbiDescriptor = @convention(c) () -> PiqaeNodeAbiDescriptor
    private typealias InputOperation = @convention(c) (UnsafePointer<UInt8>?, Int) -> PiqaeBuffer
    private typealias HandleOperation = @convention(c) (UInt64) -> PiqaeBuffer
    private typealias ProviderOperation = @convention(c) (UInt64, PiqaeHostKeyProvider) -> PiqaeBuffer
    private typealias CommandOperation = @convention(c) (
        UInt64, UnsafePointer<UInt8>?, Int
    ) -> PiqaeBuffer
    private typealias FreeOperation = @convention(c) (PiqaeBuffer) -> Void

    private let dynamicHandle: UnsafeMutableRawPointer?
    private let descriptor: AbiDescriptor
    private let createOperation: InputOperation
    private let startOperation: HandleOperation
    private let providerOperation: ProviderOperation
    private let stopOperation: HandleOperation
    private let commandOperation: CommandOperation
    private let destroyOperation: HandleOperation
    private let freeOperation: FreeOperation

    init(url: URL?) throws {
        let dynamicHandle: UnsafeMutableRawPointer?
        if let url { dynamicHandle = dlopen(url.path, RTLD_NOW | RTLD_LOCAL) }
        else { dynamicHandle = dlopen(nil, RTLD_NOW | RTLD_LOCAL) }
        guard let dynamicHandle else { throw PiqaeNativeRuntimeError.libraryUnavailable }
        do {
            descriptor = try Self.symbol(dynamicHandle, "piqae_node_abi_descriptor")
            createOperation = try Self.symbol(dynamicHandle, "piqae_node_create")
            startOperation = try Self.symbol(dynamicHandle, "piqae_node_start")
            providerOperation = try Self.symbol(dynamicHandle, "piqae_node_set_host_key_provider")
            stopOperation = try Self.symbol(dynamicHandle, "piqae_node_stop")
            commandOperation = try Self.symbol(dynamicHandle, "piqae_node_command")
            destroyOperation = try Self.symbol(dynamicHandle, "piqae_node_destroy")
            freeOperation = try Self.symbol(dynamicHandle, "piqae_node_free")
        } catch {
            dlclose(dynamicHandle)
            throw error
        }
        self.dynamicHandle = dynamicHandle
    }

    deinit { if let dynamicHandle { dlclose(dynamicHandle) } }

    func abiDescriptor() -> PiqaeNodeAbiDescriptor { descriptor() }
    func create(_ data: Data) -> Data { call(data, createOperation) }
    func start(_ handle: UInt64) -> Data { read(startOperation(handle)) }
    func setHostKeyProvider(_ handle: UInt64, _ provider: PiqaeHostKeyProvider) -> Data {
        read(providerOperation(handle, provider))
    }
    func stop(_ handle: UInt64) -> Data { read(stopOperation(handle)) }
    func command(_ handle: UInt64, _ data: Data) -> Data {
        data.withUnsafeBytes { buffer in
            read(commandOperation(handle, buffer.bindMemory(to: UInt8.self).baseAddress, data.count))
        }
    }
    func destroy(_ handle: UInt64) -> Data { read(destroyOperation(handle)) }

    private func call(_ data: Data, _ operation: InputOperation) -> Data {
        data.withUnsafeBytes { buffer in
            read(operation(buffer.bindMemory(to: UInt8.self).baseAddress, data.count))
        }
    }

    private func read(_ buffer: PiqaeBuffer) -> Data {
        defer { freeOperation(buffer) }
        guard let bytes = buffer.data, buffer.length > 0, buffer.length <= 1_048_576 else {
            return Data()
        }
        return Data(bytes: bytes, count: buffer.length)
    }

    private static func symbol<Function>(
        _ handle: UnsafeMutableRawPointer,
        _ name: String
    ) throws -> Function {
        guard let pointer = dlsym(handle, name) else {
            throw PiqaeNativeRuntimeError.libraryUnavailable
        }
        return unsafeBitCast(pointer, to: Function.self)
    }
}
