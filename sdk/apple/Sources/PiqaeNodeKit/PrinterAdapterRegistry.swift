import Foundation

public enum PiqaePrinterTransport: String, Codable, Sendable, CaseIterable {
    case installedDriver = "installed_driver"
    case airPrint = "airprint"
    case ipp
    case bluetoothLE = "bluetooth_le"
    case externalAccessory = "external_accessory"
    case networkRaw = "network_raw"
    case vendorSDK = "vendor_sdk"
}

/// Produces non-reversible, installation-scoped evidence through the shared
/// runtime. Implementations must fail closed when the host key is unavailable.
public protocol PiqaeOpaqueIdentityProvider: Sendable {
    func deriveOpaqueID(
        namespace: String,
        canonicalIdentity: Data
    ) async throws -> String
}

/// Matches the display-safe selector used by Piqae support packs. Serial
/// numbers, Bluetooth addresses, credentials, and native option payloads do
/// not belong in this fingerprint.
public struct PiqaeAdapterFingerprint: Codable, Equatable, Sendable {
    public enum Platform: String, Codable, Sendable {
        case iosAirPrint = "ios_air_print"
        case iosNetwork = "ios_network"
        case iosBluetoothLE = "ios_bluetooth_le"
        case iosExternalAccessory = "ios_external_accessory"
    }

    public let platform: Platform
    public let adapterID: String
    public let adapterVersion: String
    public let deviceFamily: String?
    public let firmwareVersion: String?

    public init(
        platform: Platform,
        adapterID: String,
        adapterVersion: String,
        deviceFamily: String? = nil,
        firmwareVersion: String? = nil
    ) {
        self.platform = platform
        self.adapterID = adapterID
        self.adapterVersion = adapterVersion
        self.deviceFamily = deviceFamily
        self.firmwareVersion = firmwareVersion
    }
}

public enum PiqaeAdapterBackgroundWake: String, Codable, Sendable, CaseIterable {
    case none
    case bluetoothEvent = "bluetooth_event"
    case externalAccessoryEvent = "external_accessory_event"
}

public enum PiqaePortableOption: String, Codable, Sendable, CaseIterable {
    case copies
    case media
    case orientation
    case cut
    case density
}

public struct PiqaePrinterAdapterDescriptor: Codable, Equatable, Sendable, Identifiable {
    public let id: String
    public let displayName: String
    public let version: String
    public let transports: [PiqaePrinterTransport]
    public let portableOptions: [PiqaePortableOption]
    public let supportsProfiles: Bool
    public let backgroundWake: PiqaeAdapterBackgroundWake

    public init(
        id: String,
        displayName: String,
        version: String,
        transports: [PiqaePrinterTransport],
        portableOptions: [PiqaePortableOption],
        supportsProfiles: Bool,
        backgroundWake: PiqaeAdapterBackgroundWake = .none
    ) {
        self.id = id
        self.displayName = displayName
        self.version = version
        self.transports = transports
        self.portableOptions = portableOptions
        self.supportsProfiles = supportsProfiles
        self.backgroundWake = backgroundWake
    }
}

public struct PiqaeMediaDescriptor: Codable, Equatable, Sendable, Identifiable {
    public enum Kind: String, Codable, Sendable {
        case sheet
        case roll
        case label
        case receipt
        case unknown
    }

    public let id: String
    public let displayName: String
    public let kind: Kind
    public let widthMicrometres: UInt32?
    public let heightMicrometres: UInt32?

    public init(
        id: String,
        displayName: String,
        kind: Kind,
        widthMicrometres: UInt32? = nil,
        heightMicrometres: UInt32? = nil
    ) {
        self.id = id
        self.displayName = displayName
        self.kind = kind
        self.widthMicrometres = widthMicrometres
        self.heightMicrometres = heightMicrometres
    }
}

public struct PiqaeLoadedMediaObservation: Codable, Equatable, Sendable {
    public enum Source: String, Codable, Sendable {
        case deviceSensor = "device_sensor"
        case operatorConfirmed = "operator_confirmed"
        case driverDefault = "driver_default"
        case unknown
    }

    public let media: PiqaeMediaDescriptor
    public let source: Source
    public let confidence: Double
    public let observedAt: Date
    public let freshUntil: Date

    public init(
        media: PiqaeMediaDescriptor,
        source: Source,
        confidence: Double,
        observedAt: Date,
        freshUntil: Date
    ) {
        self.media = media
        self.source = source
        self.confidence = min(max(confidence, 0), 1)
        self.observedAt = observedAt
        self.freshUntil = freshUntil
    }
}

public struct PiqaePrinterAlert: Codable, Equatable, Sendable, Identifiable {
    public enum Severity: String, Codable, Sendable {
        case information
        case warning
        case blocking
    }

    public let id: String
    public let severity: Severity
    public let code: String
    public let message: String

    public init(id: String, severity: Severity, code: String, message: String) {
        self.id = id
        self.severity = severity
        self.code = code
        self.message = message
    }
}
