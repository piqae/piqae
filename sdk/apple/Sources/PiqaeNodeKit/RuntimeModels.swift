import Foundation

public struct PiqaeRuntimeAdapterRegistration: Codable, Equatable, Sendable {
    public let fingerprint: PiqaeAdapterFingerprint
    public let capabilityContract: PiqaeRuntimeCapabilityContract

    public init(
        fingerprint: PiqaeAdapterFingerprint,
        capabilityContract: PiqaeRuntimeCapabilityContract
    ) {
        self.fingerprint = fingerprint
        self.capabilityContract = capabilityContract
    }

    enum CodingKeys: String, CodingKey {
        case fingerprint
        case capabilityContract = "capability_contract"
    }
}

public struct PiqaeRuntimeCapabilityContract: Codable, Equatable, Sendable {
    public let documentKinds: [String]
    public let transports: [PiqaePrinterTransport]
    public let portableOptions: [PiqaePortableOption]
    public let supportsProfiles: Bool

    public init(descriptor: PiqaePrinterAdapterDescriptor) {
        documentKinds = descriptor.documentKinds
        transports = descriptor.transports
        portableOptions = descriptor.portableOptions
        supportsProfiles = descriptor.supportsProfiles
    }

    enum CodingKeys: String, CodingKey {
        case documentKinds = "document_kinds"
        case transports
        case portableOptions = "portable_options"
        case supportsProfiles = "supports_profiles"
    }
}

public struct PiqaeRuntimePrinterObservation: Codable, Equatable, Sendable {
    public let nativeID: String
    public let name: String
    public let state: String
    public let isDefault: Bool
    public let nativeOptions: [String: PiqaeRuntimeNativePrinterOption]

    public init(
        nativeID: String,
        name: String,
        state: String,
        isDefault: Bool = false,
        nativeOptions: [String: PiqaeRuntimeNativePrinterOption] = [:]
    ) {
        self.nativeID = nativeID
        self.name = name
        self.state = state
        self.isDefault = isDefault
        self.nativeOptions = nativeOptions
    }

    enum CodingKeys: String, CodingKey {
        case nativeID = "native_id"
        case name, state
        case isDefault = "is_default"
        case nativeOptions = "native_options"
    }
}

public struct PiqaeRuntimeNativePrinterOption: Codable, Equatable, Sendable {
    public struct Choice: Codable, Equatable, Sendable {
        public let value: String
        public let displayName: String

        public init(value: String, displayName: String) {
            self.value = value
            self.displayName = displayName
        }

        enum CodingKeys: String, CodingKey {
            case value
            case displayName = "display_name"
        }
    }

    public let displayName: String
    public let defaultChoice: String?
    public let selectedChoice: String?
    public let choices: [Choice]

    public init(
        displayName: String,
        defaultChoice: String?,
        selectedChoice: String?,
        choices: [Choice]
    ) {
        self.displayName = displayName
        self.defaultChoice = defaultChoice
        self.selectedChoice = selectedChoice
        self.choices = choices
    }

    enum CodingKeys: String, CodingKey {
        case displayName = "display_name"
        case defaultChoice = "default_choice"
        case selectedChoice = "selected_choice"
        case choices
    }
}

public struct PiqaeRuntimePrinterSnapshot: Codable, Equatable, Sendable {
    public let printerID: String
    public let adapterID: String
    public let nativeID: String
    public let name: String
    public let state: String
    public let isDefault: Bool
    public let observedUnixMilliseconds: Int64

    public init(
        printerID: String,
        adapterID: String,
        nativeID: String,
        name: String,
        state: String,
        isDefault: Bool = false,
        observedUnixMilliseconds: Int64
    ) {
        self.printerID = printerID
        self.adapterID = adapterID
        self.nativeID = nativeID
        self.name = name
        self.state = state
        self.isDefault = isDefault
        self.observedUnixMilliseconds = observedUnixMilliseconds
    }

    enum CodingKeys: String, CodingKey {
        case printerID = "printer_id"
        case adapterID = "adapter_id"
        case nativeID = "native_id"
        case name, state
        case isDefault = "is_default"
        case observedUnixMilliseconds = "observed_unix_ms"
    }
}

public struct PiqaeRuntimeJobRequest: Sendable {
    public let adapterID: String
    public let idempotencyKey: String
    public let printerID: PiqaePrinterID
    public let title: String
    public let contentKind: String
    public let content: Data
    public let optionsJSON: String
    public let expiresUnixMilliseconds: Int64?

    public init(
        adapterID: String,
        idempotencyKey: String,
        printerID: PiqaePrinterID,
        title: String,
        contentKind: String,
        content: Data,
        optionsJSON: String,
        expiresUnixMilliseconds: Int64? = nil
    ) {
        self.adapterID = adapterID
        self.idempotencyKey = idempotencyKey
        self.printerID = printerID
        self.title = title
        self.contentKind = contentKind
        self.content = content
        self.optionsJSON = optionsJSON
        self.expiresUnixMilliseconds = expiresUnixMilliseconds
    }
}

public struct PiqaeRuntimeJobAccepted: Codable, Equatable, Sendable {
    public let jobID: String
    public let state: String
    enum CodingKeys: String, CodingKey { case jobID = "job_id"; case state }
}

public enum PiqaeRuntimeAdapterOperationPhase: String, Codable, Sendable {
    case claimed
    case handoffStarted = "handoff_started"
    case accepted
}

public struct PiqaeRuntimeAdapterOperation: Codable, Equatable, Sendable {
    public let operationID: String
    public let adapterID: String
    public let jobID: String
    public let idempotencyKey: String
    public let fence: String
    public let deadlineUnixMilliseconds: Int64
    public let printerID: String
    public let printerNativeID: String
    public let title: String
    public let contentPath: String
    public let contentKind: String
    public let contentSHA256: String
    public let optionsJSON: String
    public let phase: PiqaeRuntimeAdapterOperationPhase
    public let nativeJobID: String?

    public init(
        operationID: String,
        adapterID: String,
        jobID: String,
        idempotencyKey: String,
        fence: String,
        deadlineUnixMilliseconds: Int64,
        printerID: String,
        printerNativeID: String,
        title: String,
        contentPath: String,
        contentKind: String,
        contentSHA256: String,
        optionsJSON: String,
        phase: PiqaeRuntimeAdapterOperationPhase,
        nativeJobID: String? = nil
    ) {
        self.operationID = operationID
        self.adapterID = adapterID
        self.jobID = jobID
        self.idempotencyKey = idempotencyKey
        self.fence = fence
        self.deadlineUnixMilliseconds = deadlineUnixMilliseconds
        self.printerID = printerID
        self.printerNativeID = printerNativeID
        self.title = title
        self.contentPath = contentPath
        self.contentKind = contentKind
        self.contentSHA256 = contentSHA256
        self.optionsJSON = optionsJSON
        self.phase = phase
        self.nativeJobID = nativeJobID
    }

    enum CodingKeys: String, CodingKey {
        case operationID = "operation_id"
        case adapterID = "adapter_id"
        case jobID = "job_id"
        case idempotencyKey = "idempotency_key"
        case fence
        case deadlineUnixMilliseconds = "deadline_unix_ms"
        case printerID = "printer_id"
        case printerNativeID = "printer_native_id"
        case title
        case contentPath = "content_path"
        case contentKind = "content_kind"
        case contentSHA256 = "content_sha256"
        case optionsJSON = "options_json"
        case phase
        case nativeJobID = "native_job_id"
    }
}

public enum PiqaeRuntimeAdapterOutcome: Encodable, Equatable, Sendable {
    case rejectedBeforeHandoff(code: String, retryable: Bool)
    case accepted(nativeJobID: String)
    case completedReported(nativeJobID: String)
    case failedTerminal(nativeJobID: String, code: String)
    case ambiguous(code: String)

    enum CodingKeys: String, CodingKey {
        case outcome, code, retryable
        case nativeJobID = "native_job_id"
    }

    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case let .rejectedBeforeHandoff(code, retryable):
            try values.encode("rejected_before_handoff", forKey: .outcome)
            try values.encode(code, forKey: .code)
            try values.encode(retryable, forKey: .retryable)
        case let .accepted(nativeJobID):
            try values.encode("accepted", forKey: .outcome)
            try values.encode(nativeJobID, forKey: .nativeJobID)
        case let .completedReported(nativeJobID):
            try values.encode("completed_reported", forKey: .outcome)
            try values.encode(nativeJobID, forKey: .nativeJobID)
        case let .failedTerminal(nativeJobID, code):
            try values.encode("failed_terminal", forKey: .outcome)
            try values.encode(nativeJobID, forKey: .nativeJobID)
            try values.encode(code, forKey: .code)
        case let .ambiguous(code):
            try values.encode("ambiguous", forKey: .outcome)
            try values.encode(code, forKey: .code)
        }
    }
}

public struct PiqaeRuntimeAdapterAcknowledgement: Codable, Equatable, Sendable {
    public let operationID: String
    public let jobID: String
    public let state: String
    public let duplicate: Bool
    public init(operationID: String, jobID: String, state: String, duplicate: Bool = false) {
        self.operationID = operationID
        self.jobID = jobID
        self.state = state
        self.duplicate = duplicate
    }
    enum CodingKeys: String, CodingKey {
        case operationID = "operation_id"
        case jobID = "job_id"
        case state, duplicate
    }
}

public struct PiqaeRuntimeJobSnapshot: Codable, Equatable, Sendable {
    public let jobID: String
    public let state: String
    public let nativeJobID: String?
    enum CodingKeys: String, CodingKey {
        case jobID = "job_id"
        case state
        case nativeJobID = "native_job_id"
    }
}

public struct PiqaeRuntimeProfileSnapshot: Codable, Equatable, Sendable {
    public let profileID: String
    public let printerID: String
    public let revision: UInt64
    public let name: String
    public let isDefault: Bool
    public let optionsJSON: String
    enum CodingKeys: String, CodingKey {
        case profileID = "profile_id"
        case printerID = "printer_id"
        case revision, name
        case isDefault = "is_default"
        case optionsJSON = "options_json"
    }
}

public struct PiqaeRuntimeProfileCreateRequest: Sendable {
    public let printerID: PiqaePrinterID
    public let name: String
    public let isDefault: Bool
    public let optionsJSON: String

    public init(
        printerID: PiqaePrinterID,
        name: String,
        isDefault: Bool = false,
        optionsJSON: String = "{}"
    ) {
        self.printerID = printerID
        self.name = name
        self.isDefault = isDefault
        self.optionsJSON = optionsJSON
    }
}

public struct PiqaeRuntimeProfileUpdateRequest: Sendable {
    public let printerID: PiqaePrinterID
    public let profileID: PiqaeProfileID
    public let expectedRevision: UInt64
    public let name: String
    public let isDefault: Bool
    public let optionsJSON: String

    public init(
        printerID: PiqaePrinterID,
        profileID: PiqaeProfileID,
        expectedRevision: UInt64,
        name: String,
        isDefault: Bool,
        optionsJSON: String = "{}"
    ) {
        self.printerID = printerID
        self.profileID = profileID
        self.expectedRevision = expectedRevision
        self.name = name
        self.isDefault = isDefault
        self.optionsJSON = optionsJSON
    }
}

public struct PiqaeRuntimeConnectorSnapshot: Codable, Equatable, Sendable {
    public let connectorID: String
    public let controlPlaneURL: URL
    public let displayName: String?
    public let workspaceName: String?
    public let enabled: Bool

    public init(
        connectorID: String,
        controlPlaneURL: URL,
        displayName: String? = nil,
        workspaceName: String? = nil,
        enabled: Bool
    ) {
        self.connectorID = connectorID
        self.controlPlaneURL = controlPlaneURL
        self.displayName = displayName
        self.workspaceName = workspaceName
        self.enabled = enabled
    }

    enum CodingKeys: String, CodingKey {
        case connectorID = "connector_id"
        case controlPlaneURL = "control_plane_url"
        case displayName = "display_name"
        case workspaceName = "workspace_name"
        case enabled
    }
}
