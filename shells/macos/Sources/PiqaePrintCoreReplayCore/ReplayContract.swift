import Foundation

public struct PrintCoreReplayRequest: Codable, Equatable, Sendable {
    public let printerNativeID: String
    public let pdfPath: String
    public let jobTitle: String
    public let nativeProfile: PrintCoreNativeProfile
    public let portableOptions: PortablePrintOptions
    public let safeOverrides: [SafePrintOverride]

    public init(
        printerNativeID: String,
        pdfPath: String,
        jobTitle: String,
        nativeProfile: PrintCoreNativeProfile,
        portableOptions: PortablePrintOptions = .init(),
        safeOverrides: [SafePrintOverride] = []
    ) {
        self.printerNativeID = printerNativeID
        self.pdfPath = pdfPath
        self.jobTitle = jobTitle
        self.nativeProfile = nativeProfile
        self.portableOptions = portableOptions
        self.safeOverrides = safeOverrides
    }

    enum CodingKeys: String, CodingKey {
        case printerNativeID = "printer_native_id"
        case pdfPath = "pdf_path"
        case jobTitle = "job_title"
        case nativeProfile = "native_profile"
        case portableOptions = "portable_options"
        case safeOverrides = "safe_overrides"
    }
}

public struct PrintCoreNativeProfile: Codable, Equatable, Sendable {
    public let kind: String
    public let schemaVersion: UInt16
    public let digest: String
    public let blob: Data

    public init(
        kind: String,
        schemaVersion: UInt16,
        digest: String,
        blob: Data
    ) {
        self.kind = kind
        self.schemaVersion = schemaVersion
        self.digest = digest
        self.blob = blob
    }

    enum CodingKeys: String, CodingKey {
        case kind
        case schemaVersion = "schema_version"
        case digest
        case blob = "blob_base64"
    }
}

public enum SafePrintOverride: String, Codable, CaseIterable, Sendable {
    case bin
    case collate
    case color
    case copies
    case dpi
    case duplex
    case fitToPage = "fit_to_page"
    case media
    case nup
    case pages
    case paper
    case rotate
}

public enum PortableDuplex: String, Codable, Sendable {
    case oneSided = "one-sided"
    case longEdge = "long-edge"
    case shortEdge = "short-edge"
}

public enum PortableRotation: String, Codable, Sendable {
    case degrees0 = "0"
    case degrees90 = "90"
    case degrees180 = "180"
    case degrees270 = "270"
}

public struct PortablePrintOptions: Codable, Equatable, Sendable {
    public var bin: String?
    public var collate: Bool?
    public var color: Bool?
    public var copies: UInt32?
    public var dpi: String?
    public var duplex: PortableDuplex?
    public var fitToPage: Bool?
    public var media: String?
    public var nup: UInt16?
    public var pages: String?
    public var paper: String?
    public var rotate: PortableRotation?
    public var nativeOptions: [String: String]

    public init(
        bin: String? = nil,
        collate: Bool? = nil,
        color: Bool? = nil,
        copies: UInt32? = nil,
        dpi: String? = nil,
        duplex: PortableDuplex? = nil,
        fitToPage: Bool? = nil,
        media: String? = nil,
        nup: UInt16? = nil,
        pages: String? = nil,
        paper: String? = nil,
        rotate: PortableRotation? = nil,
        nativeOptions: [String: String] = [:]
    ) {
        self.bin = bin
        self.collate = collate
        self.color = color
        self.copies = copies
        self.dpi = dpi
        self.duplex = duplex
        self.fitToPage = fitToPage
        self.media = media
        self.nup = nup
        self.pages = pages
        self.paper = paper
        self.rotate = rotate
        self.nativeOptions = nativeOptions
    }

    enum CodingKeys: String, CodingKey {
        case bin
        case collate
        case color
        case copies
        case dpi
        case duplex
        case fitToPage = "fit_to_page"
        case media
        case nup
        case pages
        case paper
        case rotate
        case nativeOptions = "native_options"
    }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        bin = try values.decodeIfPresent(String.self, forKey: .bin)
        collate = try values.decodeIfPresent(Bool.self, forKey: .collate)
        color = try values.decodeIfPresent(Bool.self, forKey: .color)
        copies = try values.decodeIfPresent(UInt32.self, forKey: .copies)
        dpi = try values.decodeIfPresent(String.self, forKey: .dpi)
        duplex = try values.decodeIfPresent(PortableDuplex.self, forKey: .duplex)
        fitToPage = try values.decodeIfPresent(Bool.self, forKey: .fitToPage)
        media = try values.decodeIfPresent(String.self, forKey: .media)
        nup = try values.decodeIfPresent(UInt16.self, forKey: .nup)
        pages = try values.decodeIfPresent(String.self, forKey: .pages)
        paper = try values.decodeIfPresent(String.self, forKey: .paper)
        rotate = try values.decodeIfPresent(PortableRotation.self, forKey: .rotate)
        nativeOptions =
            try values.decodeIfPresent([String: String].self, forKey: .nativeOptions) ?? [:]
    }
}

public struct PrintCoreReplayResponse: Codable, Equatable, Sendable {
    public let ok: Bool
    public let nativeJobID: String?
    public let code: String?
    public let message: String?
    public let retryable: Bool
    public let handoffMayHaveSucceeded: Bool

    public init(
        ok: Bool,
        nativeJobID: String? = nil,
        code: String? = nil,
        message: String? = nil,
        retryable: Bool = false,
        handoffMayHaveSucceeded: Bool = false
    ) {
        self.ok = ok
        self.nativeJobID = nativeJobID
        self.code = code
        self.message = message
        self.retryable = retryable
        self.handoffMayHaveSucceeded = handoffMayHaveSucceeded
    }

    enum CodingKeys: String, CodingKey {
        case ok
        case nativeJobID = "native_job_id"
        case code
        case message
        case retryable
        case handoffMayHaveSucceeded = "handoff_may_have_succeeded"
    }
}
