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
    /// Exchanges a short-lived invitation. Platform service-account credentials
    /// belong in the provider's backend and must never be embedded in an app.
    func enroll(_ request: PiqaeEnrollmentRequest) async throws -> PiqaeConnection
}

public struct PiqaeCloudConfiguration: Sendable {
    public let authorityURL: URL
    public let invitation: PiqaeSensitiveString
    public let provider: any PiqaeCloudEnrollmentProvider

    public init(
        authorityURL: URL,
        invitation: PiqaeSensitiveString,
        provider: any PiqaeCloudEnrollmentProvider
    ) throws {
        guard Self.isSafeAuthorityURL(authorityURL) else {
            throw PiqaeNodeError.invalidConfiguration(
                "Cloud authorities must use HTTPS. Loopback HTTP is allowed for local development."
            )
        }
        self.authorityURL = authorityURL
        self.invitation = invitation
        self.provider = provider
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
    func snapshot() async throws -> PiqaeNodeSnapshot
    func connect(_ request: PiqaeEnrollmentRequest) async throws -> PiqaeConnection
    func submit(_ request: PiqaePrintRequest) async throws -> PiqaeJobReceipt
    func profiles(for printerID: PiqaePrinterID) async throws -> [PiqaePrintProfile]
}

public extension PiqaeInstalledNodeIPC {
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
}

public struct PiqaeNodeConfiguration: Sendable {
    public static let supportedLocalProtocolVersions: ClosedRange<UInt32> = 1 ... 1

    public let startupMode: PiqaeNodeStartupMode
    public let connectivity: PiqaeConnectivityConfiguration
    public let availability: PiqaeNodeAvailabilityClass
    public let identityStore: any PiqaeInstallationIdentityStore
    public let installedNodeIPC: (any PiqaeInstalledNodeIPC)?
    public let printerAdapters: [any PiqaePrinterAdapter]

    public init(
        startupMode: PiqaeNodeStartupMode = .automatic,
        connectivity: PiqaeConnectivityConfiguration = .localOnly,
        availability: PiqaeNodeAvailabilityClass? = nil,
        identityStore: any PiqaeInstallationIdentityStore = PiqaeKeychainInstallationIdentityStore(),
        installedNodeIPC: (any PiqaeInstalledNodeIPC)? = nil,
        printerAdapters: [any PiqaePrinterAdapter] = []
    ) {
        self.startupMode = startupMode
        self.connectivity = connectivity
        self.availability = availability ?? Self.defaultAvailability
        self.identityStore = identityStore
        self.installedNodeIPC = installedNodeIPC
        self.printerAdapters = printerAdapters
    }

    public static func localOnly(
        startupMode: PiqaeNodeStartupMode = .automatic,
        availability: PiqaeNodeAvailabilityClass? = nil,
        identityStore: any PiqaeInstallationIdentityStore = PiqaeKeychainInstallationIdentityStore(),
        installedNodeIPC: (any PiqaeInstalledNodeIPC)? = nil,
        printerAdapters: [any PiqaePrinterAdapter] = []
    ) -> PiqaeNodeConfiguration {
        PiqaeNodeConfiguration(
            startupMode: startupMode,
            connectivity: .localOnly,
            availability: availability,
            identityStore: identityStore,
            installedNodeIPC: installedNodeIPC,
            printerAdapters: printerAdapters
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
