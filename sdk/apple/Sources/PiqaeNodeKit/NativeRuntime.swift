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
public actor PiqaeNativeRuntime: PiqaeEmbeddedNodeRuntime, PiqaeOpaqueIdentityProvider {
    public static var linkedLibraryAvailable: Bool { piqae_node_link_anchor() != 0 }
    private let configuration: PiqaeNativeRuntimeConfiguration
    private let keyStore: any PiqaeHostKeyStore
    private let connectorKeyStore: any PiqaeConnectorKeyStore
    private var library: PiqaeNativeLibrary?
    private var handle: UInt64?
    private var keyContext: PiqaeHostKeyCallbackContext?
    private var connectorKeyContext: PiqaeConnectorKeyCallbackContext?
    private var workAvailableHandler: (@Sendable () -> Void)?
    private var workAvailableContext: PiqaeWorkAvailableCallbackContext?

    public init(
        configuration: PiqaeNativeRuntimeConfiguration,
        keyStore: (any PiqaeHostKeyStore)? = nil,
        connectorKeyStore: (any PiqaeConnectorKeyStore)? = nil
    ) {
        self.configuration = configuration
        self.keyStore = keyStore ?? PiqaeKeychainHostKeyStore(
            account: "host-hmac-sha256-v1.\(configuration.applicationID)"
        )
        self.connectorKeyStore = connectorKeyStore ?? PiqaeKeychainConnectorKeyStore(
            service: "com.piqae.nodekit.connector-signing.v1.\(configuration.applicationID)"
        )
    }

    public func start() async throws {
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
            let connectorContext = PiqaeConnectorKeyCallbackContext(store: connectorKeyStore)
            let connectorProvider = PiqaeConnectorKeyProvider(
                context: Unmanaged.passUnretained(connectorContext).toOpaque(),
                generate: piqaeAppleGenerateConnectorKey,
                sign: piqaeAppleSignConnector,
                delete_key: piqaeAppleDeleteConnectorKey
            )
            _ = try Self.unwrap(
                library.setConnectorKeyProvider(createdData.handle, connectorProvider)
            ) as HandleData
            let workContext = PiqaeWorkAvailableCallbackContext(
                handler: workAvailableHandler ?? {}
            )
            let workProvider = PiqaeWorkAvailableProvider(
                context: Unmanaged.passUnretained(workContext).toOpaque(),
                notify: piqaeAppleWorkAvailable
            )
            _ = try Self.unwrap(
                library.setWorkAvailableProvider(createdData.handle, workProvider)
            ) as HandleData
            _ = try Self.unwrap(library.start(createdData.handle)) as NativeSnapshot
            self.library = library
            handle = createdData.handle
            self.keyContext = keyContext
            connectorKeyContext = connectorContext
            workAvailableContext = workContext
        } catch {
            _ = try? Self.unwrap(library.destroy(createdData.handle)) as DestroyData
            throw error
        }
    }

    public func stop() async throws {
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
        connectorKeyContext = nil
        workAvailableContext = nil
        self.library = nil
        if let destroyError { throw destroyError }
        if let stopError { throw stopError }
    }

    public func setWorkAvailableHandler(
        _ handler: @escaping @Sendable () -> Void
    ) async throws {
        guard handle == nil else {
            throw PiqaeNativeRuntimeError.rejected(
                code: "runtime_started",
                message: "The work-available handler must be installed before start."
            )
        }
        workAvailableHandler = handler
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

    public func registerAdapter(_ registration: PiqaeRuntimeAdapterRegistration) async throws {
        _ = try commandResponse(
            RegisterAdapterCommand(type: "register_adapter", registration: registration),
            as: RegisteredData.self
        )
    }

    public func observePrinterInventory(
        adapterID: String,
        printers: [PiqaeRuntimePrinterObservation]
    ) async throws -> [PiqaeRuntimePrinterSnapshot] {
        let response = try commandResponse(
            ObserveInventoryCommand(
                type: "observe_printer_inventory",
                adapterID: adapterID,
                printers: printers
            ),
            as: PrinterInventoryData.self
        )
        return response.printers
    }

    public func printerInventory() async throws -> [PiqaeRuntimePrinterSnapshot] {
        try commandResponse(TypeOnlyCommand(type: "printer_inventory"), as: PrinterInventoryData.self)
            .printers
    }

    public func enqueue(_ request: PiqaeRuntimeJobRequest) async throws -> PiqaeRuntimeJobAccepted {
        let response = try commandResponse(
            EnqueueJobCommand(request: request),
            as: JobAcceptedData.self
        )
        return response.job
    }

    public func nextOperation(adapterID: String) async throws -> PiqaeRuntimeAdapterOperation? {
        try commandResponse(
            AdapterIDCommand(type: "next_adapter_operation", adapterID: adapterID),
            as: AdapterOperationData.self
        ).operation
    }

    public func nativeObservations(adapterID: String) async throws
        -> [PiqaeRuntimeAdapterOperation]
    {
        try commandResponse(
            AdapterIDCommand(type: "adapter_observations", adapterID: adapterID),
            as: AdapterObservationsData.self
        ).operations
    }

    public func beginHandoff(_ operation: PiqaeRuntimeAdapterOperation) async throws
        -> PiqaeRuntimeAdapterOperation
    {
        let response = try commandResponse(
            AdapterOperationCommand(
                type: "begin_adapter_handoff",
                adapterID: operation.adapterID,
                operationID: operation.operationID,
                fence: operation.fence
            ),
            as: AdapterOperationData.self
        )
        guard let started = response.operation else { throw PiqaeNativeRuntimeError.invalidResponse }
        return started
    }

    public func complete(
        _ operation: PiqaeRuntimeAdapterOperation,
        outcome: PiqaeRuntimeAdapterOutcome
    ) async throws -> PiqaeRuntimeAdapterAcknowledgement {
        try commandResponse(
            CompleteOperationCommand(
                type: "complete_adapter_operation",
                adapterID: operation.adapterID,
                operationID: operation.operationID,
                fence: operation.fence,
                result: outcome
            ),
            as: AdapterAcknowledgementData.self
        ).acknowledgement
    }

    public func job(id: PiqaeJobID) async throws -> PiqaeRuntimeJobSnapshot {
        try commandResponse(
            JobIDCommand(type: "job_snapshot", jobID: id.rawValue),
            as: JobSnapshotData.self
        ).job
    }

    public func profiles(printerID: PiqaePrinterID) async throws
        -> [PiqaeRuntimeProfileSnapshot]
    {
        try commandResponse(
            PrinterIDCommand(type: "profile_snapshots", printerID: printerID.rawValue),
            as: ProfileSnapshotsData.self
        ).profiles
    }

    public func createProfile(_ request: PiqaeRuntimeProfileCreateRequest) async throws
        -> PiqaeRuntimeProfileSnapshot
    {
        try commandResponse(
            CreateProfileCommand(request: request),
            as: ProfileSnapshotData.self
        ).profile
    }

    public func updateProfile(_ request: PiqaeRuntimeProfileUpdateRequest) async throws
        -> PiqaeRuntimeProfileSnapshot
    {
        try commandResponse(
            UpdateProfileCommand(request: request),
            as: ProfileSnapshotData.self
        ).profile
    }

    public func deleteProfile(
        printerID: PiqaePrinterID,
        profileID: PiqaeProfileID,
        expectedRevision: UInt64
    ) async throws {
        _ = try commandResponse(
            DeleteProfileCommand(
                type: "delete_profile",
                printerID: printerID.rawValue,
                profileID: profileID.rawValue,
                expectedRevision: expectedRevision
            ),
            as: DeletedData.self
        )
    }

    public func connectors() async throws -> [PiqaeRuntimeConnectorSnapshot] {
        try commandResponse(
            TypeOnlyCommand(type: "connector_snapshots"),
            as: ConnectorSnapshotsData.self
        ).connectors
    }

    public func connectInvitation(_ request: PiqaeEnrollmentRequest) async throws
        -> PiqaeRuntimeConnectorSnapshot
    {
        let prepared = try commandResponse(
            PrepareConnectorKeyCommand(
                type: "prepare_connector_key",
                applicationScope: configuration.applicationID
            ),
            as: PreparedConnectorKeyData.self
        )
        do {
            let token = try request.invitation.withValue { value in
                guard value.utf8.count <= 4_096 else {
                    throw PiqaeNodeError.invalidConfiguration(
                        "The invitation exceeds the supported size."
                    )
                }
                return value
            }
            return try commandResponse(
                ConnectInvitationCommand(
                    type: "connect_invitation",
                    controlPlaneURL: request.authorityURL,
                    invitationToken: token,
                    connectorKeyHandle: prepared.keyHandle,
                    printerGrant: "all_local_printers",
                    allowedPrinterIDs: [],
                    nodeName: configuration.applicationID,
                    hostname: ProcessInfo.processInfo.hostName
                ),
                as: ConnectedConnectorData.self
            ).connector
        } catch {
            _ = try? commandResponse(
                CancelPreparedConnectorKeyCommand(
                    type: "cancel_prepared_connector_key",
                    keyHandle: prepared.keyHandle
                ),
                as: CancelledConnectorKeyData.self
            )
            throw error
        }
    }

    public func revokeConnector(id: PiqaeConnectionID) async throws {
        _ = try commandResponse(
            ConnectorIDCommand(type: "revoke_connector", connectorID: id.rawValue),
            as: RevokedData.self
        )
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

private final class PiqaeWorkAvailableCallbackContext: @unchecked Sendable {
    private let handler: @Sendable () -> Void

    init(handler: @escaping @Sendable () -> Void) {
        self.handler = handler
    }

    func notify() { handler() }
}

private let piqaeAppleWorkAvailable: PiqaeWorkAvailableCallback = { context in
    guard let context else { return }
    Unmanaged<PiqaeWorkAvailableCallbackContext>
        .fromOpaque(context).takeUnretainedValue().notify()
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

private struct TypeOnlyCommand: Encodable { let type: String }

private struct RegisterAdapterCommand: Encodable {
    let type: String
    let registration: PiqaeRuntimeAdapterRegistration
}

private struct ObserveInventoryCommand: Encodable {
    let type: String
    let adapterID: String
    let printers: [PiqaeRuntimePrinterObservation]
    enum CodingKeys: String, CodingKey {
        case type
        case adapterID = "adapter_id"
        case printers
    }
}

private struct AdapterIDCommand: Encodable {
    let type: String
    let adapterID: String
    enum CodingKeys: String, CodingKey { case type; case adapterID = "adapter_id" }
}

private struct AdapterOperationCommand: Encodable {
    let type: String
    let adapterID: String
    let operationID: String
    let fence: String
    enum CodingKeys: String, CodingKey {
        case type
        case adapterID = "adapter_id"
        case operationID = "operation_id"
        case fence
    }
}

private struct CompleteOperationCommand: Encodable {
    let type: String
    let adapterID: String
    let operationID: String
    let fence: String
    let result: PiqaeRuntimeAdapterOutcome
    enum CodingKeys: String, CodingKey {
        case type
        case adapterID = "adapter_id"
        case operationID = "operation_id"
        case fence, result
    }
}

private struct EnqueueJobCommand: Encodable {
    let type = "enqueue_local_job"
    let adapterID: String
    let idempotencyKey: String
    let printerID: String
    let title: String
    let contentKind: String
    let contentBase64: String
    let optionsJSON: String
    let expiresUnixMilliseconds: Int64?

    init(request: PiqaeRuntimeJobRequest) {
        adapterID = request.adapterID
        idempotencyKey = request.idempotencyKey
        printerID = request.printerID.rawValue
        title = request.title
        contentKind = request.contentKind
        contentBase64 = request.content.base64EncodedString()
        optionsJSON = request.optionsJSON
        expiresUnixMilliseconds = request.expiresUnixMilliseconds
    }

    enum CodingKeys: String, CodingKey {
        case type
        case adapterID = "adapter_id"
        case idempotencyKey = "idempotency_key"
        case printerID = "printer_id"
        case title
        case contentKind = "content_kind"
        case contentBase64 = "content_base64"
        case optionsJSON = "options_json"
        case expiresUnixMilliseconds = "expires_unix_ms"
    }
}

private struct JobIDCommand: Encodable {
    let type: String
    let jobID: String
    enum CodingKeys: String, CodingKey { case type; case jobID = "job_id" }
}

private struct PrinterIDCommand: Encodable {
    let type: String
    let printerID: String
    enum CodingKeys: String, CodingKey { case type; case printerID = "printer_id" }
}

private struct CreateProfileCommand: Encodable {
    let type = "create_profile"
    let printerID: String
    let name: String
    let isDefault: Bool
    let optionsJSON: String
    init(request: PiqaeRuntimeProfileCreateRequest) {
        printerID = request.printerID.rawValue
        name = request.name
        isDefault = request.isDefault
        optionsJSON = request.optionsJSON
    }
    enum CodingKeys: String, CodingKey {
        case type
        case printerID = "printer_id"
        case name
        case isDefault = "is_default"
        case optionsJSON = "options_json"
    }
}

private struct UpdateProfileCommand: Encodable {
    let type = "update_profile"
    let printerID: String
    let profileID: String
    let expectedRevision: UInt64
    let name: String
    let isDefault: Bool
    let optionsJSON: String
    init(request: PiqaeRuntimeProfileUpdateRequest) {
        printerID = request.printerID.rawValue
        profileID = request.profileID.rawValue
        expectedRevision = request.expectedRevision
        name = request.name
        isDefault = request.isDefault
        optionsJSON = request.optionsJSON
    }
    enum CodingKeys: String, CodingKey {
        case type
        case printerID = "printer_id"
        case profileID = "profile_id"
        case expectedRevision = "expected_revision"
        case name
        case isDefault = "is_default"
        case optionsJSON = "options_json"
    }
}

private struct DeleteProfileCommand: Encodable {
    let type: String
    let printerID: String
    let profileID: String
    let expectedRevision: UInt64
    enum CodingKeys: String, CodingKey {
        case type
        case printerID = "printer_id"
        case profileID = "profile_id"
        case expectedRevision = "expected_revision"
    }
}

private struct ConnectorIDCommand: Encodable {
    let type: String
    let connectorID: String
    enum CodingKeys: String, CodingKey { case type; case connectorID = "connector_id" }
}
private struct PrepareConnectorKeyCommand: Encodable {
    let type: String
    let applicationScope: String
    enum CodingKeys: String, CodingKey {
        case type
        case applicationScope = "application_scope"
    }
}
private struct CancelPreparedConnectorKeyCommand: Encodable {
    let type: String
    let keyHandle: String
    enum CodingKeys: String, CodingKey {
        case type
        case keyHandle = "key_handle"
    }
}
private struct ConnectInvitationCommand: Encodable {
    let type: String
    let controlPlaneURL: URL
    let invitationToken: String
    let connectorKeyHandle: String
    let printerGrant: String
    let allowedPrinterIDs: [String]
    let nodeName: String
    let hostname: String
    enum CodingKeys: String, CodingKey {
        case type
        case controlPlaneURL = "control_plane_url"
        case invitationToken = "invitation_token"
        case connectorKeyHandle = "connector_key_handle"
        case printerGrant = "printer_grant"
        case allowedPrinterIDs = "allowed_printer_ids"
        case nodeName = "node_name"
        case hostname
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
private struct RegisteredData: Decodable { let registered: Bool }
private struct PrinterInventoryData: Decodable { let printers: [PiqaeRuntimePrinterSnapshot] }
private struct JobAcceptedData: Decodable { let job: PiqaeRuntimeJobAccepted }
private struct AdapterOperationData: Decodable { let operation: PiqaeRuntimeAdapterOperation? }
private struct AdapterObservationsData: Decodable {
    let operations: [PiqaeRuntimeAdapterOperation]
}
private struct AdapterAcknowledgementData: Decodable {
    let acknowledgement: PiqaeRuntimeAdapterAcknowledgement
}
private struct JobSnapshotData: Decodable { let job: PiqaeRuntimeJobSnapshot }
private struct ProfileSnapshotsData: Decodable { let profiles: [PiqaeRuntimeProfileSnapshot] }
private struct ProfileSnapshotData: Decodable { let profile: PiqaeRuntimeProfileSnapshot }
private struct ConnectorSnapshotsData: Decodable { let connectors: [PiqaeRuntimeConnectorSnapshot] }
private struct PreparedConnectorKeyData: Decodable {
    let keyHandle: String
    enum CodingKeys: String, CodingKey { case keyHandle = "key_handle" }
}
private struct ConnectedConnectorData: Decodable { let connector: PiqaeRuntimeConnectorSnapshot }
private struct CancelledConnectorKeyData: Decodable { let cancelled: Bool }
private struct DeletedData: Decodable { let deleted: Bool }
private struct RevokedData: Decodable { let revoked: Bool }

private final class PiqaeNativeLibrary: @unchecked Sendable {
    private typealias AbiDescriptor = @convention(c) () -> PiqaeNodeAbiDescriptor
    private typealias InputOperation = @convention(c) (UnsafePointer<UInt8>?, Int) -> PiqaeBuffer
    private typealias HandleOperation = @convention(c) (UInt64) -> PiqaeBuffer
    private typealias ProviderOperation = @convention(c) (UInt64, PiqaeHostKeyProvider) -> PiqaeBuffer
    private typealias ConnectorProviderOperation = @convention(c) (
        UInt64, PiqaeConnectorKeyProvider
    ) -> PiqaeBuffer
    private typealias WorkAvailableProviderOperation = @convention(c) (
        UInt64, PiqaeWorkAvailableProvider
    ) -> PiqaeBuffer
    private typealias CommandOperation = @convention(c) (
        UInt64, UnsafePointer<UInt8>?, Int
    ) -> PiqaeBuffer
    private typealias FreeOperation = @convention(c) (PiqaeBuffer) -> Void

    private let dynamicHandle: UnsafeMutableRawPointer?
    private let descriptor: AbiDescriptor
    private let createOperation: InputOperation
    private let startOperation: HandleOperation
    private let providerOperation: ProviderOperation
    private let connectorProviderOperation: ConnectorProviderOperation
    private let workAvailableProviderOperation: WorkAvailableProviderOperation
    private let stopOperation: HandleOperation
    private let commandOperation: CommandOperation
    private let destroyOperation: HandleOperation
    private let freeOperation: FreeOperation

    init(url: URL?) throws {
        if url == nil {
            guard piqae_node_link_anchor() != 0 else {
                throw PiqaeNativeRuntimeError.libraryUnavailable
            }
            dynamicHandle = nil
            descriptor = piqae_node_linked_abi_descriptor
            createOperation = piqae_node_linked_create
            startOperation = piqae_node_linked_start
            providerOperation = piqae_node_linked_set_host_key_provider
            connectorProviderOperation = piqae_node_linked_set_connector_key_provider
            workAvailableProviderOperation = piqae_node_linked_set_work_available_provider
            stopOperation = piqae_node_linked_stop
            commandOperation = piqae_node_linked_command
            destroyOperation = piqae_node_linked_destroy
            freeOperation = piqae_node_linked_free
            return
        }
        guard let url else { throw PiqaeNativeRuntimeError.libraryUnavailable }
        let dynamicHandle = dlopen(url.path, RTLD_NOW | RTLD_LOCAL)
        guard let dynamicHandle else { throw PiqaeNativeRuntimeError.libraryUnavailable }
        do {
            descriptor = try Self.symbol(dynamicHandle, "piqae_node_abi_descriptor")
            createOperation = try Self.symbol(dynamicHandle, "piqae_node_create")
            startOperation = try Self.symbol(dynamicHandle, "piqae_node_start")
            providerOperation = try Self.symbol(dynamicHandle, "piqae_node_set_host_key_provider")
            connectorProviderOperation = try Self.symbol(
                dynamicHandle, "piqae_node_set_connector_key_provider"
            )
            workAvailableProviderOperation = try Self.symbol(
                dynamicHandle, "piqae_node_set_work_available_provider"
            )
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
    func setConnectorKeyProvider(_ handle: UInt64, _ provider: PiqaeConnectorKeyProvider) -> Data {
        read(connectorProviderOperation(handle, provider))
    }
    func setWorkAvailableProvider(
        _ handle: UInt64,
        _ provider: PiqaeWorkAvailableProvider
    ) -> Data {
        read(workAvailableProviderOperation(handle, provider))
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
