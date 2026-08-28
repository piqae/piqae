import Foundation

public struct PiqaeInstallationID: RawRepresentable, Codable, Hashable, Sendable {
    public let rawValue: String

    public init(rawValue: String) {
        self.rawValue = rawValue
    }
}

public struct PiqaePrinterID: RawRepresentable, Codable, Hashable, Sendable {
    public let rawValue: String

    public init(rawValue: String) {
        self.rawValue = rawValue
    }
}

public struct PiqaeConnectionID: RawRepresentable, Codable, Hashable, Sendable {
    public let rawValue: String

    public init(rawValue: String) {
        self.rawValue = rawValue
    }
}

public struct PiqaeJobID: RawRepresentable, Codable, Hashable, Sendable {
    public let rawValue: String

    public init(rawValue: String) {
        self.rawValue = rawValue
    }
}

public struct PiqaeProfileID: RawRepresentable, Codable, Hashable, Sendable {
    public let rawValue: String

    public init(rawValue: String) {
        self.rawValue = rawValue
    }
}

public enum PiqaeNodeHostMode: String, Codable, Sendable, CaseIterable {
    case machineService = "machine_service"
    case userAgent = "user_agent"
    case embeddedApplication = "embedded_application"
    case attachedClient = "attached_client"
}

public enum PiqaeNodeAvailabilityClass: String, Codable, Sendable, CaseIterable {
    case continuousWhileAwake = "continuous_while_awake"
    case foregroundOnly = "foreground_only"
    case backgroundOpportunistic = "background_opportunistic"
    case managedKiosk = "managed_kiosk"
    case wakeRelayCapable = "wake_relay_capable"
}

public enum PiqaeNodePhase: String, Codable, Sendable {
    case stopped
    case starting
    case ready
    case suspended
    case degraded
}

public enum PiqaeConnectionState: String, Codable, Sendable {
    case localOnly = "local_only"
    case connecting
    case connected
    case needsReauthorization = "needs_reauthorization"
    case offline
}

public enum PiqaePrinterState: String, Codable, Sendable {
    case available
    case busy
    case paused
    case offline
    case unknown
}

public struct PiqaeQueueObservation: Codable, Equatable, Sendable {
    public let piqaeOwned: UInt32
    public let external: UInt32
    public let unknown: UInt32
    public let observedAt: Date
    public let freshUntil: Date

    public init(
        piqaeOwned: UInt32 = 0,
        external: UInt32 = 0,
        unknown: UInt32 = 0,
        observedAt: Date,
        freshUntil: Date
    ) {
        self.piqaeOwned = piqaeOwned
        self.external = external
        self.unknown = unknown
        self.observedAt = observedAt
        self.freshUntil = freshUntil
    }
}

public struct PiqaePrinterCapabilities: Codable, Equatable, Sendable {
    public let color: Bool?
    public let duplex: Bool?
    public let cutter: Bool?
    public let portableRevision: UInt64
    public let nativeRevision: String?
    public let supportedMedia: [PiqaeMediaDescriptor]

    public init(
        color: Bool? = nil,
        duplex: Bool? = nil,
        cutter: Bool? = nil,
        portableRevision: UInt64 = 0,
        nativeRevision: String? = nil,
        supportedMedia: [PiqaeMediaDescriptor] = []
    ) {
        self.color = color
        self.duplex = duplex
        self.cutter = cutter
        self.portableRevision = portableRevision
        self.nativeRevision = nativeRevision
        self.supportedMedia = supportedMedia
    }
}

public struct PiqaePrinter: Identifiable, Codable, Equatable, Sendable {
    public let id: PiqaePrinterID
    public let adapterID: String
    public let adapterFingerprint: PiqaeAdapterFingerprint?
    public let nativeID: String
    public let displayName: String
    public let model: String?
    public let location: String?
    public let state: PiqaePrinterState
    public let capabilities: PiqaePrinterCapabilities
    public let queue: PiqaeQueueObservation?
    public let loadedMedia: PiqaeLoadedMediaObservation?
    public let alerts: [PiqaePrinterAlert]
    public let observedAt: Date
    public let freshUntil: Date

    public init(
        id: PiqaePrinterID,
        adapterID: String,
        adapterFingerprint: PiqaeAdapterFingerprint? = nil,
        nativeID: String,
        displayName: String,
        model: String? = nil,
        location: String? = nil,
        state: PiqaePrinterState,
        capabilities: PiqaePrinterCapabilities = .init(),
        queue: PiqaeQueueObservation? = nil,
        loadedMedia: PiqaeLoadedMediaObservation? = nil,
        alerts: [PiqaePrinterAlert] = [],
        observedAt: Date,
        freshUntil: Date
    ) {
        self.id = id
        self.adapterID = adapterID
        self.adapterFingerprint = adapterFingerprint
        self.nativeID = nativeID
        self.displayName = displayName
        self.model = model
        self.location = location
        self.state = state
        self.capabilities = capabilities
        self.queue = queue
        self.loadedMedia = loadedMedia
        self.alerts = alerts
        self.observedAt = observedAt
        self.freshUntil = freshUntil
    }
}

public struct PiqaeConnection: Identifiable, Codable, Equatable, Sendable {
    public let id: PiqaeConnectionID
    public let authorityURL: URL?
    public let workspaceName: String?
    public let state: PiqaeConnectionState
    public let nodeIdentityRevision: UInt64?
    public let nodeIdentityConflictRevision: UInt64?

    public init(
        id: PiqaeConnectionID,
        authorityURL: URL?,
        workspaceName: String?,
        state: PiqaeConnectionState,
        nodeIdentityRevision: UInt64? = nil,
        nodeIdentityConflictRevision: UInt64? = nil
    ) {
        self.id = id
        self.authorityURL = authorityURL
        self.workspaceName = workspaceName
        self.state = state
        self.nodeIdentityRevision = nodeIdentityRevision
        self.nodeIdentityConflictRevision = nodeIdentityConflictRevision
    }

    public static let localOnly = PiqaeConnection(
        id: .init(rawValue: "local_only"),
        authorityURL: nil,
        workspaceName: nil,
        state: .localOnly
    )
}

public struct PiqaePrintProfile: Identifiable, Codable, Equatable, Sendable {
    public let id: PiqaeProfileID
    public let printerID: PiqaePrinterID
    public let name: String
    public let revision: UInt64
    public let isDefault: Bool

    public init(
        id: PiqaeProfileID,
        printerID: PiqaePrinterID,
        name: String,
        revision: UInt64,
        isDefault: Bool = false
    ) {
        self.id = id
        self.printerID = printerID
        self.name = name
        self.revision = revision
        self.isDefault = isDefault
    }
}

public struct PiqaeNodeSnapshot: Codable, Equatable, Sendable {
    public let installationID: PiqaeInstallationID?
    public let hostMode: PiqaeNodeHostMode
    public let availability: PiqaeNodeAvailabilityClass
    public let phase: PiqaeNodePhase
    public let connections: [PiqaeConnection]
    public let printers: [PiqaePrinter]
    public let lastUpdatedAt: Date
    public let statusMessage: String?

    public init(
        installationID: PiqaeInstallationID?,
        hostMode: PiqaeNodeHostMode,
        availability: PiqaeNodeAvailabilityClass,
        phase: PiqaeNodePhase,
        connections: [PiqaeConnection],
        printers: [PiqaePrinter],
        lastUpdatedAt: Date,
        statusMessage: String? = nil
    ) {
        self.installationID = installationID
        self.hostMode = hostMode
        self.availability = availability
        self.phase = phase
        self.connections = connections
        self.printers = printers
        self.lastUpdatedAt = lastUpdatedAt
        self.statusMessage = statusMessage
    }
}

public enum PiqaeNodeError: Error, LocalizedError, Equatable, Sendable {
    case notStarted
    case alreadyStarted
    case unsupportedHostMode
    case installedNodeUnavailable
    case incompatibleInstalledNode(found: UInt32, supported: ClosedRange<UInt32>)
    case nodeAlreadyRunning
    case printerNotFound(PiqaePrinterID)
    case adapterUnavailable(String)
    case unsupportedOperation(String)
    case invalidConfiguration(String)
    case backgroundExecutionUnavailable
    case submissionRejected(String)
    case brokerAuthorizationRequired
    case brokerAuthorizationDenied
    case brokerAuthorizationExpired
    case brokerCapabilityDenied(String)
    case brokerRejected(code: String)
    case invalidBrokerResponse

    public var errorDescription: String? {
        switch self {
        case .notStarted: "The Piqae node has not started."
        case .alreadyStarted: "The Piqae node is already running."
        case .unsupportedHostMode: "This node mode is unavailable on this platform."
        case .installedNodeUnavailable: "No compatible installed Piqae node is available."
        case let .incompatibleInstalledNode(found, supported):
            "The installed node uses local protocol \(found); this SDK supports \(supported.lowerBound)...\(supported.upperBound)."
        case .nodeAlreadyRunning: "Another node runtime owns this installation state."
        case let .printerNotFound(id): "Printer \(id.rawValue) is unavailable."
        case let .adapterUnavailable(id): "Printer adapter \(id) is unavailable."
        case let .unsupportedOperation(message), let .invalidConfiguration(message),
             let .submissionRejected(message): message
        case .backgroundExecutionUnavailable:
            "iPadOS did not grant enough execution time; the work remains queued."
        case .brokerAuthorizationRequired: "The installed node requires local application approval."
        case .brokerAuthorizationDenied: "The node operator denied this application's access."
        case .brokerAuthorizationExpired: "The local application approval request expired."
        case let .brokerCapabilityDenied(capability):
            "The installed node did not approve required capability \(capability)."
        case let .brokerRejected(code): "The installed node rejected the request (\(code))."
        case .invalidBrokerResponse: "The installed node returned an invalid broker response."
        }
    }
}
