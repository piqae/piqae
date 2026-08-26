#if os(macOS)
import CPiqaeNodeABI
import CryptoKit
import Foundation
import Security

public enum PiqaeBrokerCapability: String, Codable, CaseIterable, Hashable, Sendable {
    case observeStatus = "observe_status"
    case observePrinters = "observe_printers"
    case observeJobHistory = "observe_job_history"
    case manageProfiles = "manage_profiles"
    case submitLocalJobs = "submit_local_jobs"
    case manageConnectors = "manage_connectors"
}

public struct PiqaeBrokerApplication: Sendable {
    public let applicationID: String
    public let displayName: String
    public let signingIdentitySHA256: String?

    public init(
        applicationID: String,
        displayName: String,
        signingIdentitySHA256: String? = nil
    ) throws {
        let validID = !applicationID.isEmpty && applicationID.utf8.count <= 255
            && applicationID.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || ".-_".contains($0)) }
        let validName = !displayName.isEmpty && displayName.utf8.count <= 128
        let validSigning = signingIdentitySHA256.map {
            $0.utf8.count == 64 && $0.allSatisfy { $0.isHexDigit }
        } ?? true
        guard validID, validName, validSigning else {
            throw PiqaeNodeError.invalidConfiguration("The broker application identity is invalid.")
        }
        self.applicationID = applicationID
        self.displayName = displayName
        self.signingIdentitySHA256 = signingIdentitySHA256?.lowercased()
    }
}

/// Stores only the app-scoped local broker credential. Implementations must
/// keep the bytes out of logs, preferences, iCloud, and shared app containers.
public protocol PiqaeBrokerCredentialStore: Sendable {
    func load(account: String) async throws -> Data?
    func save(_ credential: Data, account: String) async throws
    func remove(account: String) async throws
}

public actor PiqaeKeychainBrokerCredentialStore: PiqaeBrokerCredentialStore {
    private let service: String

    public init(service: String = "com.piqae.nodekit.broker-capability.v1") {
        self.service = service
    }

    public func load(account: String) async throws -> Data? {
        var query = baseQuery(account: account)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data else {
            throw PiqaeKeychainError.operationFailed(status)
        }
        return data
    }

    public func save(_ credential: Data, account: String) async throws {
        var query = baseQuery(account: account)
        query[kSecValueData as String] = credential
        query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let status = SecItemAdd(query as CFDictionary, nil)
        if status == errSecSuccess { return }
        if status == errSecDuplicateItem {
            let update = [kSecValueData as String: credential]
            let updated = SecItemUpdate(baseQuery(account: account) as CFDictionary, update as CFDictionary)
            guard updated == errSecSuccess else { throw PiqaeKeychainError.operationFailed(updated) }
            return
        }
        throw PiqaeKeychainError.operationFailed(status)
    }

    public func remove(account: String) async throws {
        let status = SecItemDelete(baseQuery(account: account) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw PiqaeKeychainError.operationFailed(status)
        }
    }

    private func baseQuery(account: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrSynchronizable as String: kCFBooleanFalse as Any,
        ]
    }
}

protocol PiqaeBrokerWireTransport: Sendable {
    func send(endpoint: String, request: Data) async throws -> Data
    func execute(
        endpoint: String,
        credential: Data,
        capability: Data,
        operation: Data
    ) async throws -> Data
}

private struct PiqaeNativeBrokerWireTransport: PiqaeBrokerWireTransport {
    func send(endpoint: String, request: Data) async throws -> Data {
        guard piqae_node_link_anchor() != 0 else { throw PiqaeNativeRuntimeError.libraryUnavailable }
        return try await Task.detached(priority: .userInitiated) {
            let data = endpoint.data(using: .utf8) ?? Data()
            let buffer = data.withUnsafeBytes { endpointBytes in
                request.withUnsafeBytes { requestBytes in
                    piqae_node_linked_broker_request(
                        endpointBytes.bindMemory(to: UInt8.self).baseAddress,
                        data.count,
                        requestBytes.bindMemory(to: UInt8.self).baseAddress,
                        request.count
                    )
                }
            }
            return try Self.unwrap(buffer)
        }.value
    }

    func execute(
        endpoint: String,
        credential: Data,
        capability: Data,
        operation: Data
    ) async throws -> Data {
        guard piqae_node_link_anchor() != 0 else { throw PiqaeNativeRuntimeError.libraryUnavailable }
        return try await Task.detached(priority: .userInitiated) {
            let endpointData = Data(endpoint.utf8)
            let buffer = endpointData.withUnsafeBytes { endpointBytes in
                credential.withUnsafeBytes { credentialBytes in
                    capability.withUnsafeBytes { capabilityBytes in
                        operation.withUnsafeBytes { operationBytes in
                            piqae_node_linked_broker_execute(
                                endpointBytes.bindMemory(to: UInt8.self).baseAddress,
                                endpointData.count,
                                credentialBytes.bindMemory(to: UInt8.self).baseAddress,
                                credential.count,
                                capabilityBytes.bindMemory(to: UInt8.self).baseAddress,
                                capability.count,
                                operationBytes.bindMemory(to: UInt8.self).baseAddress,
                                operation.count
                            )
                        }
                    }
                }
            }
            let response = try Self.unwrap(buffer)
            guard let object = try JSONSerialization.jsonObject(with: response) as? [String: Any],
                let result = object["result"], JSONSerialization.isValidJSONObject(result)
            else { throw PiqaeNativeRuntimeError.invalidResponse }
            return try JSONSerialization.data(withJSONObject: result)
        }.value
    }

    private static func unwrap(_ buffer: PiqaeBuffer) throws -> Data {
        defer { piqae_node_linked_free(buffer) }
        guard let bytes = buffer.data, (1 ... 2_097_152).contains(buffer.length) else {
            throw PiqaeNativeRuntimeError.invalidResponse
        }
        let envelopeData = Data(bytes: bytes, count: buffer.length)
        let envelope = try JSONSerialization.jsonObject(with: envelopeData) as? [String: Any]
        guard let envelope, envelope["ok"] as? Bool == true,
            let response = envelope["data"], JSONSerialization.isValidJSONObject(response)
        else {
            let failure = envelope?["error"] as? [String: Any]
            throw PiqaeNativeRuntimeError.rejected(
                code: failure?["code"] as? String ?? "broker_transport_failed",
                message: failure?["message"] as? String
                    ?? "The installed node broker request failed."
            )
        }
        return try JSONSerialization.data(withJSONObject: response)
    }
}

/// Authenticated protocol-4 client for the installed per-user macOS node.
/// It never opens the node database and owns no print queue.
public actor PiqaeMacInstalledNodeBroker: PiqaeInstalledNodeIPC {
    public static var defaultEndpoint: URL {
        if let override = ProcessInfo.processInfo.environment["PIQAE_DATA_DIR"], !override.isEmpty {
            return URL(fileURLWithPath: override, isDirectory: true)
                .appendingPathComponent("runtime/node.sock")
        }
        return FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/Spool", isDirectory: true)
            .appendingPathComponent("runtime/node.sock")
    }

    private static let protocolVersion: UInt32 = 4
    private let endpoint: URL
    private let application: PiqaeBrokerApplication
    private let requiredCapabilities: Set<PiqaeBrokerCapability>
    private let credentialStore: any PiqaeBrokerCredentialStore
    private let transport: any PiqaeBrokerWireTransport
    private let pollNanoseconds: UInt64
    private let now: @Sendable () -> Date
    private var credential: BrokerCredential?

    public init(
        endpoint: URL = PiqaeMacInstalledNodeBroker.defaultEndpoint,
        application: PiqaeBrokerApplication,
        requiredCapabilities: Set<PiqaeBrokerCapability> = [
            .observeStatus, .observePrinters, .observeJobHistory, .submitLocalJobs,
            .manageConnectors,
        ],
        credentialStore: any PiqaeBrokerCredentialStore = PiqaeKeychainBrokerCredentialStore()
    ) throws {
        guard endpoint.isFileURL, endpoint.path.hasPrefix("/") else {
            throw PiqaeNodeError.invalidConfiguration("The installed-node broker endpoint must be absolute.")
        }
        guard !requiredCapabilities.isEmpty else {
            throw PiqaeNodeError.invalidConfiguration("At least one broker capability is required.")
        }
        self.endpoint = endpoint
        self.application = application
        self.requiredCapabilities = requiredCapabilities
        self.credentialStore = credentialStore
        transport = PiqaeNativeBrokerWireTransport()
        pollNanoseconds = 500_000_000
        now = { Date() }
    }

    init(
        endpoint: URL,
        application: PiqaeBrokerApplication,
        requiredCapabilities: Set<PiqaeBrokerCapability>,
        credentialStore: any PiqaeBrokerCredentialStore,
        transport: any PiqaeBrokerWireTransport,
        pollNanoseconds: UInt64 = 1,
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.endpoint = endpoint
        self.application = application
        self.requiredCapabilities = requiredCapabilities
        self.credentialStore = credentialStore
        self.transport = transport
        self.pollNanoseconds = pollNanoseconds
        self.now = now
    }

    public func probe() async -> PiqaeInstalledNodeProbe {
        do {
            let result = try await request(operation: ["type": "presence"])
            guard result["type"] as? String == "presence",
                let minimum = (result["protocol_min"] as? NSNumber)?.uint64Value,
                let maximum = (result["protocol_max"] as? NSNumber)?.uint64Value,
                minimum <= UInt64(Self.protocolVersion), maximum >= UInt64(Self.protocolVersion)
            else { return .init(state: .available(protocolVersion: 0)) }
            return .init(state: .available(protocolVersion: Self.protocolVersion))
        } catch {
            return .init(state: .unavailable)
        }
    }

    public func prepareForAttachment() async throws {
        if let stored = try await credentialStore.load(account: credentialAccount),
            let decoded = try? JSONDecoder().decode(BrokerCredential.self, from: stored),
            Set(decoded.grantedCapabilities).isSuperset(of: requiredCapabilities)
        {
            credential = decoded
            do {
                _ = try await execute(capability: .observeStatus, operation: ["type": "status"])
                return
            } catch let error as PiqaeNodeError where error.isBrokerUnauthorized {
                credential = nil
                try await credentialStore.remove(account: credentialAccount)
            }
        }

        let handle = try await requestAuthorization()
        while true {
            guard handle.expiresUnixMS > Int64(now().timeIntervalSince1970 * 1_000) else {
                throw PiqaeNodeError.brokerAuthorizationExpired
            }
            let status = try await authorizationStatus(handle)
            switch status {
            case "approved":
                let issued = try await exchange(handle)
                let granted = Set(issued.grantedCapabilities)
                guard granted.isSuperset(of: requiredCapabilities) else {
                    throw PiqaeNodeError.brokerCapabilityDenied(
                        requiredCapabilities.subtracting(granted).sorted { $0.rawValue < $1.rawValue }
                            .map(\.rawValue).joined(separator: ",")
                    )
                }
                let encoded = try JSONEncoder().encode(issued)
                try await credentialStore.save(encoded, account: credentialAccount)
                credential = issued
                return
            case "denied": throw PiqaeNodeError.brokerAuthorizationDenied
            case "expired": throw PiqaeNodeError.brokerAuthorizationExpired
            case "pending":
                try await Task.sleep(nanoseconds: pollNanoseconds)
            default: throw PiqaeNodeError.invalidBrokerResponse
            }
        }
    }

    /// Removes only this application's device-local broker credential. The
    /// next attachment requests fresh installed-node consent and atomically
    /// rotates the server-side app grant; it does not touch queues or cloud
    /// connector keys.
    public func resetAuthorization() async throws {
        credential = nil
        try await credentialStore.remove(account: credentialAccount)
    }

    public func snapshot() async throws -> PiqaeNodeSnapshot {
        let status = try await execute(capability: .observeStatus, operation: ["type": "status"])
        let printersResult = try await execute(
            capability: .observePrinters, operation: ["type": "printers"]
        )
        let statusData = try JSONSerialization.data(withJSONObject: status)
        let localStatus = try JSONDecoder().decode(BrokerStatus.self, from: statusData)
        let printerData = try JSONSerialization.data(withJSONObject: printersResult["printers"] ?? [])
        let printers = try JSONDecoder().decode([BrokerPrinter].self, from: printerData)
        let observedAt = now()
        return PiqaeNodeSnapshot(
            installationID: localStatus.agentID.map(PiqaeInstallationID.init(rawValue:)),
            hostMode: .userAgent,
            availability: .continuousWhileAwake,
            phase: localStatus.paused ? .suspended : localStatus.phase,
            connections: [localStatus.connectionSnapshot],
            printers: printers.map { $0.snapshot(observedAt: observedAt) },
            lastUpdatedAt: observedAt
        )
    }

    public func submit(_ request: PiqaePrintRequest) async throws -> PiqaeJobReceipt {
        let (kind, content): (String, Data)
        switch request.content {
        case let .pdf(data): (kind, content) = ("pdf", data)
        case let .raw(data, _): (kind, content) = ("raw", data)
        case .image:
            throw PiqaeNodeError.submissionRejected("The installed node accepts PDF or raw content.")
        }
        var options: [String: Any] = ["copies": request.intent.copies]
        if let media = request.intent.media { options["media"] = media }
        let result = try await execute(
            capability: .submitLocalJobs,
            operation: [
                "type": "sdk",
                "operation": [
                    "type": "submit_local_job",
                    "printer_id": request.printerID.rawValue,
                    "title": request.title,
                    "idempotency_key": request.idempotencyKey,
                    "profile_id": request.profileID?.rawValue ?? NSNull(),
                    "content_kind": kind,
                    "content_base64": content.base64EncodedString(),
                    "options": options,
                    "expires_unix_ms": NSNull(),
                ],
            ]
        )
        guard let data = try sdkData(result) as? [String: Any] else {
            throw PiqaeNodeError.invalidBrokerResponse
        }
        guard let jobID = data["job_id"] as? String, let state = data["state"] as? String else {
            throw PiqaeNodeError.invalidBrokerResponse
        }
        return PiqaeJobReceipt(
            jobID: .init(rawValue: jobID), nativeJobID: nil,
            handoffState: Self.handoffState(state), acceptedAt: now()
        )
    }

    public func connect(_ request: PiqaeEnrollmentRequest) async throws -> PiqaeConnection {
        let invitation = try request.invitation.withValue { value in
            guard (1 ... 4_096).contains(value.utf8.count) else {
                throw PiqaeNodeError.invalidConfiguration(
                    "The connector invitation is outside its supported bounds."
                )
            }
            return value
        }
        let hostname = String(ProcessInfo.processInfo.hostName.prefix(256))
        guard !hostname.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw PiqaeNodeError.invalidConfiguration("The computer hostname is unavailable.")
        }
        let result = try await execute(
            capability: .manageConnectors,
            operation: [
                "type": "sdk",
                "operation": [
                    "type": "connect_invitation",
                    "control_plane_url": request.authorityURL.absoluteString,
                    "invitation_token": invitation,
                    "printer_grant": "all_local_printers",
                    "allowed_printer_ids": [],
                    "node_name": String(application.displayName.prefix(256)),
                    "hostname": hostname,
                ],
            ]
        )
        let encoded = try JSONSerialization.data(withJSONObject: try sdkData(result))
        let connected = try JSONDecoder().decode(BrokerConnectedConnector.self, from: encoded)
        guard !connected.connectorID.isEmpty, !connected.agentID.isEmpty else {
            throw PiqaeNodeError.invalidBrokerResponse
        }
        return PiqaeConnection(
            id: .init(rawValue: connected.connectorID),
            authorityURL: request.authorityURL,
            workspaceName: connected.workspaceName,
            state: .connected
        )
    }

    public func profiles(for printerID: PiqaePrinterID) async throws -> [PiqaePrintProfile] {
        let result = try await execute(
            capability: .observePrinters,
            operation: [
                "type": "sdk",
                "operation": ["type": "profiles", "printer_id": printerID.rawValue],
            ]
        )
        let data = try sdkData(result)
        let encoded = try JSONSerialization.data(withJSONObject: data)
        return try JSONDecoder().decode([BrokerProfile].self, from: encoded).map {
            $0.snapshot(printerID: printerID)
        }
    }

    public func jobHistory(offset: Int, limit: Int) async throws -> PiqaeJobHistoryPage {
        guard offset >= 0, (1 ... 200).contains(limit) else {
            throw PiqaeNodeError.invalidConfiguration("History offset/limit are outside their bounds.")
        }
        let result = try await execute(
            capability: .observeJobHistory,
            operation: [
                "type": "sdk",
                "operation": ["type": "job_history", "offset": offset, "limit": limit],
            ]
        )
        let encoded = try JSONSerialization.data(withJSONObject: try sdkData(result))
        let page = try JSONDecoder().decode(BrokerHistoryPage.self, from: encoded)
        return page.snapshot
    }

    private var credentialAccount: String {
        let digest = SHA256.hash(data: Data(endpoint.path.utf8)).map { String(format: "%02x", $0) }.joined()
        return "\(application.applicationID).\(digest)"
    }

    private func requestAuthorization() async throws -> AuthorizationHandle {
        let identity: [String: Any] = [
            "application_id": application.applicationID,
            "display_name": application.displayName,
            "signing_identity_sha256": application.signingIdentitySHA256 ?? NSNull(),
        ]
        let result = try await request(operation: [
            "type": "request_authorization",
            "application": identity,
            "requested_capabilities": requiredCapabilities.sorted { $0.rawValue < $1.rawValue }
                .map(\.rawValue),
        ])
        let encoded = try JSONSerialization.data(withJSONObject: result)
        return try JSONDecoder().decode(AuthorizationHandle.self, from: encoded)
    }

    private func authorizationStatus(_ handle: AuthorizationHandle) async throws -> String {
        let result = try await request(operation: [
            "type": "authorization_status", "handle": handle.dictionary,
        ])
        guard let state = result["state"] as? String else { throw PiqaeNodeError.invalidBrokerResponse }
        return state
    }

    private func exchange(_ handle: AuthorizationHandle) async throws -> BrokerCredential {
        let result = try await request(operation: [
            "type": "exchange_authorization", "handle": handle.dictionary,
        ])
        let encoded = try JSONSerialization.data(withJSONObject: result)
        return try JSONDecoder().decode(BrokerCredential.self, from: encoded)
    }

    private func execute(
        capability: PiqaeBrokerCapability,
        operation: [String: Any]
    ) async throws -> [String: Any] {
        guard let credential else { throw PiqaeNodeError.brokerAuthorizationRequired }
        guard credential.grantedCapabilities.contains(capability) else {
            throw PiqaeNodeError.brokerCapabilityDenied(capability.rawValue)
        }
        let credentialData = try JSONEncoder().encode(credential)
        let capabilityData = try JSONEncoder().encode(capability)
        let operationData = try JSONSerialization.data(withJSONObject: operation)
        let response = try await transport.execute(
            endpoint: endpoint.path,
            credential: credentialData,
            capability: capabilityData,
            operation: operationData
        )
        guard let result = try JSONSerialization.jsonObject(with: response) as? [String: Any]
        else { throw PiqaeNodeError.invalidBrokerResponse }
        return result
    }

    private func request(operation: [String: Any]) async throws -> [String: Any] {
        let requestID = UUID().uuidString.lowercased()
        let encoded = try JSONSerialization.data(withJSONObject: [
            "protocol": Self.protocolVersion, "request_id": requestID, "operation": operation,
        ])
        let responseData = try await transport.send(endpoint: endpoint.path, request: encoded)
        guard let response = try JSONSerialization.jsonObject(with: responseData) as? [String: Any],
            response["request_id"] as? String == requestID,
            (response["protocol"] as? NSNumber)?.uint32Value == Self.protocolVersion,
            let result = response["result"] as? [String: Any]
        else { throw PiqaeNodeError.invalidBrokerResponse }
        if let failure = result["Err"] as? [String: Any] {
            let code = failure["code"] as? String ?? "broker_rejected"
            throw PiqaeNodeError.brokerRejected(code: code)
        }
        guard let success = result["Ok"] as? [String: Any] else {
            throw PiqaeNodeError.invalidBrokerResponse
        }
        return success
    }

    private func sdkData(_ result: [String: Any]) throws -> Any {
        guard result["type"] as? String == "sdk", let data = result["data"] else {
            throw PiqaeNodeError.invalidBrokerResponse
        }
        return data
    }

    private static func handoffState(_ state: String) -> PiqaeNativeHandoffState {
        switch state {
        case "accepted_by_spooler", "spooling", "printing", "completed_reported": .acceptedBySpooler
        case "delivery_uncertain", "ambiguous_handoff": .deliveryUncertain
        default: .queuedLocally
        }
    }
}

private struct BrokerCredential: Codable, Sendable {
    let applicationID: String
    let token: String
    let grantedCapabilities: [PiqaeBrokerCapability]
    enum CodingKeys: String, CodingKey {
        case applicationID = "application_id"
        case token
        case grantedCapabilities = "granted_capabilities"
    }
}

private struct AuthorizationHandle: Codable {
    let authorizationID: UUID
    let nonce: String
    let expiresUnixMS: Int64
    enum CodingKeys: String, CodingKey {
        case authorizationID = "authorization_id"
        case nonce
        case expiresUnixMS = "expires_unix_ms"
    }
    var dictionary: [String: Any] {
        [
            "authorization_id": authorizationID.uuidString.lowercased(),
            "nonce": nonce,
            "expires_unix_ms": expiresUnixMS,
        ]
    }
}

private struct BrokerStatus: Decodable {
    let agentID: String?
    let workspaceName: String?
    let connection: String
    let paused: Bool
    enum CodingKeys: String, CodingKey {
        case agentID = "agent_id"
        case workspaceName = "workspace_name"
        case connection, paused
    }
    var phase: PiqaeNodePhase {
        switch connection { case "connected", "local_only": .ready; case "connecting": .starting; default: .degraded }
    }
    var connectionSnapshot: PiqaeConnection {
        if connection == "local_only" { return .localOnly }
        let state: PiqaeConnectionState = switch connection {
        case "connected": .connected
        case "connecting": .connecting
        case "unauthorized": .needsReauthorization
        default: .offline
        }
        return PiqaeConnection(
            id: .init(rawValue: "installed_node_connection"), authorityURL: nil,
            workspaceName: workspaceName, state: state
        )
    }
}

private struct BrokerQueueCounts: Decodable { let queued: UInt32; let active: UInt32 }
private struct BrokerPrinter: Decodable {
    let printerID: String
    let nativeID: String
    let name: String
    let state: String
    let queueCounts: BrokerQueueCounts
    enum CodingKeys: String, CodingKey {
        case printerID = "printer_id"; case nativeID = "native_id"; case name, state
        case queueCounts = "queue_counts"
    }
    func snapshot(observedAt: Date) -> PiqaePrinter {
        let mapped: PiqaePrinterState = switch state.lowercased() {
        case "idle", "online", "available": .available
        case "printing", "processing", "busy": .busy
        case "paused": .paused
        case "offline", "unavailable": .offline
        default: .unknown
        }
        return PiqaePrinter(
            id: .init(rawValue: printerID), adapterID: "piqae.installed-node", nativeID: nativeID,
            displayName: name, state: mapped,
            queue: .init(
                piqaeOwned: queueCounts.queued &+ queueCounts.active,
                observedAt: observedAt, freshUntil: observedAt.addingTimeInterval(5)
            ),
            observedAt: observedAt, freshUntil: observedAt.addingTimeInterval(5)
        )
    }
}

private struct BrokerProfile: Decodable {
    let profileID: String; let revision: UInt64; let name: String; let isDefault: Bool
    enum CodingKeys: String, CodingKey {
        case profileID = "profile_id"; case revision, name; case isDefault = "is_default"
    }
    func snapshot(printerID: PiqaePrinterID) -> PiqaePrintProfile {
        .init(id: .init(rawValue: profileID), printerID: printerID, name: name,
              revision: revision, isDefault: isDefault)
    }
}

private struct BrokerConnectedConnector: Decodable {
    let connectorID: String
    let agentID: String
    let displayName: String?
    let workspaceName: String?
    let manageURL: String?
    enum CodingKeys: String, CodingKey {
        case connectorID = "connector_id"; case agentID = "agent_id"
        case displayName = "display_name"; case workspaceName = "workspace_name"
        case manageURL = "manage_url"
    }
}

private struct BrokerHistoryPage: Decodable {
    let jobs: [BrokerHistoryJob]
    let nextOffset: Int?
    enum CodingKeys: String, CodingKey { case jobs; case nextOffset = "next_offset" }
    var snapshot: PiqaeJobHistoryPage {
        .init(jobs: jobs.map(\.snapshot), nextOffset: nextOffset)
    }
}

private struct BrokerHistoryJob: Decodable {
    let jobID: String; let printerID: String; let title: String; let state: String
    let nativeJobID: String?; let canReprint: Bool; let createdUnixMS: Int64?
    enum CodingKeys: String, CodingKey {
        case jobID = "job_id"; case printerID = "printer_id"; case title, state
        case nativeJobID = "native_job_id"; case canReprint = "can_reprint"
        case createdUnixMS = "created_unix_ms"
    }
    var snapshot: PiqaeJobHistoryEntry {
        .init(
            jobID: .init(rawValue: jobID), printerID: .init(rawValue: printerID), title: title,
            state: state, nativeJobID: nativeJobID, canReprint: canReprint,
            createdAt: createdUnixMS.map { Date(timeIntervalSince1970: Double($0) / 1_000) }
        )
    }
}

private extension Character {
    var isHexDigit: Bool { isNumber || ("a" ... "f").contains(lowercased()) }
}

private extension PiqaeNodeError {
    var isBrokerUnauthorized: Bool {
        if case let .brokerRejected(code) = self { return code == "application_unauthorized" }
        return false
    }
}
#endif
