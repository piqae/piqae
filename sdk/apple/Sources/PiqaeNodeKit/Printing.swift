import Foundation

public enum PiqaePrintContent: Sendable {
    case pdf(Data)
    case image(Data, typeIdentifier: String)
    case raw(Data, mediaType: String)

    public var byteCount: Int {
        switch self {
        case let .pdf(data), let .image(data, _), let .raw(data, _): data.count
        }
    }
}

public struct PiqaePortablePrintIntent: Codable, Equatable, Sendable {
    public enum Orientation: String, Codable, Sendable {
        case portrait
        case landscape
    }

    public enum Cut: String, Codable, Sendable {
        case none
        case afterJob = "after_job"
        case afterPage = "after_page"
    }

    public let copies: UInt16
    public let media: String?
    public let orientation: Orientation?
    public let cut: Cut?
    public let density: Int16?

    public static let standard = PiqaePortablePrintIntent(
        validatedCopies: 1,
        media: nil,
        orientation: nil,
        cut: nil,
        density: nil
    )

    public init(
        copies: UInt16 = 1,
        media: String? = nil,
        orientation: Orientation? = nil,
        cut: Cut? = nil,
        density: Int16? = nil
    ) throws {
        guard copies > 0, copies <= 999 else {
            throw PiqaeNodeError.invalidConfiguration("Copies must be between 1 and 999.")
        }
        self.copies = copies
        self.media = media
        self.orientation = orientation
        self.cut = cut
        self.density = density
    }

    private init(
        validatedCopies copies: UInt16,
        media: String?,
        orientation: Orientation?,
        cut: Cut?,
        density: Int16?
    ) {
        self.copies = copies
        self.media = media
        self.orientation = orientation
        self.cut = cut
        self.density = density
    }
}

public struct PiqaePrintRequest: Sendable {
    public let printerID: PiqaePrinterID
    public let title: String
    public let content: PiqaePrintContent
    public let intent: PiqaePortablePrintIntent
    public let profileID: PiqaeProfileID?
    public let idempotencyKey: String

    public init(
        printerID: PiqaePrinterID,
        title: String,
        content: PiqaePrintContent,
        intent: PiqaePortablePrintIntent = .standard,
        profileID: PiqaeProfileID? = nil,
        idempotencyKey: String
    ) throws {
        let trimmedTitle = title.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedKey = idempotencyKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedTitle.isEmpty, trimmedTitle.utf8.count <= 256 else {
            throw PiqaeNodeError.invalidConfiguration("Job titles must contain 1 to 256 UTF-8 bytes.")
        }
        guard !trimmedKey.isEmpty, trimmedKey.utf8.count <= 200 else {
            throw PiqaeNodeError.invalidConfiguration("Idempotency keys must contain 1 to 200 UTF-8 bytes.")
        }
        guard content.byteCount > 0, content.byteCount <= 100 * 1024 * 1024 else {
            throw PiqaeNodeError.invalidConfiguration("Print content must contain 1 byte to 100 MiB.")
        }
        self.printerID = printerID
        self.title = trimmedTitle
        self.content = content
        self.intent = intent
        self.profileID = profileID
        self.idempotencyKey = trimmedKey
    }
}

public enum PiqaeNativeHandoffState: String, Codable, Sendable {
    case acceptedBySpooler = "accepted_by_spooler"
    case deliveryUncertain = "delivery_uncertain"
}

public struct PiqaeJobReceipt: Codable, Equatable, Sendable {
    public let jobID: PiqaeJobID
    public let nativeJobID: String?
    public let handoffState: PiqaeNativeHandoffState
    public let acceptedAt: Date

    public init(
        jobID: PiqaeJobID,
        nativeJobID: String?,
        handoffState: PiqaeNativeHandoffState,
        acceptedAt: Date
    ) {
        self.jobID = jobID
        self.nativeJobID = nativeJobID
        self.handoffState = handoffState
        self.acceptedAt = acceptedAt
    }
}

public struct PiqaeProfileCaptureRequest: Sendable {
    public let printerID: PiqaePrinterID
    public let name: String

    public init(printerID: PiqaePrinterID, name: String) throws {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, trimmed.utf8.count <= 128 else {
            throw PiqaeNodeError.invalidConfiguration("Profile names must contain 1 to 128 UTF-8 bytes.")
        }
        self.printerID = printerID
        self.name = trimmed
    }
}

public protocol PiqaePrinterAdapter: Sendable {
    var adapterID: String { get }
    var descriptor: PiqaePrinterAdapterDescriptor { get }
    func discoverPrinters() async throws -> [PiqaePrinter]
    func validate(_ request: PiqaePrintRequest, for printer: PiqaePrinter) async throws
    /// Must honor `request.idempotencyKey`. Durable runtimes persist this
    /// mapping; adapters must never reinterpret a retry as another copy.
    func submit(_ request: PiqaePrintRequest, to printer: PiqaePrinter) async throws -> PiqaeJobReceipt
    func profiles(for printer: PiqaePrinter) async throws -> [PiqaePrintProfile]
    func captureProfile(
        _ request: PiqaeProfileCaptureRequest,
        for printer: PiqaePrinter
    ) async throws -> PiqaePrintProfile
}

public extension PiqaePrinterAdapter {
    var descriptor: PiqaePrinterAdapterDescriptor {
        PiqaePrinterAdapterDescriptor(
            id: adapterID,
            displayName: adapterID,
            version: "unspecified",
            transports: [.vendorSDK],
            portableOptions: [],
            supportsProfiles: false
        )
    }

    func validate(_ request: PiqaePrintRequest, for printer: PiqaePrinter) async throws {}

    func profiles(for printer: PiqaePrinter) async throws -> [PiqaePrintProfile] { [] }

    func captureProfile(
        _ request: PiqaeProfileCaptureRequest,
        for printer: PiqaePrinter
    ) async throws -> PiqaePrintProfile {
        throw PiqaeNodeError.unsupportedOperation(
            "Adapter \(adapterID) does not support native profile capture."
        )
    }
}
