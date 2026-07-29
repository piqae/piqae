import Foundation

public enum LocalProfileCaptureOperation: String, Codable, Equatable, Sendable {
    case create
    case edit
    case clone
}

public struct LocalMacNativeConfiguration: Codable, Equatable, Sendable {
    public let kind: String
    public let schemaVersion: UInt32
    public let propertyListPrintSettings: Data
    public let pmPrintSettings: Data
    public let pmPageFormat: Data

    public init(
        kind: String = "macos_printcore",
        schemaVersion: UInt32 = 1,
        propertyListPrintSettings: Data,
        pmPrintSettings: Data,
        pmPageFormat: Data
    ) {
        self.kind = kind
        self.schemaVersion = schemaVersion
        self.propertyListPrintSettings = propertyListPrintSettings
        self.pmPrintSettings = pmPrintSettings
        self.pmPageFormat = pmPageFormat
    }

    enum CodingKeys: String, CodingKey {
        case kind
        case schemaVersion = "schema_version"
        case propertyListPrintSettings = "property_list_print_settings"
        case pmPrintSettings = "pm_print_settings"
        case pmPageFormat = "pm_page_format"
    }
}

public struct LocalNativeProfileSeed: Codable, Equatable, Sendable {
    public let kind: String
    public let schemaVersion: UInt32
    public let digest: String
    public let nativeBlob: Data

    public init(
        kind: String,
        schemaVersion: UInt32,
        digest: String,
        nativeBlob: Data
    ) {
        self.kind = kind
        self.schemaVersion = schemaVersion
        self.digest = digest
        self.nativeBlob = nativeBlob
    }

    enum CodingKeys: String, CodingKey {
        case kind
        case schemaVersion = "schema_version"
        case digest
        case nativeBlob = "native_blob_base64"
    }
}

public struct LocalProfileCaptureSession: Codable, Equatable, Sendable {
    public let sessionID: String
    public let captureToken: String
    public let expiresUnixMS: Int64
    public let operation: LocalProfileCaptureOperation
    public let printerID: String
    public let nativeID: String?
    public let printerName: String
    public let profileID: String?
    public let profileName: String?
    public let stockID: String?
    public let safeOverrides: [String]?
    public let expectedRevision: UInt64?
    public let nativeConfiguration: LocalNativeProfileSeed?

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case captureToken = "capture_token"
        case expiresUnixMS = "expires_unix_ms"
        case operation
        case printerID = "printer_id"
        case nativeID = "native_id"
        case printerName = "printer_name"
        case profileID = "profile_id"
        case profileName = "profile_name"
        case stockID = "stock_id"
        case safeOverrides = "safe_overrides"
        case expectedRevision = "expected_revision"
        case nativeConfiguration = "native_configuration"
    }
}

public struct LocalMacDriverFingerprint: Codable, Equatable, Sendable {
    public let platform: String
    public let driverName: String
    public let driverVersion: String?
    public let architecture: String?
    public let nativeQueueID: String
    public let deviceFingerprint: String?
    public let driverPackageFingerprint: String?

    public init(
        platform: String = "macos",
        driverName: String,
        driverVersion: String? = nil,
        architecture: String?,
        nativeQueueID: String,
        deviceFingerprint: String?,
        driverPackageFingerprint: String? = nil
    ) {
        self.platform = platform
        self.driverName = driverName
        self.driverVersion = driverVersion
        self.architecture = architecture
        self.nativeQueueID = nativeQueueID
        self.deviceFingerprint = deviceFingerprint
        self.driverPackageFingerprint = driverPackageFingerprint
    }

    enum CodingKeys: String, CodingKey {
        case platform
        case driverName = "driver_name"
        case driverVersion = "driver_version"
        case architecture
        case nativeQueueID = "native_queue_id"
        case deviceFingerprint = "device_fingerprint"
        case driverPackageFingerprint = "driver_package_fingerprint"
    }
}

public struct LocalProfileSummary: Codable, Equatable, Sendable {
    public let paper: String?
    public let dimensionsMM: [Double]?
    public let source: String?
    public let media: String?
    public let color: String?
    public let duplex: String?
    public let resolution: String?
    public let copies: UInt32?
    public let native: [String: String]
    public let details: [String: String]

    public init(
        paper: String?,
        dimensionsMM: [Double]?,
        source: String? = nil,
        media: String? = nil,
        color: String? = nil,
        duplex: String? = nil,
        resolution: String? = nil,
        copies: UInt32? = nil,
        native: [String: String] = [:],
        details: [String: String] = [:]
    ) {
        self.paper = paper
        self.dimensionsMM = dimensionsMM
        self.source = source
        self.media = media
        self.color = color
        self.duplex = duplex
        self.resolution = resolution
        self.copies = copies
        self.native = native
        self.details = details
    }

    enum CodingKeys: String, CodingKey {
        case paper
        case dimensionsMM = "dimensions_mm"
        case source
        case media
        case color
        case duplex
        case resolution
        case copies
        case native
        case details
    }
}

public struct LocalProfileDependency: Codable, Equatable, Sendable {
    public let kind: String
    public let value: String

    public init(kind: String, value: String) {
        self.kind = kind
        self.value = value
    }
}

public struct LocalProfileCaptureCompletion: Codable, Equatable, Sendable {
    public let name: String
    public let isDefault: Bool
    public let options: [String: String]
    public let nativeKind: String
    public let nativeSchemaVersion: UInt32
    public let nativeDigest: String
    public let nativeBlob: Data
    public let driverFingerprint: LocalMacDriverFingerprint
    public let summary: LocalProfileSummary
    public let stockID: String?
    public let dependencies: [LocalProfileDependency]
    public let safeOverrides: [String]
    public let published: Bool

    public init(
        name: String,
        isDefault: Bool = false,
        options: [String: String] = [:],
        nativeKind: String = "macos_printcore",
        nativeSchemaVersion: UInt32 = 1,
        nativeDigest: String,
        nativeBlob: Data,
        driverFingerprint: LocalMacDriverFingerprint,
        summary: LocalProfileSummary,
        stockID: String?,
        dependencies: [LocalProfileDependency] = [],
        safeOverrides: [String],
        published: Bool = false
    ) {
        self.name = name
        self.isDefault = isDefault
        self.options = options
        self.nativeKind = nativeKind
        self.nativeSchemaVersion = nativeSchemaVersion
        self.nativeDigest = nativeDigest
        self.nativeBlob = nativeBlob
        self.driverFingerprint = driverFingerprint
        self.summary = summary
        self.stockID = stockID
        self.dependencies = dependencies
        self.safeOverrides = safeOverrides
        self.published = published
    }

    enum CodingKeys: String, CodingKey {
        case name
        case isDefault = "is_default"
        case options
        case nativeKind = "native_kind"
        case nativeSchemaVersion = "native_schema_version"
        case nativeDigest = "native_digest"
        case nativeBlob = "native_blob_base64"
        case driverFingerprint = "driver_fingerprint"
        case summary
        case stockID = "stock_id"
        case dependencies
        case safeOverrides = "safe_overrides"
        case published
    }
}

public enum ProfileMenuState: Equatable, Sendable {
    case unavailable
    case empty
    case available(profileCount: Int)

    public init(profiles: [LocalPrintProfile]?, agentAvailable: Bool) {
        guard agentAvailable, let profiles else {
            self = .unavailable
            return
        }
        self = profiles.isEmpty ? .empty : .available(profileCount: profiles.count)
    }
}
