import Foundation
import PiqaeNodeKit

public actor PiqaeMemoryInstallationIdentityStore: PiqaeInstallationIdentityStore {
    private let id: PiqaeInstallationID

    public init(id: PiqaeInstallationID = .init(rawValue: "ins_apple_test")) {
        self.id = id
    }

    public func loadOrCreateInstallationID() async throws -> PiqaeInstallationID { id }
}

public actor PiqaeFakePrinterAdapter: PiqaePrinterAdapter {
    public nonisolated let adapterID: String
    public nonisolated let descriptor: PiqaePrinterAdapterDescriptor
    private var inventory: [PiqaePrinter]
    private var profileInventory: [PiqaePrinterID: [PiqaePrintProfile]]
    private var submissionCountValue = 0
    private var receiptsByIdempotencyKey: [String: PiqaeJobReceipt] = [:]
    private let now: Date

    public init(
        adapterID: String = "fake.printer",
        printers: [PiqaePrinter],
        profiles: [PiqaePrinterID: [PiqaePrintProfile]] = [:],
        now: Date = Date(timeIntervalSince1970: 1_700_000_000)
    ) {
        self.adapterID = adapterID
        descriptor = PiqaePrinterAdapterDescriptor(
            id: adapterID,
            displayName: "Deterministic fake printer",
            version: "1",
            transports: [.vendorSDK],
            portableOptions: PiqaePortableOption.allCases,
            supportsProfiles: true
        )
        inventory = printers
        profileInventory = profiles
        self.now = now
    }

    public func discoverPrinters() async throws -> [PiqaePrinter] { inventory }

    public func submit(
        _ request: PiqaePrintRequest,
        to printer: PiqaePrinter
    ) async throws -> PiqaeJobReceipt {
        if let prior = receiptsByIdempotencyKey[request.idempotencyKey] { return prior }
        submissionCountValue += 1
        let receipt = PiqaeJobReceipt(
            jobID: .init(rawValue: "job_fake_\(submissionCountValue)"),
            nativeJobID: "native_fake_\(submissionCountValue)",
            handoffState: .acceptedBySpooler,
            acceptedAt: now
        )
        receiptsByIdempotencyKey[request.idempotencyKey] = receipt
        return receipt
    }

    public func profiles(for printer: PiqaePrinter) async throws -> [PiqaePrintProfile] {
        profileInventory[printer.id] ?? []
    }

    public func captureProfile(
        _ request: PiqaeProfileCaptureRequest,
        for printer: PiqaePrinter
    ) async throws -> PiqaePrintProfile {
        let profile = PiqaePrintProfile(
            id: .init(rawValue: "prf_fake_\((profileInventory[printer.id] ?? []).count + 1)"),
            printerID: printer.id,
            name: request.name,
            revision: 1
        )
        profileInventory[printer.id, default: []].append(profile)
        return profile
    }

    public func replacePrinters(_ printers: [PiqaePrinter]) {
        inventory = printers
    }

    public func submissionCount() -> Int { submissionCountValue }

    public static func printer(
        id: String = "prn_fake",
        adapterID: String = "fake.printer",
        name: String = "Virtual receipt printer",
        state: PiqaePrinterState = .available,
        now: Date = Date(timeIntervalSince1970: 1_700_000_000)
    ) -> PiqaePrinter {
        PiqaePrinter(
            id: .init(rawValue: id),
            adapterID: adapterID,
            adapterFingerprint: .init(
                platform: .iosNetwork,
                adapterID: adapterID,
                adapterVersion: "1",
                deviceFamily: "Piqae deterministic fake"
            ),
            nativeID: "virtual://\(id)",
            displayName: name,
            model: "Piqae deterministic fake",
            state: state,
            observedAt: now,
            freshUntil: now.addingTimeInterval(60)
        )
    }
}

public actor PiqaeFakeEnrollmentProvider: PiqaeCloudEnrollmentProvider {
    private let connection: PiqaeConnection
    private var requestCountValue = 0

    public init(connection: PiqaeConnection) {
        self.connection = connection
    }

    public func enroll(_ request: PiqaeEnrollmentRequest) async throws -> PiqaeConnection {
        requestCountValue += 1
        return connection
    }

    public func requestCount() -> Int { requestCountValue }
}

public actor PiqaeFakeLifecycleReporter: PiqaeHostLifecycleReporter {
    public private(set) var events: [PiqaeHostLifecycleEvent] = []
    public var error: (any Error & Sendable)?

    public init() {}

    public func report(_ event: PiqaeHostLifecycleEvent) async throws {
        if let error { throw error }
        events.append(event)
    }
}

public actor PiqaeFakeRemoteNotificationProvider:
    PiqaeRemoteNotificationRegistrationProvider
{
    public private(set) var registrations: [PiqaeRemoteNotificationRegistration] = []
    public var error: (any Error & Sendable)?

    public init() {}

    public func register(_ request: PiqaeRemoteNotificationRegistration) async throws {
        if let error { throw error }
        registrations.append(request)
    }
}

public actor PiqaeFakeInstalledNodeIPC: PiqaeInstalledNodeIPC {
    private let protocolVersion: UInt32?
    private var snapshotValue: PiqaeNodeSnapshot
    private var submissionCountValue = 0

    public init(protocolVersion: UInt32?, snapshot: PiqaeNodeSnapshot) {
        self.protocolVersion = protocolVersion
        snapshotValue = snapshot
    }

    public func probe() async -> PiqaeInstalledNodeProbe {
        if let protocolVersion {
            return .init(state: .available(protocolVersion: protocolVersion))
        }
        return .init(state: .unavailable)
    }

    public func snapshot() async throws -> PiqaeNodeSnapshot { snapshotValue }

    public func connect(_ request: PiqaeEnrollmentRequest) async throws -> PiqaeConnection {
        PiqaeConnection(
            id: .init(rawValue: "ncon_attached_test"),
            authorityURL: request.authorityURL,
            workspaceName: "Attached test workspace",
            state: .connected
        )
    }

    public func submit(_ request: PiqaePrintRequest) async throws -> PiqaeJobReceipt {
        submissionCountValue += 1
        return PiqaeJobReceipt(
            jobID: .init(rawValue: "job_attached_\(submissionCountValue)"),
            nativeJobID: "native_attached_\(submissionCountValue)",
            handoffState: .acceptedBySpooler,
            acceptedAt: Date(timeIntervalSince1970: 1_700_000_000)
        )
    }

    public func replaceSnapshot(_ snapshot: PiqaeNodeSnapshot) {
        snapshotValue = snapshot
    }

    public func submissionCount() -> Int { submissionCountValue }
}
