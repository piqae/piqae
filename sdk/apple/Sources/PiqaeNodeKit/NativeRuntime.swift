import CPiqaeNodeABI
import CryptoKit
import Darwin
import Foundation
import Security

public enum PiqaeNativeRuntimeError: Error, LocalizedError, Equatable, Sendable {
    case libraryUnavailable
    case incompatibleABI
    case invalidResponse
    /// The loaded native core predates an operation exposed by this SDK.
    /// Applications must update the bundled core; NodeKit never falls back to
    /// a second renderer, queue, or cloud implementation.
    case nativeCoreUpdateRequired
    case rejected(code: String, message: String)
    case keyUnavailable
    case nodeIdentityRevisionConflict(currentRevision: UInt64)

    public var errorDescription: String? {
        switch self {
        case .libraryUnavailable: "The Piqae native runtime library is unavailable."
        case .incompatibleABI: "The Piqae native runtime ABI is incompatible with this SDK."
        case .invalidResponse: "The Piqae native runtime returned an invalid response."
        case .nativeCoreUpdateRequired:
            "The bundled Piqae native runtime must be updated to use this SDK operation."
        case let .rejected(_, message): message
        case .keyUnavailable: "The Piqae installation key is unavailable."
        case .nodeIdentityRevisionConflict:
            "The node details changed elsewhere. Review them before saving again."
        }
    }
}

public struct PiqaeNativeRuntimeConfiguration: Sendable {
    public let applicationID: String
    public let dataDirectory: String
    public let hostMode: PiqaeNodeHostMode
    public let availability: PiqaeNodeAvailabilityClass
    public let localOnly: Bool
    /// Operator-visible name used only when enrolling a new connector.
    public let nodeName: String
    /// Bounded platform hint. iOS defaults to a generic value rather than the
    /// user-assigned device name or a login-derived hostname.
    public let hostname: String
    /// Portable host ownership, connection, and operator-visible identity
    /// contract. Older callers may omit it; identity editing is then
    /// unavailable rather than synthesized from connector credentials.
    public let hostConfiguration: PiqaeHostConfiguration?
    public let libraryURL: URL?

    public init(
        applicationID: String,
        dataDirectory: String = "node-runtime",
        hostMode: PiqaeNodeHostMode = .embeddedApplication,
        availability: PiqaeNodeAvailabilityClass,
        localOnly: Bool,
        nodeName: String? = nil,
        hostname: String? = nil,
        hostConfiguration: PiqaeHostConfiguration? = nil,
        libraryURL: URL? = nil
    ) {
        self.applicationID = applicationID
        self.dataDirectory = dataDirectory
        self.hostMode = hostMode
        self.availability = availability
        self.localOnly = localOnly
        let proposedName = nodeName?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        self.nodeName = Self.boundedUTF8(
            proposedName.isEmpty ? applicationID : proposedName,
            maximumBytes: 120
        )
        let proposedHostname = hostname?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        #if os(iOS)
        self.hostname = Self.boundedUTF8(
            proposedHostname.isEmpty ? "ios-application-host" : proposedHostname,
            maximumBytes: 120
        )
        #else
        self.hostname = Self.boundedUTF8(
            proposedHostname.isEmpty ? ProcessInfo.processInfo.hostName : proposedHostname,
            maximumBytes: 120
        )
        #endif
        self.hostConfiguration = hostConfiguration
        self.libraryURL = libraryURL
    }

    private static func boundedUTF8(_ value: String, maximumBytes: Int) -> String {
        var result = ""
        var usedBytes = 0
        for character in value {
            let characterBytes = String(character).utf8.count
            guard usedBytes + characterBytes <= maximumBytes else { break }
            result.append(character)
            usedBytes += characterBytes
        }
        return result
    }
}

/// Real allocator-neutral binding to `piqae-node-ffi`. This object owns the
/// native handle and Keychain callback context until `stop()` destroys it.
public actor PiqaeNativeRuntime: PiqaeEmbeddedNodeRuntime, PiqaeOpaqueIdentityProvider {
    public static var linkedLibraryAvailable: Bool { piqae_node_link_anchor() != 0 }
    public static let nativeABIVersion: UInt16 = 1
    public static let nativeContractVersion: UInt16 = 2
    static func supportsNativeContract(abi: UInt16, minimum: UInt16, maximum: UInt16) -> Bool {
        abi == nativeABIVersion
            && minimum == nativeContractVersion
            && maximum == nativeContractVersion
    }
    private let configuration: PiqaeNativeRuntimeConfiguration
    private let keyStore: any PiqaeHostKeyStore
    private let connectorKeyStore: any PiqaeConnectorKeyStore
    private var library: PiqaeNativeLibrary?
    private var handle: UInt64?
    private var keyContext: PiqaeHostKeyCallbackContext?
    private var connectorKeyContext: PiqaeConnectorKeyCallbackContext?
    private var workAvailableHandler: (@Sendable () -> Void)?
    private var workAvailableContext: PiqaeWorkAvailableCallbackContext?
    private var enrollmentNodeName: String

    public init(
        configuration: PiqaeNativeRuntimeConfiguration,
        keyStore: (any PiqaeHostKeyStore)? = nil,
        connectorKeyStore: (any PiqaeConnectorKeyStore)? = nil
    ) {
        self.configuration = configuration
        enrollmentNodeName = configuration.nodeName
        self.keyStore = keyStore ?? PiqaeKeychainHostKeyStore(
            account: "host-hmac-sha256-v1.\(configuration.applicationID)"
        )
        self.connectorKeyStore = connectorKeyStore ?? PiqaeKeychainConnectorKeyStore(
            service: "com.piqae.nodekit.connector-signing.v1.\(configuration.applicationID)"
        )
    }

    /// Updates only the operator-visible name used for future connector
    /// enrollment. Existing cloud resources are renamed through their scoped
    /// management API; this never rewrites credentials or queue identity.
    public func updateEnrollmentNodeName(_ value: String) throws {
        let value = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty, value.utf8.count <= 120 else {
            throw PiqaeNodeError.invalidConfiguration(
                "Node name must contain 1 to 120 UTF-8 bytes."
            )
        }
        enrollmentNodeName = value
    }

    public func start() async throws {
        guard handle == nil else { return }
        let library = try PiqaeNativeLibrary(url: configuration.libraryURL)
        let descriptor = library.abiDescriptor()
        guard Self.supportsNativeContract(
            abi: descriptor.abi_version,
            minimum: descriptor.contract_min,
            maximum: descriptor.contract_max
        )
        else { throw PiqaeNativeRuntimeError.incompatibleABI }

        let request = NativeConfiguration(
            contract: 2,
            hostMode: configuration.hostMode,
            availability: configuration.availability,
            localOnly: configuration.localOnly,
            applicationID: configuration.applicationID,
            dataDirectory: configuration.dataDirectory,
            hostConfiguration: configuration.hostConfiguration
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

    public func reconcileCloud(timeoutMilliseconds: UInt64) async throws -> Bool {
        let outcome = try await reconcileCloudOutcome(timeoutMilliseconds: timeoutMilliseconds)
        return outcome.loopCompleted && outcome.failedCount == 0
    }

    public func reconcileCloudOutcome(
        timeoutMilliseconds: UInt64
    ) async throws -> PiqaeCloudReconcileOutcome {
        let timeout = min(max(1, timeoutMilliseconds), 10_000)
        guard let library, let handle else {
            throw PiqaeNativeRuntimeError.rejected(
                code: "runtime_not_started",
                message: "The native runtime has not started."
            )
        }
        let requestData = try JSONEncoder().encode(
            TypeOnlyCommand(type: "reconcile_cloud_request")
        )
        let request: ReconcileCloudRequestData = try Self.unwrap(
            library.command(handle, requestData)
        )
        if !request.cloudConfigured {
            guard request.generation == nil else {
                throw PiqaeNativeRuntimeError.invalidResponse
            }
            return .noCloud
        }
        guard let generation = request.generation, generation > 0 else {
            throw PiqaeNativeRuntimeError.invalidResponse
        }
        let deadline = ContinuousClock.now.advanced(by: .milliseconds(Int64(timeout)))
        while ContinuousClock.now < deadline {
            try Task.checkCancellation()
            let pollData = try JSONEncoder().encode(
                ReconcileCloudPollCommand(
                    type: "reconcile_cloud_poll",
                    generation: generation
                )
            )
            let poll: ReconcileCloudPollData = try Self.unwrap(
                library.command(handle, pollData)
            )
            guard poll.cloudConfigured,
                poll.generation == generation,
                poll.pending == (poll.outcome == nil)
            else { throw PiqaeNativeRuntimeError.invalidResponse }
            if let outcome = poll.outcome {
                // Coalescing may complete a later supervisor pass for this
                // request, but an older generation cannot satisfy it.
                guard outcome.generation >= generation else {
                    throw PiqaeNativeRuntimeError.invalidResponse
                }
                return PiqaeCloudReconcileOutcome(
                    generation: outcome.generation,
                    cloudConfigured: poll.cloudConfigured,
                    loopCompleted: outcome.loopCompleted,
                    connectorCount: outcome.connectorCount,
                    succeededCount: outcome.succeededCount,
                    failedCount: outcome.failedCount,
                    allSucceeded: outcome.successScope == .all,
                    partialSuccess: outcome.successScope == .partial,
                    retryable: outcome.retryable,
                    failureClass: outcome.failureClass
                )
            }
            let remaining = ContinuousClock.now.duration(to: deadline)
            try await Task.sleep(for: min(.milliseconds(25), remaining))
        }
        return PiqaeCloudReconcileOutcome(
            generation: generation,
            cloudConfigured: true,
            loopCompleted: false,
            connectorCount: 0,
            succeededCount: 0,
            failedCount: 0,
            allSucceeded: false,
            partialSuccess: false,
            retryable: true,
            failureClass: .transient
        )
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

    public func printPacketCapabilities() async throws -> PiqaePrintPacketCapabilities {
        try commandResponse(
            TypeOnlyCommand(type: "print_packet_capabilities"),
            as: PrintPacketCapabilitiesData.self
        ).capabilities
    }

    public func validatePrintPacket(_ packet: PiqaePrintPacket) async throws
        -> PiqaePrintPacketValidation
    {
        try await ensurePrintPacketSupport(for: packet)
        var payload = try Self.printPacketPayload(packet)
        payload["type"] = "validate_print_packet"
        let data = try JSONSerialization.data(withJSONObject: payload)
        return try commandResponse(data, as: PiqaePrintPacketValidation.self)
    }

    public func enqueuePrintPacket(_ request: PiqaePrintPacketSubmissionRequest) async throws
        -> PiqaePrintPacketSubmission
    {
        try await ensurePrintPacketSupport(for: request.packet)
        var payload = try Self.printPacketPayload(request.packet)
        payload["type"] = "enqueue_print_packet"
        payload["adapter_id"] = request.adapterID
        payload["printer_id"] = request.printerID.rawValue
        payload["idempotency_key"] = request.idempotencyKey
        payload["title"] = request.title
        payload["options_json"] = request.optionsJSON
        payload["expires_unix_ms"] = request.expiresAt.map {
            Int64($0.timeIntervalSince1970 * 1_000)
        } ?? NSNull()
        let data = try JSONSerialization.data(withJSONObject: payload)
        return try commandResponse(data, as: PiqaePrintPacketSubmission.self)
    }

    private func ensurePrintPacketSupport(for packet: PiqaePrintPacket) async throws {
        let capabilities = try await printPacketCapabilities()
        guard capabilities.contract == "printpacket/v1",
            capabilities.rendererABI == "printpacket.pdf-renderer/v1",
            capabilities.resourceABI == "printpacket.resources/v1",
            capabilities.conformanceProfile == "printpacket.conformance/core-v1",
            capabilities.cacheProfile == "printpacket.render-cache/v1",
            capabilities.directOfflineRendering
        else {
            throw PiqaeNativeRuntimeError.nativeCoreUpdateRequired
        }
        guard capabilities.supportedOutputTargets.contains(where: packet.outputTarget.isAdvertised)
        else {
            throw PiqaeNativeRuntimeError.rejected(
                code: "printpacket_unsupported_target",
                message: "The exact PrintPacket output target is not supported by this runtime."
            )
        }
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

    public func jobHistory(offset: Int, limit: Int) async throws -> PiqaeJobHistoryPage {
        guard offset >= 0, offset <= 10_000, (1 ... 200).contains(limit) else {
            throw PiqaeNodeError.invalidConfiguration(
                "History pagination must use an offset from 0 to 10,000 and a limit from 1 to 200."
            )
        }
        let response = try commandResponse(
            JobHistoryCommand(type: "job_history", offset: offset, limit: limit),
            as: JobHistoryData.self
        )
        return PiqaeJobHistoryPage(
            jobs: response.jobs.map {
                PiqaeJobHistoryEntry(
                    jobID: .init(rawValue: $0.jobID),
                    printerID: .init(rawValue: $0.printerID),
                    title: $0.title,
                    state: $0.state,
                    nativeJobID: $0.nativeJobID,
                    canReprint: $0.canReprint,
                    createdAt: $0.createdUnixMilliseconds.map {
                        Date(timeIntervalSince1970: TimeInterval($0) / 1_000)
                    }
                )
            },
            nextOffset: response.nextOffset
        )
    }

    public func updateNodeIdentity(_ request: PiqaeNodeIdentityUpdateRequest) async throws
        -> PiqaeNodeIdentitySnapshot
    {
        try commandResponse(
            UpdateNodeIdentityCommand(request: request),
            as: NodeIdentityUpdatedData.self
        ).snapshot
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
                    nodeName: enrollmentNodeName,
                    hostname: configuration.hostname
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

    private func commandResponse<Response: Decodable>(_ request: Data, as type: Response.Type)
        throws -> Response
    {
        guard let library, let handle else {
            throw PiqaeNativeRuntimeError.rejected(
                code: "runtime_not_started",
                message: "The native runtime has not started."
            )
        }
        return try Self.unwrap(library.command(handle, request))
    }

    private static func printPacketPayload(_ packet: PiqaePrintPacket) throws -> [String: Any] {
        guard
            let specification = try JSONSerialization.jsonObject(with: packet.templateJSON)
                as? [String: Any]
        else {
            throw PiqaeNodeError.invalidConfiguration(
                "The PrintPacket template must be a JSON object."
            )
        }
        let data = try JSONSerialization.jsonObject(with: packet.dataJSON, options: .fragmentsAllowed)
        return [
            "specification": specification,
            "data": data,
            "output_target": packet.outputTarget.jsonObject,
            "resources_base64": packet.resources.mapValues { $0.base64EncodedString() },
        ]
    }

    private static func unwrap<Response: Decodable>(_ data: Data) throws -> Response {
        let envelope: NativeEnvelope<Response>
        do { envelope = try JSONDecoder().decode(NativeEnvelope<Response>.self, from: data) }
        catch { throw PiqaeNativeRuntimeError.invalidResponse }
        if envelope.ok, let value = envelope.data { return value }
        if let error = envelope.error {
            throw mappedRuntimeError(
                code: error.code,
                message: error.message,
                currentRevision: error.details?.currentRevision
            )
        }
        throw PiqaeNativeRuntimeError.invalidResponse
    }

    static func mappedRuntimeError(
        code: String,
        message: String,
        currentRevision: UInt64? = nil
    ) -> PiqaeNativeRuntimeError {
        if code == "printpacket_core_update_required" {
            return .nativeCoreUpdateRequired
        }
        if code == "node_identity_revision_conflict", let currentRevision {
            return .nodeIdentityRevisionConflict(currentRevision: currentRevision)
        }
        return .rejected(code: code, message: message)
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
    let hostConfiguration: PiqaeHostConfiguration?

    enum CodingKeys: String, CodingKey {
        case contract
        case hostMode = "host_mode"
        case availability
        case localOnly = "local_only"
        case applicationID = "application_id"
        case dataDirectory = "data_directory"
        case hostConfiguration = "host_configuration"
    }
}

private struct LifecycleCommand: Encodable {
    let type: String
    let event: PiqaeHostLifecycleEvent
}
private struct ReconcileCloudPollCommand: Encodable {
    let type: String
    let generation: UInt64
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

private struct JobHistoryCommand: Encodable {
    let type: String
    let offset: Int
    let limit: Int
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
private struct UpdateNodeIdentityCommand: Encodable {
    let type = "update_node_identity"
    let expectedRevision: UInt64
    let displayName: String
    let site: String?
    let location: String?
    let labels: [String]

    init(request: PiqaeNodeIdentityUpdateRequest) {
        expectedRevision = request.expectedRevision
        displayName = request.identity.displayName
        site = request.identity.site
        location = request.identity.location
        labels = request.identity.labels
    }

    enum CodingKeys: String, CodingKey {
        case type
        case expectedRevision = "expected_revision"
        case displayName = "display_name"
        case site, location, labels
    }
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
    let details: NativeErrorDetails?
}

private struct NativeErrorDetails: Decodable {
    let currentRevision: UInt64?
    enum CodingKeys: String, CodingKey { case currentRevision = "current_revision" }
}

private struct HandleData: Decodable { let handle: UInt64 }
private struct DestroyData: Decodable { let destroyed: Bool }
private struct NativeSnapshot: Decodable { let handle: UInt64 }
private struct ReconcileCloudRequestData: Decodable {
    let cloudConfigured: Bool
    let generation: UInt64?
    enum CodingKeys: String, CodingKey {
        case cloudConfigured = "cloud_configured"
        case generation
    }
}
private struct NativeReconcileCloudOutcomeData: Decodable {
    enum SuccessScope: String, Decodable { case none, partial, all }
    let generation: UInt64
    let loopCompleted: Bool
    let connectorCount: Int
    let succeededCount: Int
    let failedCount: Int
    let successScope: SuccessScope
    let retryable: Bool
    let failureClass: PiqaeCloudReconcileFailureClass
    enum CodingKeys: String, CodingKey {
        case generation
        case loopCompleted = "loop_completed"
        case connectorCount = "connector_count"
        case succeededCount = "succeeded_count"
        case failedCount = "failed_count"
        case successScope = "success_scope"
        case retryable
        case failureClass = "failure_class"
    }
}
private struct ReconcileCloudPollData: Decodable {
    let cloudConfigured: Bool
    let generation: UInt64
    let pending: Bool
    let outcome: NativeReconcileCloudOutcomeData?
    enum CodingKeys: String, CodingKey {
        case cloudConfigured = "cloud_configured"
        case generation, pending, outcome
    }
}
private struct OpaqueEvidenceData: Decodable {
    let opaqueEvidence: String
    enum CodingKeys: String, CodingKey { case opaqueEvidence = "opaque_evidence" }
}
private struct RegisteredData: Decodable { let registered: Bool }
private struct PrinterInventoryData: Decodable { let printers: [PiqaeRuntimePrinterSnapshot] }
private struct JobAcceptedData: Decodable { let job: PiqaeRuntimeJobAccepted }
private struct PrintPacketCapabilitiesData: Decodable {
    let capabilities: PiqaePrintPacketCapabilities
}
private struct AdapterOperationData: Decodable { let operation: PiqaeRuntimeAdapterOperation? }
private struct AdapterObservationsData: Decodable {
    let operations: [PiqaeRuntimeAdapterOperation]
}
private struct AdapterAcknowledgementData: Decodable {
    let acknowledgement: PiqaeRuntimeAdapterAcknowledgement
}
private struct JobSnapshotData: Decodable { let job: PiqaeRuntimeJobSnapshot }
private struct JobHistoryData: Decodable {
    let jobs: [JobHistoryItem]
    let nextOffset: Int?
    enum CodingKeys: String, CodingKey {
        case jobs
        case nextOffset = "next_offset"
    }
}
private struct JobHistoryItem: Decodable {
    let jobID: String
    let printerID: String
    let title: String
    let state: String
    let nativeJobID: String?
    let canReprint: Bool
    let createdUnixMilliseconds: Int64?
    enum CodingKeys: String, CodingKey {
        case jobID = "job_id"
        case printerID = "printer_id"
        case title, state
        case nativeJobID = "native_job_id"
        case canReprint = "can_reprint"
        case createdUnixMilliseconds = "created_unix_ms"
    }
}
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
private struct NodeIdentityUpdatedData: Decodable {
    let revision: UInt64
    let identity: PiqaeNodeIdentityConfiguration
    var snapshot: PiqaeNodeIdentitySnapshot {
        PiqaeNodeIdentitySnapshot(revision: revision, identity: identity)
    }
}

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
