import Foundation

public final class PiqaeNode: @unchecked Sendable {
    private let engine: PiqaeNodeEngine

    public let connections: PiqaeConnectionsService
    public let printers: PiqaePrintersService
    public let jobs: PiqaeJobsService
    public let profiles: PiqaeProfilesService

    public init(_ configuration: PiqaeNodeConfiguration) {
        let engine = PiqaeNodeEngine(configuration: configuration)
        self.engine = engine
        connections = PiqaeConnectionsService(engine: engine)
        printers = PiqaePrintersService(engine: engine)
        jobs = PiqaeJobsService(engine: engine)
        profiles = PiqaeProfilesService(engine: engine)
    }

    public func start() async throws {
        try await engine.start()
    }

    public func stop() async {
        await engine.stop()
    }

    public func snapshot() async -> PiqaeNodeSnapshot {
        await engine.currentSnapshot()
    }

    public func observe() async -> AsyncStream<PiqaeNodeSnapshot> {
        await engine.observe()
    }

    public func updateExecutionContext(_ context: PiqaeExecutionContext) async {
        await engine.updateExecutionContext(context)
    }

    public func handleWakeHint(
        _ hint: PiqaeWakeHint,
        context: PiqaeExecutionContext
    ) async -> PiqaeWakeHintResult {
        await engine.handleWakeHint(hint, context: context)
    }
}

public final class PiqaeConnectionsService: @unchecked Sendable {
    private let engine: PiqaeNodeEngine
    fileprivate init(engine: PiqaeNodeEngine) { self.engine = engine }

    public func list() async throws -> [PiqaeConnection] {
        try await engine.connections()
    }

    public func connect(_ configuration: PiqaeCloudConfiguration) async throws -> PiqaeConnection {
        try await engine.connect(configuration)
    }

    public func observe() async -> AsyncStream<[PiqaeConnection]> {
        let snapshots = await engine.observe()
        return AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
            let task = Task {
                for await snapshot in snapshots {
                    guard !Task.isCancelled else { break }
                    continuation.yield(snapshot.connections)
                }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}

public final class PiqaePrintersService: @unchecked Sendable {
    private let engine: PiqaeNodeEngine
    fileprivate init(engine: PiqaeNodeEngine) { self.engine = engine }

    public func list(refresh: Bool = false) async throws -> [PiqaePrinter] {
        if refresh { try await engine.refresh() }
        return try await engine.printers()
    }

    public func refresh() async throws {
        try await engine.refresh()
    }

    public func adapters() async throws -> [PiqaePrinterAdapterDescriptor] {
        try await engine.adapterDescriptors()
    }

    public func observe() async -> AsyncStream<[PiqaePrinter]> {
        let snapshots = await engine.observe()
        return AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
            let task = Task {
                for await snapshot in snapshots {
                    guard !Task.isCancelled else { break }
                    continuation.yield(snapshot.printers)
                }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}

public final class PiqaeJobsService: @unchecked Sendable {
    private let engine: PiqaeNodeEngine
    fileprivate init(engine: PiqaeNodeEngine) { self.engine = engine }

    /// Returns native spooler acceptance, never proof of physical paper output.
    public func submit(_ request: PiqaePrintRequest) async throws -> PiqaeJobReceipt {
        try await engine.submit(request)
    }
}

public final class PiqaeProfilesService: @unchecked Sendable {
    private let engine: PiqaeNodeEngine
    fileprivate init(engine: PiqaeNodeEngine) { self.engine = engine }

    public func list(for printerID: PiqaePrinterID) async throws -> [PiqaePrintProfile] {
        try await engine.profiles(for: printerID)
    }

    public func capture(_ request: PiqaeProfileCaptureRequest) async throws -> PiqaePrintProfile {
        try await engine.captureProfile(request)
    }
}

private actor PiqaeProcessRuntimeRegistry {
    static let shared = PiqaeProcessRuntimeRegistry()
    private var owners: Set<PiqaeInstallationID> = []

    func acquire(_ id: PiqaeInstallationID) -> Bool {
        owners.insert(id).inserted
    }

    func release(_ id: PiqaeInstallationID) {
        owners.remove(id)
    }
}

actor PiqaeNodeEngine {
    private let configuration: PiqaeNodeConfiguration
    private let admissionPolicy = PiqaeBackgroundAdmissionPolicy()
    private var started = false
    private var ownsEmbeddedRuntime = false
    private var executionContext = PiqaeExecutionContext.foreground
    private var selectedIPC: (any PiqaeInstalledNodeIPC)?
    private var adaptersByID: [String: any PiqaePrinterAdapter] = [:]
    private var observers: [UUID: AsyncStream<PiqaeNodeSnapshot>.Continuation] = [:]
    private var snapshotValue: PiqaeNodeSnapshot

    init(configuration: PiqaeNodeConfiguration) {
        self.configuration = configuration
        let hostMode: PiqaeNodeHostMode = configuration.startupMode == .attach
            ? .attachedClient : .embeddedApplication
        snapshotValue = PiqaeNodeSnapshot(
            installationID: nil,
            hostMode: hostMode,
            availability: configuration.availability,
            phase: .stopped,
            connections: [],
            printers: [],
            lastUpdatedAt: Date()
        )
    }

    func start() async throws {
        guard !started else { throw PiqaeNodeError.alreadyStarted }
        setPhase(.starting)

        do {
            #if os(iOS)
            guard configuration.startupMode != .attach else {
                throw PiqaeNodeError.unsupportedHostMode
            }
            try await startEmbedded()
            #else
            switch configuration.startupMode {
            case .embedded:
                try await startEmbedded()
            case .attach:
                try await startAttached(required: true)
            case .automatic:
                if let ipc = configuration.installedNodeIPC {
                    switch await ipc.probe().state {
                    case .unavailable:
                        try await startEmbedded()
                    case let .available(version):
                        guard Self.supports(version) else {
                            throw PiqaeNodeError.incompatibleInstalledNode(
                                found: version,
                                supported: PiqaeNodeConfiguration.supportedLocalProtocolVersions
                            )
                        }
                        try await attach(ipc)
                    }
                } else {
                    try await startEmbedded()
                }
            }
            #endif

            started = true
            if case let .cloud(cloud) = configuration.connectivity {
                _ = try await connect(cloud)
            }
            snapshotValue = replacingSnapshot(phase: .ready, statusMessage: nil)
            emit()
        } catch {
            if ownsEmbeddedRuntime, let id = snapshotValue.installationID {
                await PiqaeProcessRuntimeRegistry.shared.release(id)
                ownsEmbeddedRuntime = false
            }
            started = false
            selectedIPC = nil
            adaptersByID.removeAll(keepingCapacity: true)
            snapshotValue = replacingSnapshot(
                phase: .degraded,
                statusMessage: Self.redactedMessage(for: error)
            )
            emit()
            throw error
        }
    }

    func stop() async {
        if ownsEmbeddedRuntime, let id = snapshotValue.installationID {
            await PiqaeProcessRuntimeRegistry.shared.release(id)
        }
        ownsEmbeddedRuntime = false
        selectedIPC = nil
        started = false
        snapshotValue = replacingSnapshot(phase: .stopped, statusMessage: nil)
        emit()
    }

    func currentSnapshot() -> PiqaeNodeSnapshot { snapshotValue }

    func observe() -> AsyncStream<PiqaeNodeSnapshot> {
        let observerID = UUID()
        return AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
            observers[observerID] = continuation
            continuation.yield(snapshotValue)
            continuation.onTermination = { [weak self] _ in
                Task { await self?.removeObserver(observerID) }
            }
        }
    }

    func connections() throws -> [PiqaeConnection] {
        try requireStarted()
        return snapshotValue.connections.isEmpty ? [.localOnly] : snapshotValue.connections
    }

    func printers() throws -> [PiqaePrinter] {
        try requireStarted()
        return snapshotValue.printers
    }

    func adapterDescriptors() throws -> [PiqaePrinterAdapterDescriptor] {
        try requireStarted()
        if selectedIPC != nil {
            return [
                PiqaePrinterAdapterDescriptor(
                    id: "piqae.installed-node",
                    displayName: "Installed Piqae node",
                    version: "local-protocol-1",
                    transports: [.installedDriver],
                    portableOptions: PiqaePortableOption.allCases,
                    supportsProfiles: true
                ),
            ]
        }
        return adaptersByID.values.map(\.descriptor).sorted { $0.id < $1.id }
    }

    func refresh() async throws {
        try requireStarted()
        if let selectedIPC {
            snapshotValue = normalizedAttachedSnapshot(try await selectedIPC.snapshot())
            emit()
            return
        }
        let discovered = try await discoverEmbeddedPrinters()
        snapshotValue = replacingSnapshot(printers: discovered, statusMessage: nil)
        emit()
    }

    func connect(_ cloud: PiqaeCloudConfiguration) async throws -> PiqaeConnection {
        guard let installationID = snapshotValue.installationID else {
            throw PiqaeNodeError.notStarted
        }
        let request = PiqaeEnrollmentRequest(
            authorityURL: cloud.authorityURL,
            invitation: cloud.invitation,
            installationID: installationID,
            hostMode: snapshotValue.hostMode,
            availability: snapshotValue.availability
        )
        let connection: PiqaeConnection
        if let selectedIPC {
            connection = try await selectedIPC.connect(request)
        } else {
            connection = try await cloud.provider.enroll(request)
        }
        var connections = snapshotValue.connections.filter {
            $0.id != connection.id && $0.state != .localOnly
        }
        connections.append(connection)
        snapshotValue = replacingSnapshot(
            connections: connections.sorted { $0.id.rawValue < $1.id.rawValue },
            statusMessage: nil
        )
        emit()
        return connection
    }

    func submit(_ request: PiqaePrintRequest) async throws -> PiqaeJobReceipt {
        try requireStarted()
        let handoff = PiqaePendingHandoff(
            payloadIsDurable: false,
            estimatedSecondsToNativeAcceptance: 10
        )
        switch admissionPolicy.evaluate(
            handoff,
            context: executionContext,
            availability: snapshotValue.availability
        ) {
        case .admit, .finishAlreadyStarted:
            break
        case .deferUntilForeground:
            throw PiqaeNodeError.backgroundExecutionUnavailable
        }

        let receipt: PiqaeJobReceipt
        if let selectedIPC {
            receipt = try await selectedIPC.submit(request)
        } else {
            guard let printer = snapshotValue.printers.first(where: { $0.id == request.printerID }) else {
                throw PiqaeNodeError.printerNotFound(request.printerID)
            }
            guard let adapter = adaptersByID[printer.adapterID] else {
                throw PiqaeNodeError.adapterUnavailable(printer.adapterID)
            }
            try await adapter.validate(request, for: printer)
            receipt = try await adapter.submit(request, to: printer)
        }
        return receipt
    }

    func profiles(for printerID: PiqaePrinterID) async throws -> [PiqaePrintProfile] {
        try requireStarted()
        if let selectedIPC { return try await selectedIPC.profiles(for: printerID) }
        guard let printer = snapshotValue.printers.first(where: { $0.id == printerID }) else {
            throw PiqaeNodeError.printerNotFound(printerID)
        }
        guard let adapter = adaptersByID[printer.adapterID] else {
            throw PiqaeNodeError.adapterUnavailable(printer.adapterID)
        }
        return try await adapter.profiles(for: printer)
    }

    func captureProfile(_ request: PiqaeProfileCaptureRequest) async throws -> PiqaePrintProfile {
        try requireStarted()
        guard selectedIPC == nil else {
            throw PiqaeNodeError.unsupportedOperation(
                "Profile capture must be authorized and presented by the installed node."
            )
        }
        guard let printer = snapshotValue.printers.first(where: { $0.id == request.printerID }) else {
            throw PiqaeNodeError.printerNotFound(request.printerID)
        }
        guard let adapter = adaptersByID[printer.adapterID] else {
            throw PiqaeNodeError.adapterUnavailable(printer.adapterID)
        }
        return try await adapter.captureProfile(request, for: printer)
    }

    func updateExecutionContext(_ context: PiqaeExecutionContext) {
        executionContext = context
        if context.phase == .suspended {
            snapshotValue = replacingSnapshot(phase: .suspended, statusMessage: nil)
        } else if started {
            snapshotValue = replacingSnapshot(phase: .ready, statusMessage: nil)
        }
        emit()
    }

    func handleWakeHint(
        _ hint: PiqaeWakeHint,
        context: PiqaeExecutionContext
    ) async -> PiqaeWakeHintResult {
        guard started else { return .deferred(reason: "The node has not started.") }
        guard context.phase != .suspended else {
            return .deferred(reason: "The host application is suspended.")
        }
        executionContext = context
        do {
            try await refresh()
            return .reconciledWithoutLeasing
        } catch {
            return .deferred(reason: "Reconciliation is temporarily unavailable.")
        }
    }

    private func startEmbedded() async throws {
        let installationID = try await configuration.identityStore.loadOrCreateInstallationID()
        guard await PiqaeProcessRuntimeRegistry.shared.acquire(installationID) else {
            throw PiqaeNodeError.nodeAlreadyRunning
        }
        ownsEmbeddedRuntime = true
        adaptersByID.removeAll(keepingCapacity: true)
        for adapter in configuration.printerAdapters {
            guard adaptersByID[adapter.adapterID] == nil else {
                throw PiqaeNodeError.invalidConfiguration(
                    "Printer adapter IDs must be unique; found \(adapter.adapterID) more than once."
                )
            }
            adaptersByID[adapter.adapterID] = adapter
        }
        let printers = try await discoverEmbeddedPrinters()
        let initialConnections: [PiqaeConnection]
        switch configuration.connectivity {
        case .localOnly: initialConnections = [.localOnly]
        case .cloud: initialConnections = []
        }
        snapshotValue = PiqaeNodeSnapshot(
            installationID: installationID,
            hostMode: .embeddedApplication,
            availability: configuration.availability,
            phase: .starting,
            connections: initialConnections,
            printers: printers,
            lastUpdatedAt: Date()
        )
        emit()
    }

    #if !os(iOS)
    private func startAttached(required: Bool) async throws {
        guard let ipc = configuration.installedNodeIPC else {
            throw PiqaeNodeError.installedNodeUnavailable
        }
        switch await ipc.probe().state {
        case .unavailable:
            if required { throw PiqaeNodeError.installedNodeUnavailable }
            try await startEmbedded()
        case let .available(version):
            guard Self.supports(version) else {
                throw PiqaeNodeError.incompatibleInstalledNode(
                    found: version,
                    supported: PiqaeNodeConfiguration.supportedLocalProtocolVersions
                )
            }
            try await attach(ipc)
        }
    }

    private func attach(_ ipc: any PiqaeInstalledNodeIPC) async throws {
        selectedIPC = ipc
        snapshotValue = normalizedAttachedSnapshot(try await ipc.snapshot())
        emit()
    }
    #endif

    private func discoverEmbeddedPrinters() async throws -> [PiqaePrinter] {
        var result: [PiqaePrinter] = []
        var seen: Set<PiqaePrinterID> = []
        for adapter in configuration.printerAdapters {
            for printer in try await adapter.discoverPrinters() {
                guard printer.adapterID == adapter.adapterID else {
                    throw PiqaeNodeError.invalidConfiguration(
                        "Adapter \(adapter.adapterID) returned a printer owned by \(printer.adapterID)."
                    )
                }
                guard seen.insert(printer.id).inserted else {
                    throw PiqaeNodeError.invalidConfiguration(
                        "Multiple adapters returned printer ID \(printer.id.rawValue)."
                    )
                }
                result.append(printer)
            }
        }
        return result.sorted { $0.displayName.localizedCaseInsensitiveCompare($1.displayName) == .orderedAscending }
    }

    private func requireStarted() throws {
        guard started else { throw PiqaeNodeError.notStarted }
    }

    private func removeObserver(_ id: UUID) {
        observers.removeValue(forKey: id)
    }

    private func emit() {
        for observer in observers.values { observer.yield(snapshotValue) }
    }

    private func setPhase(_ phase: PiqaeNodePhase) {
        snapshotValue = replacingSnapshot(phase: phase, statusMessage: nil)
        emit()
    }

    private func replacingSnapshot(
        phase: PiqaeNodePhase? = nil,
        connections: [PiqaeConnection]? = nil,
        printers: [PiqaePrinter]? = nil,
        statusMessage: String? = nil
    ) -> PiqaeNodeSnapshot {
        PiqaeNodeSnapshot(
            installationID: snapshotValue.installationID,
            hostMode: snapshotValue.hostMode,
            availability: snapshotValue.availability,
            phase: phase ?? snapshotValue.phase,
            connections: connections ?? snapshotValue.connections,
            printers: printers ?? snapshotValue.printers,
            lastUpdatedAt: Date(),
            statusMessage: statusMessage
        )
    }

    private func normalizedAttachedSnapshot(_ remote: PiqaeNodeSnapshot) -> PiqaeNodeSnapshot {
        PiqaeNodeSnapshot(
            installationID: remote.installationID,
            hostMode: .attachedClient,
            availability: remote.availability,
            phase: remote.phase,
            connections: remote.connections,
            printers: remote.printers,
            lastUpdatedAt: remote.lastUpdatedAt,
            statusMessage: remote.statusMessage
        )
    }

    private static func supports(_ version: UInt32) -> Bool {
        PiqaeNodeConfiguration.supportedLocalProtocolVersions.contains(version)
    }

    private static func redactedMessage(for error: Error) -> String {
        if let nodeError = error as? PiqaeNodeError { return nodeError.localizedDescription }
        return "The node could not start."
    }
}
