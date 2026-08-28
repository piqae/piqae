import Foundation

/// Explicit renderer target for vendor-neutral PrintPacket documents.
public enum PiqaePrintPacketOutputTarget: Equatable, Sendable {
    case pdf(profile: String = "printpacket.pdf-base14/v1")
    case printerNative(
        language: String,
        profile: String,
        dpi: UInt16,
        printableWidthDots: UInt32
    )

    var jsonObject: [String: Any] {
        switch self {
        case let .pdf(profile):
            ["kind": "pdf", "profile": profile]
        case let .printerNative(language, profile, dpi, printableWidthDots):
            [
                "kind": "printer_native",
                "language": language,
                "profile": profile,
                "dpi": dpi,
                "printable_width_dots": printableWidthDots,
            ]
        }
    }

    func isAdvertised(by capability: PiqaePrintPacketSupportedOutputTarget) -> Bool {
        switch (self, capability) {
        case let (.pdf(profile), .pdf(supportedProfile)):
            profile == supportedProfile
        case let (
            .printerNative(language, profile, dpi, printableWidthDots),
            .printerNative(
                supportedLanguage,
                supportedProfile,
                supportedDPI,
                supportedPrintableWidthDots
            )
        ):
            language == supportedLanguage && profile == supportedProfile && dpi == supportedDPI
                && printableWidthDots == supportedPrintableWidthDots
        default:
            false
        }
    }
}

public enum PiqaePrintPacketSupportedOutputTarget: Codable, Equatable, Sendable {
    case pdf(profile: String)
    case printerNative(
        language: String,
        profile: String,
        dpi: UInt16,
        printableWidthDots: UInt32
    )

    private enum CodingKeys: String, CodingKey {
        case kind, profile, language, dpi
        case printableWidthDots = "printable_width_dots"
    }

    private enum Kind: String, Codable { case pdf; case printerNative = "printer_native" }

    public init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        switch try values.decode(Kind.self, forKey: .kind) {
        case .pdf:
            self = try .pdf(profile: values.decode(String.self, forKey: .profile))
        case .printerNative:
            self = try .printerNative(
                language: values.decode(String.self, forKey: .language),
                profile: values.decode(String.self, forKey: .profile),
                dpi: values.decode(UInt16.self, forKey: .dpi),
                printableWidthDots: values.decode(UInt32.self, forKey: .printableWidthDots)
            )
        }
    }

    public func encode(to encoder: any Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case let .pdf(profile):
            try values.encode(Kind.pdf, forKey: .kind)
            try values.encode(profile, forKey: .profile)
        case let .printerNative(language, profile, dpi, printableWidthDots):
            try values.encode(Kind.printerNative, forKey: .kind)
            try values.encode(language, forKey: .language)
            try values.encode(profile, forKey: .profile)
            try values.encode(dpi, forKey: .dpi)
            try values.encode(printableWidthDots, forKey: .printableWidthDots)
        }
    }
}

public struct PiqaePrintPacketHardLimits: Codable, Equatable, Sendable {
    public let maxTemplateBytes: UInt64
    public let maxDataBytes: UInt64
    public let maxOutputBytes: UInt64
    public let maxPages: UInt32
    public let maxResources: UInt32
    public let maxResourceBytes: UInt64
    public let maxTotalResourceBytes: UInt64

    enum CodingKeys: String, CodingKey {
        case maxTemplateBytes = "max_template_bytes"
        case maxDataBytes = "max_data_bytes"
        case maxOutputBytes = "max_output_bytes"
        case maxPages = "max_pages"
        case maxResources = "max_resources"
        case maxResourceBytes = "max_resource_bytes"
        case maxTotalResourceBytes = "max_total_resource_bytes"
    }
}

public struct PiqaePrintPacketCapabilities: Codable, Equatable, Sendable {
    public let contract: String
    public let rendererABI: String
    public let resourceABI: String
    public let rendererBuild: String
    public let conformanceProfile: String
    public let cacheProfile: String
    public let supportedFeatures: [String]
    public let supportedOutputTargets: [PiqaePrintPacketSupportedOutputTarget]
    public let resourceMediaTypes: [String]
    public let hardLimits: PiqaePrintPacketHardLimits
    public let persistentResourceCache: Bool
    public let directOfflineRendering: Bool

    enum CodingKeys: String, CodingKey {
        case contract
        case rendererABI = "renderer_abi"
        case resourceABI = "resource_abi"
        case rendererBuild = "renderer_build"
        case conformanceProfile = "conformance_profile"
        case cacheProfile = "cache_profile"
        case supportedFeatures = "supported_features"
        case supportedOutputTargets = "supported_output_targets"
        case resourceMediaTypes = "resource_media_types"
        case hardLimits = "hard_limits"
        case persistentResourceCache = "persistent_resource_cache"
        case directOfflineRendering = "direct_offline_rendering"
    }
}

/// Immutable PrintPacket input. JSON remains caller-owned wire data until the
/// bounded native command is assembled; NodeKit does not define a competing
/// template object model or renderer.
public struct PiqaePrintPacket: Equatable, Sendable {
    public let templateJSON: Data
    public let dataJSON: Data
    public let resources: [String: Data]
    public let outputTarget: PiqaePrintPacketOutputTarget

    public init(
        templateJSON: Data,
        dataJSON: Data = Data("{}".utf8),
        resources: [String: Data] = [:],
        outputTarget: PiqaePrintPacketOutputTarget = .pdf()
    ) throws {
        guard
            let decodedTemplate = try? JSONSerialization.jsonObject(with: templateJSON),
            let template = decodedTemplate as? [String: Any],
            template["format"] as? String == "printpacket/v1",
            (try? JSONSerialization.jsonObject(with: dataJSON, options: .fragmentsAllowed)) != nil
        else {
            throw PiqaeNodeError.invalidConfiguration(
                "PrintPacket requires a printpacket/v1 template object and valid JSON data."
            )
        }
        self.templateJSON = templateJSON
        self.dataJSON = dataJSON
        self.resources = resources
        self.outputTarget = outputTarget
    }
}

public struct PiqaePrintPacketSubmissionRequest: Equatable, Sendable {
    public let adapterID: String
    public let printerID: PiqaePrinterID
    public let idempotencyKey: String
    public let title: String
    public let packet: PiqaePrintPacket
    public let intent: PiqaePortablePrintIntent
    public let profileID: PiqaeProfileID?
    public let expiresAt: Date?
    let optionsJSON: String

    public init(
        adapterID: String,
        printerID: PiqaePrinterID,
        idempotencyKey: String,
        title: String,
        packet: PiqaePrintPacket,
        intent: PiqaePortablePrintIntent = .standard,
        profileID: PiqaeProfileID? = nil,
        expiresAt: Date? = nil
    ) throws {
        guard !adapterID.isEmpty, adapterID.utf8.count <= 256,
            !idempotencyKey.isEmpty, idempotencyKey.utf8.count <= 256,
            !title.isEmpty, title.utf8.count <= 512
        else {
            throw PiqaeNodeError.invalidConfiguration(
                "PrintPacket adapter, printer, idempotency key, or title is invalid."
            )
        }
        let optionsData = try JSONEncoder().encode(
            PiqaePrintPacketRuntimeOptions(intent: intent, profileID: profileID?.rawValue)
        )
        guard let optionsJSON = String(data: optionsData, encoding: .utf8) else {
            throw PiqaeNodeError.invalidConfiguration("PrintPacket options could not be encoded.")
        }
        self.adapterID = adapterID
        self.printerID = printerID
        self.idempotencyKey = idempotencyKey
        self.title = title
        self.packet = packet
        self.intent = intent
        self.profileID = profileID
        self.optionsJSON = optionsJSON
        self.expiresAt = expiresAt
    }
}

private struct PiqaePrintPacketRuntimeOptions: Encodable {
    let intent: PiqaePortablePrintIntent
    let profileID: String?
    enum CodingKeys: String, CodingKey { case intent; case profileID = "profile_id" }
}

public struct PiqaePrintPacketManifest: Codable, Equatable, Sendable {
    public let standard: String
    public let specificationVersion: String
    public let canonicalJSON: String
    public let canonicalSHA256: String
    public let canonicalBytes: UInt64
    public let requiredFeatures: [String]
    public let resourceCount: UInt32
    public let resourceBytes: UInt64

    enum CodingKeys: String, CodingKey {
        case standard
        case specificationVersion = "specification_version"
        case canonicalJSON = "canonical_json"
        case canonicalSHA256 = "canonical_sha256"
        case canonicalBytes = "canonical_bytes"
        case requiredFeatures = "required_features"
        case resourceCount = "resource_count"
        case resourceBytes = "resource_bytes"
    }
}

public struct PiqaePrintPacketOutput: Codable, Equatable, Sendable {
    public let mediaType: String
    public let profile: String
    public let sha256: String
    public let bytes: UInt64
    public let pages: UInt32

    enum CodingKeys: String, CodingKey {
        case mediaType = "media_type"
        case profile, sha256, bytes, pages
    }
}

public struct PiqaePrintPacketValidation: Codable, Equatable, Sendable {
    public let manifest: PiqaePrintPacketManifest
    public let cacheKey: String
    public let output: PiqaePrintPacketOutput

    enum CodingKeys: String, CodingKey {
        case manifest
        case cacheKey = "cache_key"
        case output
    }
}

public struct PiqaePrintPacketSubmission: Codable, Equatable, Sendable {
    public let job: PiqaeRuntimeJobAccepted
    public let manifest: PiqaePrintPacketManifest
    public let cacheKey: String
    public let output: PiqaePrintPacketOutput

    enum CodingKeys: String, CodingKey {
        case job, manifest
        case cacheKey = "cache_key"
        case output
    }
}
