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

/// The shared durable node runtime hosted inside an application process.
/// Platform facades must never implement a second queue beside this runtime.
public protocol PiqaeEmbeddedNodeRuntime: PiqaeHostLifecycleReporter, Sendable {
    /// Installs the data-free wakeup used when durable remote work becomes
    /// available. Implementations invoke it from any thread and retain it only
    /// for the lifetime of the started runtime.
    func setWorkAvailableHandler(_ handler: @escaping @Sendable () -> Void) async throws
    func start() async throws
    func stop() async throws
    func registerAdapter(_ registration: PiqaeRuntimeAdapterRegistration) async throws
    func observePrinterInventory(
        adapterID: String,
        printers: [PiqaeRuntimePrinterObservation]
    ) async throws -> [PiqaeRuntimePrinterSnapshot]
    func printerInventory() async throws -> [PiqaeRuntimePrinterSnapshot]
    func enqueue(_ request: PiqaeRuntimeJobRequest) async throws -> PiqaeRuntimeJobAccepted
    func nextOperation(adapterID: String) async throws -> PiqaeRuntimeAdapterOperation?
    func beginHandoff(_ operation: PiqaeRuntimeAdapterOperation) async throws
        -> PiqaeRuntimeAdapterOperation
    func complete(
        _ operation: PiqaeRuntimeAdapterOperation,
        outcome: PiqaeRuntimeAdapterOutcome
    ) async throws -> PiqaeRuntimeAdapterAcknowledgement
    func job(id: PiqaeJobID) async throws -> PiqaeRuntimeJobSnapshot
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
    func connectInvitation(_ request: PiqaeEnrollmentRequest) async throws
        -> PiqaeRuntimeConnectorSnapshot
    func revokeConnector(id: PiqaeConnectionID) async throws
}

public extension PiqaeEmbeddedNodeRuntime {
    func setWorkAvailableHandler(_ handler: @escaping @Sendable () -> Void) async throws {}
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
    func nextOperation(adapterID: String) async throws -> PiqaeRuntimeAdapterOperation? { nil }
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
