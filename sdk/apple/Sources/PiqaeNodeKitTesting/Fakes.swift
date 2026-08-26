import Foundation
import PiqaeNodeKit

public actor PiqaeMemoryInstallationIdentityStore: PiqaeInstallationIdentityStore {
    private let id: PiqaeInstallationID

    public init(id: PiqaeInstallationID = .init(rawValue: "ins_apple_test")) {
        self.id = id
    }

    public func loadOrCreateInstallationID() async throws -> PiqaeInstallationID { id }
}

public actor PiqaeFakeEmbeddedRuntime: PiqaeEmbeddedNodeRuntime {
    public enum StartFailure: Error { case requested }
    public enum ConnectFailure: Error { case requested }

    private let failsToStart: Bool
    private let failsToConnect: Bool
    private let connector: PiqaeRuntimeConnectorSnapshot?
    private var nextOperationDelayNanoseconds: UInt64
    private var workAvailableHandler: (@Sendable () -> Void)?
    private var operationsByAdapter: [String: [PiqaeRuntimeAdapterOperation]] = [:]
    private var nextOperationCallCountValue = 0
    private var activeNextOperationCalls = 0
    private var maximumConcurrentNextOperationCallsValue = 0
    public private(set) var startCount = 0
    public private(set) var stopCount = 0
    public private(set) var connectCount = 0
    public private(set) var lifecycleEvents: [PiqaeHostLifecycleEvent] = []

    public init(
        failsToStart: Bool = false,
        failsToConnect: Bool = false,
        connector: PiqaeRuntimeConnectorSnapshot? = nil,
        nextOperationDelayNanoseconds: UInt64 = 0
    ) {
        self.failsToStart = failsToStart
        self.failsToConnect = failsToConnect
        self.connector = connector
        self.nextOperationDelayNanoseconds = nextOperationDelayNanoseconds
    }

    public func setWorkAvailableHandler(
        _ handler: @escaping @Sendable () -> Void
    ) async throws {
        workAvailableHandler = handler
    }

    public func start() async throws {
        startCount += 1
        if failsToStart { throw StartFailure.requested }
    }

    public func stop() async throws {
        stopCount += 1
        workAvailableHandler = nil
    }

    public func report(_ event: PiqaeHostLifecycleEvent) async throws {
        lifecycleEvents.append(event)
    }

    public func registerAdapter(_ registration: PiqaeRuntimeAdapterRegistration) async throws {}

    public func observePrinterInventory(
        adapterID: String,
        printers: [PiqaeRuntimePrinterObservation]
    ) async throws -> [PiqaeRuntimePrinterSnapshot] {
        printers.map { printer in
            PiqaeRuntimePrinterSnapshot(
                printerID: printer.nativeID.replacingOccurrences(of: "virtual://", with: ""),
                adapterID: adapterID,
                nativeID: printer.nativeID,
                name: printer.name,
                state: printer.state,
                observedUnixMilliseconds: 1_700_000_000_000
            )
        }
    }

    public func nextOperation(adapterID: String) async throws -> PiqaeRuntimeAdapterOperation? {
        nextOperationCallCountValue += 1
        activeNextOperationCalls += 1
        maximumConcurrentNextOperationCallsValue = max(
            maximumConcurrentNextOperationCallsValue,
            activeNextOperationCalls
        )
        defer { activeNextOperationCalls -= 1 }
        if nextOperationDelayNanoseconds > 0 {
            try await Task.sleep(nanoseconds: nextOperationDelayNanoseconds)
        }
        return operationsByAdapter[adapterID]?.first
    }

    public func beginHandoff(
        _ operation: PiqaeRuntimeAdapterOperation
    ) async throws -> PiqaeRuntimeAdapterOperation {
        copy(operation, phase: .handoffStarted, nativeJobID: nil)
    }

    public func complete(
        _ operation: PiqaeRuntimeAdapterOperation,
        outcome: PiqaeRuntimeAdapterOutcome
    ) async throws -> PiqaeRuntimeAdapterAcknowledgement {
        operationsByAdapter[operation.adapterID]?.removeAll {
            $0.operationID == operation.operationID
        }
        return PiqaeRuntimeAdapterAcknowledgement(
            operationID: operation.operationID,
            jobID: operation.jobID,
            state: "completed_reported"
        )
    }

    /// Adds durable work as if it arrived from a cloud connector. Tests may
    /// deliberately issue duplicate notifications to verify host coalescing.
    public func activateRemoteOperation(
        _ operation: PiqaeRuntimeAdapterOperation,
        notificationCount: Int = 1
    ) {
        operationsByAdapter[operation.adapterID, default: []].append(operation)
        guard let workAvailableHandler else { return }
        for _ in 0..<max(0, notificationCount) { workAvailableHandler() }
    }

    public func notifyWorkAvailable(count: Int = 1) {
        guard let workAvailableHandler else { return }
        for _ in 0..<max(0, count) { workAvailableHandler() }
    }

    public func setNextOperationDelayNanoseconds(_ value: UInt64) {
        nextOperationDelayNanoseconds = value
    }

    public func nextOperationCallCount() -> Int { nextOperationCallCountValue }

    public func maximumConcurrentNextOperationCalls() -> Int {
        maximumConcurrentNextOperationCallsValue
    }

    public func hasWorkAvailableHandler() -> Bool { workAvailableHandler != nil }

    public func connectInvitation(_ request: PiqaeEnrollmentRequest) async throws
        -> PiqaeRuntimeConnectorSnapshot
    {
        connectCount += 1
        if failsToConnect { throw ConnectFailure.requested }
        guard let connector else {
            throw PiqaeNodeError.unsupportedOperation("No fake connector was configured.")
        }
        return connector
    }

    private func copy(
        _ operation: PiqaeRuntimeAdapterOperation,
        phase: PiqaeRuntimeAdapterOperationPhase,
        nativeJobID: String?
    ) -> PiqaeRuntimeAdapterOperation {
        PiqaeRuntimeAdapterOperation(
            operationID: operation.operationID,
            adapterID: operation.adapterID,
            jobID: operation.jobID,
            idempotencyKey: operation.idempotencyKey,
            fence: operation.fence,
            deadlineUnixMilliseconds: operation.deadlineUnixMilliseconds,
            printerID: operation.printerID,
            printerNativeID: operation.printerNativeID,
            title: operation.title,
            contentPath: operation.contentPath,
            contentKind: operation.contentKind,
            contentSHA256: operation.contentSHA256,
            optionsJSON: operation.optionsJSON,
            phase: phase,
            nativeJobID: nativeJobID
        )
    }
}

public actor PiqaeFakePrinterAdapter: PiqaePrinterAdapter {
    public enum SubmissionBehavior: Equatable, Sendable {
        case acceptedAndCompleted
        case acceptedWithoutNativeID
        case throwAfterHandoff
    }

    public enum SubmissionFailure: Error { case requested }
    public nonisolated let adapterID: String
    public nonisolated let descriptor: PiqaePrinterAdapterDescriptor
    private var inventory: [PiqaePrinter]
    private var profileInventory: [PiqaePrinterID: [PiqaePrintProfile]]
    private var submissionCountValue = 0
    private let submissionBehavior: SubmissionBehavior
    private let now: Date

    public init(
        adapterID: String = "fake.printer",
        printers: [PiqaePrinter],
        profiles: [PiqaePrinterID: [PiqaePrintProfile]] = [:],
        submissionBehavior: SubmissionBehavior = .acceptedAndCompleted,
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
        self.submissionBehavior = submissionBehavior
        self.now = now
    }

    public func discoverPrinters() async throws -> [PiqaePrinter] { inventory }

    public func submit(
        _ request: PiqaePrintRequest,
        to printer: PiqaePrinter
    ) async throws -> PiqaeJobReceipt {
        submissionCountValue += 1
        if submissionBehavior == .throwAfterHandoff { throw SubmissionFailure.requested }
        return PiqaeJobReceipt(
            jobID: .init(rawValue: "job_fake_\(submissionCountValue)"),
            nativeJobID: submissionBehavior == .acceptedWithoutNativeID
                ? nil : "native_fake_\(submissionCountValue)",
            handoffState: .acceptedBySpooler,
            acceptedAt: now
        )
    }

    public func observeNativeJob(
        nativeJobID: String,
        printer: PiqaePrinter
    ) async throws -> PiqaeNativeJobObservation {
        submissionBehavior == .acceptedAndCompleted ? .completedReported : .unknown
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

public struct PiqaeFixedHostKeyStore: PiqaeHostKeyStore {
    private let key: Data

    public init(key: Data = Data(repeating: 7, count: 32)) {
        self.key = key
    }

    public func loadOrCreateKey() throws -> Data { key }
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
