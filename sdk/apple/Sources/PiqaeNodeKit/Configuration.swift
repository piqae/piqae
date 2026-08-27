import Foundation

public enum PiqaeNodeStartupMode: String, Sendable {
    /// Attach to an installed desktop node first, then use an app-scoped runtime.
    /// iPadOS always uses the embedded application host.
    case automatic
    case attach
    case embedded
}

public struct PiqaeSensitiveString: Sendable, CustomStringConvertible, CustomDebugStringConvertible {
    private let value: String

    public init(_ value: String) throws {
        guard !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw PiqaeNodeError.invalidConfiguration("The invitation must not be empty.")
        }
        self.value = value
    }

    public var description: String { "<redacted>" }
    public var debugDescription: String { "<redacted>" }

    /// Delivers the value only to an enrollment implementation. Avoid retaining it.
    public func withValue<T>(_ body: (String) throws -> T) rethrows -> T {
        try body(value)
    }
}

public struct PiqaeEnrollmentRequest: Sendable {
    public let authorityURL: URL
    public let invitation: PiqaeSensitiveString
    public let installationID: PiqaeInstallationID
    public let hostMode: PiqaeNodeHostMode
    public let availability: PiqaeNodeAvailabilityClass

    public init(
        authorityURL: URL,
        invitation: PiqaeSensitiveString,
        installationID: PiqaeInstallationID,
        hostMode: PiqaeNodeHostMode,
        availability: PiqaeNodeAvailabilityClass
    ) {
        self.authorityURL = authorityURL
        self.invitation = invitation
        self.installationID = installationID
        self.hostMode = hostMode
        self.availability = availability
    }
}

public protocol PiqaeCloudEnrollmentProvider: Sendable {
    /// Legacy preview hook retained for source compatibility. NodeKit no longer
    /// invokes it because only the durable runtime may persist a connector.
    func enroll(_ request: PiqaeEnrollmentRequest) async throws -> PiqaeConnection
}

public struct PiqaeCloudConfiguration: Sendable {
    public let authorityURL: URL
    public let invitation: PiqaeSensitiveString

    public init(
        authorityURL: URL,
        invitation: PiqaeSensitiveString
    ) throws {
        guard Self.isSafeAuthorityURL(authorityURL) else {
            throw PiqaeNodeError.invalidConfiguration(
                "Cloud authorities must use HTTPS. Loopback HTTP is allowed for local development."
            )
        }
        self.authorityURL = authorityURL
        self.invitation = invitation
    }

    @available(*, deprecated, message: "The provider is ignored; use init(authorityURL:invitation:).")
    public init(
        authorityURL: URL,
        invitation: PiqaeSensitiveString,
        provider _: any PiqaeCloudEnrollmentProvider
    ) throws {
        try self.init(authorityURL: authorityURL, invitation: invitation)
    }

    private static func isSafeAuthorityURL(_ url: URL) -> Bool {
        guard
            url.user == nil,
            url.password == nil,
            url.query == nil,
            url.fragment == nil,
            let host = url.host
        else {
            return false
        }
        if url.scheme?.lowercased() == "https" { return true }
        return url.scheme?.lowercased() == "http"
            && ["localhost", "127.0.0.1", "::1"].contains(host.lowercased())
    }
}

public enum PiqaeConnectivityConfiguration: Sendable {
    case localOnly
    case cloud(PiqaeCloudConfiguration)
}

public protocol PiqaeInstallationIdentityStore: Sendable {
    func loadOrCreateInstallationID() async throws -> PiqaeInstallationID
}

public enum PiqaeCloudReconcileFailureClass: String, Codable, Equatable, Sendable {
    case none
    case transient
    case authentication
    case configuration
    case localState = "local_state"
    case `protocol`
    case mixed
    case stopped
}

/// Generation-bound, privacy-safe result of one native supervisor pass.
/// Counts contain no connector, workspace, printer, job or document identity.
public struct PiqaeCloudReconcileOutcome: Codable, Equatable, Sendable {
    public let generation: UInt64
    public let cloudConfigured: Bool
    public let loopCompleted: Bool
    public let connectorCount: Int
    public let succeededCount: Int
    public let failedCount: Int
    public let allSucceeded: Bool
    public let partialSuccess: Bool
    public let retryable: Bool
    public let failureClass: PiqaeCloudReconcileFailureClass

    public init(
        generation: UInt64 = 0,
        cloudConfigured: Bool,
        loopCompleted: Bool,
        connectorCount: Int,
        succeededCount: Int,
        failedCount: Int,
        allSucceeded: Bool,
        partialSuccess: Bool,
        retryable: Bool,
        failureClass: PiqaeCloudReconcileFailureClass
    ) {
        self.generation = generation
        self.cloudConfigured = cloudConfigured
        self.loopCompleted = loopCompleted
        self.connectorCount = connectorCount
        self.succeededCount = succeededCount
        self.failedCount = failedCount
        self.allSucceeded = allSucceeded
        self.partialSuccess = partialSuccess
        self.retryable = retryable
        self.failureClass = failureClass
    }

    public static let noCloud = PiqaeCloudReconcileOutcome(
        cloudConfigured: false,
        loopCompleted: true,
        connectorCount: 0,
        succeededCount: 0,
        failedCount: 0,
        allSucceeded: true,
        partialSuccess: false,
        retryable: false,
        failureClass: .none
    )

    enum CodingKeys: String, CodingKey {
        case generation
        case cloudConfigured = "cloud_configured"
        case loopCompleted = "loop_completed"
        case connectorCount = "connector_count"
        case succeededCount = "succeeded_count"
        case failedCount = "failed_count"
        case allSucceeded = "all_succeeded"
        case partialSuccess = "partial_success"
        case retryable
        case failureClass = "failure_class"
    }
}

/// The shared durable node runtime hosted inside an application process.
/// Platform facades must never implement a second queue beside this runtime.
public protocol PiqaeEmbeddedNodeRuntime: PiqaeHostLifecycleReporter, Sendable {
    /// Installs the data-free wakeup used when durable remote work becomes
    /// available. Implementations invoke it from any thread and retain it only
    /// for the lifetime of the started runtime.
    func setWorkAvailableHandler(_ handler: @escaping @Sendable () -> Void) async throws
    /// Requests one immediate cloud sync and waits only for the bounded native
    /// supervisor pass. This is a nudge, not a lease or remote-wake proof.
    func reconcileCloud(timeoutMilliseconds: UInt64) async throws -> Bool
    /// Generation-bound form used by lifecycle coordinators. The legacy Bool
    /// requirement remains so existing custom runtimes continue to compile.
    func reconcileCloudOutcome(
        timeoutMilliseconds: UInt64
    ) async throws -> PiqaeCloudReconcileOutcome
    func start() async throws
    func stop() async throws
    func registerAdapter(_ registration: PiqaeRuntimeAdapterRegistration) async throws
    func observePrinterInventory(
        adapterID: String,
        printers: [PiqaeRuntimePrinterObservation]
    ) async throws -> [PiqaeRuntimePrinterSnapshot]
    func printerInventory() async throws -> [PiqaeRuntimePrinterSnapshot]
    func enqueue(_ request: PiqaeRuntimeJobRequest) async throws -> PiqaeRuntimeJobAccepted
    func validatePrintPacket(_ packet: PiqaePrintPacket) async throws
        -> PiqaePrintPacketValidation
    func enqueuePrintPacket(_ request: PiqaePrintPacketSubmissionRequest) async throws
        -> PiqaePrintPacketSubmission
    func nextOperation(adapterID: String) async throws -> PiqaeRuntimeAdapterOperation?
    /// Returns accepted native handoffs that require bounded status polling.
    /// This is deliberately separate from runnable queue work so an accepted
    /// handoff cannot suppress a later work-available edge.
    func nativeObservations(adapterID: String) async throws -> [PiqaeRuntimeAdapterOperation]
    func beginHandoff(_ operation: PiqaeRuntimeAdapterOperation) async throws
        -> PiqaeRuntimeAdapterOperation
    func complete(
        _ operation: PiqaeRuntimeAdapterOperation,
        outcome: PiqaeRuntimeAdapterOutcome
    ) async throws -> PiqaeRuntimeAdapterAcknowledgement
    func job(id: PiqaeJobID) async throws -> PiqaeRuntimeJobSnapshot
    func jobHistory(offset: Int, limit: Int) async throws -> PiqaeJobHistoryPage
    func profiles(printerID: PiqaePrinterID) async throws -> [PiqaeRuntimeProfileSnapshot]
    func createProfile(_ request: PiqaeRuntimeProfileCreateRequest) async throws
        -> PiqaeRuntimeProfileSnapshot
    func updateProfile(_ request: PiqaeRuntimeProfileUpdateRequest) async throws
        -> PiqaeRuntimeProfileSnapshot
    func deleteProfile(
        printerID: PiqaePrinterID,
        profileID: PiqaeProfileID,
        expectedRevision: UInt64
    ) async throws
    func connectors() async throws -> [PiqaeRuntimeConnectorSnapshot]
    func updateNodeIdentity(_ request: PiqaeNodeIdentityUpdateRequest) async throws
        -> PiqaeNodeIdentitySnapshot
    func connectInvitation(_ request: PiqaeEnrollmentRequest) async throws
        -> PiqaeRuntimeConnectorSnapshot
    func revokeConnector(id: PiqaeConnectionID) async throws
}

public extension PiqaeEmbeddedNodeRuntime {
    func setWorkAvailableHandler(_ handler: @escaping @Sendable () -> Void) async throws {}
    func reconcileCloud(timeoutMilliseconds: UInt64) async throws -> Bool { false }
    /// Adapts an older Bool-only runtime without inventing connector counts or
    /// retrying an unclassified failure. Native runtimes override this method.
    func reconcileCloudOutcome(
        timeoutMilliseconds: UInt64
    ) async throws -> PiqaeCloudReconcileOutcome {
        let completed = try await reconcileCloud(timeoutMilliseconds: timeoutMilliseconds)
        return PiqaeCloudReconcileOutcome(
            cloudConfigured: true,
            loopCompleted: completed,
            connectorCount: 0,
            succeededCount: 0,
            failedCount: completed ? 0 : 1,
            allSucceeded: completed,
            partialSuccess: false,
            retryable: false,
            failureClass: completed ? .none : .protocol
        )
    }
    func registerAdapter(_ registration: PiqaeRuntimeAdapterRegistration) async throws {
        throw PiqaeNodeError.unsupportedOperation("The embedded runtime does not expose adapters.")
    }
    func observePrinterInventory(
        adapterID: String,
        printers: [PiqaeRuntimePrinterObservation]
    ) async throws -> [PiqaeRuntimePrinterSnapshot] {
        throw PiqaeNodeError.unsupportedOperation("The embedded runtime does not expose inventory.")
    }
    func printerInventory() async throws -> [PiqaeRuntimePrinterSnapshot] { [] }
    func enqueue(_ request: PiqaeRuntimeJobRequest) async throws -> PiqaeRuntimeJobAccepted {
        throw PiqaeNodeError.unsupportedOperation("The embedded runtime does not expose its queue.")
    }
    func validatePrintPacket(_ packet: PiqaePrintPacket) async throws
        -> PiqaePrintPacketValidation
    {
        throw PiqaeNodeError.unsupportedOperation(
            "This runtime does not expose direct PrintPacket rendering."
        )
    }
    func enqueuePrintPacket(_ request: PiqaePrintPacketSubmissionRequest) async throws
        -> PiqaePrintPacketSubmission
    {
        throw PiqaeNodeError.unsupportedOperation(
            "This runtime does not expose direct PrintPacket rendering."
        )
    }
    func nextOperation(adapterID: String) async throws -> PiqaeRuntimeAdapterOperation? { nil }
    func nativeObservations(adapterID: String) async throws -> [PiqaeRuntimeAdapterOperation] { [] }
    func beginHandoff(_ operation: PiqaeRuntimeAdapterOperation) async throws
        -> PiqaeRuntimeAdapterOperation
    {
        throw PiqaeNodeError.unsupportedOperation("The embedded runtime does not expose handoff.")
    }
    func complete(
        _ operation: PiqaeRuntimeAdapterOperation,
        outcome: PiqaeRuntimeAdapterOutcome
    ) async throws -> PiqaeRuntimeAdapterAcknowledgement {
        throw PiqaeNodeError.unsupportedOperation("The embedded runtime does not expose handoff.")
    }
    func job(id: PiqaeJobID) async throws -> PiqaeRuntimeJobSnapshot {
        throw PiqaeNodeError.unsupportedOperation("The embedded runtime does not expose jobs.")
    }
    func jobHistory(offset: Int, limit: Int) async throws -> PiqaeJobHistoryPage {
        throw PiqaeNodeError.unsupportedOperation("The embedded runtime does not expose history.")
    }
    func profiles(printerID: PiqaePrinterID) async throws -> [PiqaeRuntimeProfileSnapshot] { [] }
    func createProfile(_ request: PiqaeRuntimeProfileCreateRequest) async throws
        -> PiqaeRuntimeProfileSnapshot
    {
        throw PiqaeNodeError.unsupportedOperation("The embedded runtime does not expose profiles.")
    }
    func updateProfile(_ request: PiqaeRuntimeProfileUpdateRequest) async throws
        -> PiqaeRuntimeProfileSnapshot
    {
        throw PiqaeNodeError.unsupportedOperation("The embedded runtime does not expose profiles.")
    }
    func deleteProfile(
        printerID: PiqaePrinterID,
        profileID: PiqaeProfileID,
        expectedRevision: UInt64
    ) async throws {
        throw PiqaeNodeError.unsupportedOperation("The embedded runtime does not expose profiles.")
    }
    func connectors() async throws -> [PiqaeRuntimeConnectorSnapshot] { [] }
    func updateNodeIdentity(_ request: PiqaeNodeIdentityUpdateRequest) async throws
        -> PiqaeNodeIdentitySnapshot
    {
        throw PiqaeNodeError.unsupportedOperation(
            "The embedded runtime does not expose node identity editing."
        )
    }
    func connectInvitation(_ request: PiqaeEnrollmentRequest) async throws
        -> PiqaeRuntimeConnectorSnapshot
    {
        throw PiqaeNodeError.unsupportedOperation(
            "The embedded runtime does not expose connector enrollment."
        )
    }
    func revokeConnector(id: PiqaeConnectionID) async throws {
        throw PiqaeNodeError.unsupportedOperation("The embedded runtime does not expose connectors.")
    }
}

public struct PiqaeInstalledNodeProbe: Equatable, Sendable {
    public enum State: Equatable, Sendable {
        case unavailable
        case available(protocolVersion: UInt32)
    }

    public let state: State

    public init(state: State) {
        self.state = state
    }
}

/// Versioned authenticated IPC supplied by a desktop host. Presence probes must
/// not reveal tenant or printer data. Authorization belongs to the host broker.
public protocol PiqaeInstalledNodeIPC: Sendable {
    func probe() async -> PiqaeInstalledNodeProbe
    func prepareForAttachment() async throws
    func snapshot() async throws -> PiqaeNodeSnapshot
    func connect(_ request: PiqaeEnrollmentRequest) async throws -> PiqaeConnection
    func submit(_ request: PiqaePrintRequest) async throws -> PiqaeJobReceipt
    func profiles(for printerID: PiqaePrinterID) async throws -> [PiqaePrintProfile]
    func jobHistory(offset: Int, limit: Int) async throws -> PiqaeJobHistoryPage
    func updateNodeIdentity(_ request: PiqaeNodeIdentityUpdateRequest) async throws
        -> PiqaeNodeIdentitySnapshot
}

public extension PiqaeInstalledNodeIPC {
    func prepareForAttachment() async throws {}
    func connect(_ request: PiqaeEnrollmentRequest) async throws -> PiqaeConnection {
        throw PiqaeNodeError.unsupportedOperation(
            "The installed node broker does not expose connection enrollment."
        )
    }

    func submit(_ request: PiqaePrintRequest) async throws -> PiqaeJobReceipt {
        throw PiqaeNodeError.unsupportedOperation(
            "The installed node broker does not expose app-submitted jobs."
        )
    }

    func profiles(for printerID: PiqaePrinterID) async throws -> [PiqaePrintProfile] {
        []
    }

    func jobHistory(offset: Int, limit: Int) async throws -> PiqaeJobHistoryPage {
        throw PiqaeNodeError.unsupportedOperation(
            "The installed node broker does not expose print history."
        )
    }
    func updateNodeIdentity(_ request: PiqaeNodeIdentityUpdateRequest) async throws
        -> PiqaeNodeIdentitySnapshot
    {
        throw PiqaeNodeError.unsupportedOperation(
            "The installed node broker does not expose node identity editing."
        )
    }
}

public struct PiqaeNodeConfiguration: Sendable {
    public static let supportedLocalProtocolVersions: ClosedRange<UInt32> = 4 ... 4

    public let startupMode: PiqaeNodeStartupMode
    public let connectivity: PiqaeConnectivityConfiguration
    public let availability: PiqaeNodeAvailabilityClass
    public let identityStore: any PiqaeInstallationIdentityStore
    public let installedNodeIPC: (any PiqaeInstalledNodeIPC)?
    public let embeddedRuntime: (any PiqaeEmbeddedNodeRuntime)?
    public let printerAdapters: [any PiqaePrinterAdapter]
    public let hostLifecycleReporter: (any PiqaeHostLifecycleReporter)?
    public let remoteNotificationProvider: (any PiqaeRemoteNotificationRegistrationProvider)?
    public let wakeRetryPolicy: PiqaeWakeRetryPolicy
    /// Desktop automatic mode may create a separate app-scoped node only when
    /// the host explicitly opts into that topology.
    public let allowsEmbeddedFallback: Bool

    public init(
        startupMode: PiqaeNodeStartupMode = .automatic,
        connectivity: PiqaeConnectivityConfiguration = .localOnly,
        availability: PiqaeNodeAvailabilityClass? = nil,
        identityStore: any PiqaeInstallationIdentityStore = PiqaeKeychainInstallationIdentityStore(),
        installedNodeIPC: (any PiqaeInstalledNodeIPC)? = nil,
        embeddedRuntime: (any PiqaeEmbeddedNodeRuntime)? = nil,
        printerAdapters: [any PiqaePrinterAdapter] = [],
        hostLifecycleReporter: (any PiqaeHostLifecycleReporter)? = nil,
        remoteNotificationProvider: (any PiqaeRemoteNotificationRegistrationProvider)? = nil,
        wakeRetryPolicy: PiqaeWakeRetryPolicy = .default,
        allowsEmbeddedFallback: Bool = false
    ) {
        self.startupMode = startupMode
        self.connectivity = connectivity
        self.availability = availability ?? Self.defaultAvailability
        self.identityStore = identityStore
        self.installedNodeIPC = installedNodeIPC
        self.embeddedRuntime = embeddedRuntime
        self.printerAdapters = printerAdapters
        self.hostLifecycleReporter = hostLifecycleReporter
        self.remoteNotificationProvider = remoteNotificationProvider
        self.wakeRetryPolicy = wakeRetryPolicy
        self.allowsEmbeddedFallback = allowsEmbeddedFallback
    }

    public static func localOnly(
        startupMode: PiqaeNodeStartupMode = .automatic,
        availability: PiqaeNodeAvailabilityClass? = nil,
        identityStore: any PiqaeInstallationIdentityStore = PiqaeKeychainInstallationIdentityStore(),
        installedNodeIPC: (any PiqaeInstalledNodeIPC)? = nil,
        embeddedRuntime: (any PiqaeEmbeddedNodeRuntime)? = nil,
        printerAdapters: [any PiqaePrinterAdapter] = [],
        hostLifecycleReporter: (any PiqaeHostLifecycleReporter)? = nil,
        wakeRetryPolicy: PiqaeWakeRetryPolicy = .default,
        allowsEmbeddedFallback: Bool = false
    ) -> PiqaeNodeConfiguration {
        PiqaeNodeConfiguration(
            startupMode: startupMode,
            connectivity: .localOnly,
            availability: availability,
            identityStore: identityStore,
            installedNodeIPC: installedNodeIPC,
            embeddedRuntime: embeddedRuntime,
            printerAdapters: printerAdapters,
            hostLifecycleReporter: hostLifecycleReporter,
            wakeRetryPolicy: wakeRetryPolicy,
            allowsEmbeddedFallback: allowsEmbeddedFallback
        )
    }

    private static var defaultAvailability: PiqaeNodeAvailabilityClass {
        #if os(iOS)
        .foregroundOnly
        #else
        .continuousWhileAwake
        #endif
    }
}
