import CryptoKit
import Foundation

public final class PiqaeNode: @unchecked Sendable {
    private let engine: PiqaeNodeEngine

    public let connections: PiqaeConnectionsService
    public let printers: PiqaePrintersService
    public let jobs: PiqaeJobsService
    public let profiles: PiqaeProfilesService
    public let remoteNotifications: PiqaeRemoteNotificationsService

    public init(_ configuration: PiqaeNodeConfiguration) {
        let engine = PiqaeNodeEngine(configuration: configuration)
        self.engine = engine
        connections = PiqaeConnectionsService(engine: engine)
        printers = PiqaePrintersService(engine: engine)
        jobs = PiqaeJobsService(engine: engine)
        profiles = PiqaeProfilesService(engine: engine)
        remoteNotifications = PiqaeRemoteNotificationsService(engine: engine)
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

    public func reportHostLifecycle(_ event: PiqaeHostLifecycleEvent) async throws {
        try await engine.reportHostLifecycle(event)
    }

    public func handleWakeHint(
        _ hint: PiqaeWakeHint,
        context: PiqaeExecutionContext
    ) async -> PiqaeWakeHintResult {
        await engine.handleWakeHint(hint, context: context)
    }
}

public final class PiqaeRemoteNotificationsService: @unchecked Sendable {
    private let engine: PiqaeNodeEngine
    fileprivate init(engine: PiqaeNodeEngine) { self.engine = engine }

    public let availability: PiqaeRemoteNotificationAvailability =
        .opportunisticWhileInstalled

    public func register(
        deviceToken: Data,
        environment: PiqaeAPNsEnvironment,
        bundleIdentifier: String
    ) async throws {
        try await engine.registerRemoteNotifications(
            deviceToken: deviceToken,
            environment: environment,
            bundleIdentifier: bundleIdentifier
        )
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

    public func disconnect(_ connectionID: PiqaeConnectionID) async throws {
        try await engine.disconnect(connectionID)
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

    public func status(_ jobID: PiqaeJobID) async throws -> PiqaeRuntimeJobSnapshot {
        try await engine.job(jobID)
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

    public func create(_ request: PiqaeRuntimeProfileCreateRequest) async throws
        -> PiqaePrintProfile
    {
        try await engine.createProfile(request)
    }

    public func update(_ request: PiqaeRuntimeProfileUpdateRequest) async throws
        -> PiqaePrintProfile
    {
        try await engine.updateProfile(request)
    }

    public func delete(
        printerID: PiqaePrinterID,
        profileID: PiqaeProfileID,
        expectedRevision: UInt64
    ) async throws {
        try await engine.deleteProfile(
            printerID: printerID,
            profileID: profileID,
            expectedRevision: expectedRevision
        )
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
    private var acquiredInstallationID: PiqaeInstallationID?
    private var embeddedRuntimeStarted = false
    private var executionContext = PiqaeExecutionContext.foreground
    private var selectedIPC: (any PiqaeInstalledNodeIPC)?
    private var adaptersByID: [String: any PiqaePrinterAdapter] = [:]
    private var localPrintersByLogicalID: [PiqaePrinterID: PiqaePrinter] = [:]
    private var executingAdapters: Set<String> = []
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
            if embeddedRuntimeStarted {
                try? await configuration.embeddedRuntime?.stop()
                embeddedRuntimeStarted = false
            }
            if ownsEmbeddedRuntime, let id = acquiredInstallationID {
                await PiqaeProcessRuntimeRegistry.shared.release(id)
                ownsEmbeddedRuntime = false
                acquiredInstallationID = nil
            }
            started = false
            selectedIPC = nil
            adaptersByID.removeAll(keepingCapacity: true)
            localPrintersByLogicalID.removeAll(keepingCapacity: true)
            executingAdapters.removeAll(keepingCapacity: true)
            snapshotValue = replacingSnapshot(
                phase: .degraded,
                statusMessage: Self.redactedMessage(for: error)
            )
            emit()
            throw error
        }
    }

    func stop() async {
        if embeddedRuntimeStarted {
            try? await configuration.embeddedRuntime?.stop()
            embeddedRuntimeStarted = false
        }
        if ownsEmbeddedRuntime, let id = acquiredInstallationID {
            await PiqaeProcessRuntimeRegistry.shared.release(id)
        }
        ownsEmbeddedRuntime = false
        acquiredInstallationID = nil
        selectedIPC = nil
        localPrintersByLogicalID.removeAll(keepingCapacity: true)
        executingAdapters.removeAll(keepingCapacity: true)
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
        let discovered = try await refreshEmbeddedInventory()
        snapshotValue = replacingSnapshot(printers: discovered, statusMessage: nil)
        emit()
        if let runtime = configuration.embeddedRuntime,
            canExecuteDurableHandoff()
        {
            for adapterID in adaptersByID.keys.sorted() {
                try await drainAdapter(adapterID, runtime: runtime)
            }
        }
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

    func disconnect(_ connectionID: PiqaeConnectionID) async throws {
        try requireStarted()
        guard selectedIPC == nil, let runtime = configuration.embeddedRuntime else {
            throw PiqaeNodeError.unsupportedOperation(
                "Disconnect must be authorized by the runtime which owns the connection."
            )
        }
        try await runtime.revokeConnector(id: connectionID)
        snapshotValue = replacingSnapshot(
            connections: snapshotValue.connections.filter { $0.id != connectionID },
            statusMessage: nil
        )
        emit()
    }

    func submit(_ request: PiqaePrintRequest) async throws -> PiqaeJobReceipt {
        try requireStarted()
        if let selectedIPC {
            return try await selectedIPC.submit(request)
        }
        guard let runtime = configuration.embeddedRuntime else {
            throw PiqaeNodeError.unsupportedOperation(
                "Embedded job submission requires the durable native runtime executor"
            )
        }
        guard let printer = snapshotValue.printers.first(where: { $0.id == request.printerID }) else {
            throw PiqaeNodeError.printerNotFound(request.printerID)
        }
        let encoded = try Self.runtimeJobRequest(request, adapterID: printer.adapterID)
        let accepted = try await runtime.enqueue(encoded)
        let jobID = PiqaeJobID(rawValue: accepted.jobID)
        let handoff = PiqaePendingHandoff(
            payloadIsDurable: true,
            estimatedSecondsToNativeAcceptance: 10
        )
        switch admissionPolicy.evaluate(
            handoff,
            context: executionContext,
            availability: snapshotValue.availability
        ) {
        case .admit, .finishAlreadyStarted:
            try await drainAdapter(printer.adapterID, runtime: runtime)
        case .deferUntilForeground:
            return try Self.receipt(jobID: jobID, state: accepted.state, nativeJobID: nil)
        }
        return try Self.receipt(from: await runtime.job(id: jobID))
    }

    func job(_ jobID: PiqaeJobID) async throws -> PiqaeRuntimeJobSnapshot {
        try requireStarted()
        guard selectedIPC == nil, let runtime = configuration.embeddedRuntime else {
            throw PiqaeNodeError.unsupportedOperation(
                "Job status is unavailable from this installed-node protocol."
            )
        }
        return try await runtime.job(id: jobID)
    }

    func profiles(for printerID: PiqaePrinterID) async throws -> [PiqaePrintProfile] {
        try requireStarted()
        if let selectedIPC { return try await selectedIPC.profiles(for: printerID) }
        guard let printer = localPrintersByLogicalID[printerID] else {
            throw PiqaeNodeError.printerNotFound(printerID)
        }
        guard configuration.embeddedRuntime != nil else {
            throw PiqaeNodeError.unsupportedOperation(
                "Profiles require the durable native runtime."
            )
        }
        return try await persistedProfiles(for: printer.id)
    }

    func captureProfile(_ request: PiqaeProfileCaptureRequest) async throws -> PiqaePrintProfile {
        try requireStarted()
        guard selectedIPC == nil else {
            throw PiqaeNodeError.unsupportedOperation(
                "Profile capture must be authorized and presented by the installed node."
            )
        }
        guard let printer = localPrintersByLogicalID[request.printerID] else {
            throw PiqaeNodeError.printerNotFound(request.printerID)
        }
        guard let adapter = adaptersByID[printer.adapterID] else {
            throw PiqaeNodeError.adapterUnavailable(printer.adapterID)
        }
        guard let runtime = configuration.embeddedRuntime else {
            throw PiqaeNodeError.unsupportedOperation("Profiles require the durable native runtime.")
        }
        let captured = try await adapter.captureProfile(request, for: printer)
        return Self.profile(
            try await runtime.createProfile(
                PiqaeRuntimeProfileCreateRequest(
                    printerID: request.printerID,
                    name: captured.name,
                    isDefault: captured.isDefault
                )
            )
        )
    }

    func createProfile(_ request: PiqaeRuntimeProfileCreateRequest) async throws
        -> PiqaePrintProfile
    {
        try requireStarted()
        guard let runtime = configuration.embeddedRuntime else {
            throw PiqaeNodeError.unsupportedOperation("Profiles require the durable native runtime.")
        }
        return Self.profile(try await runtime.createProfile(request))
    }

    func updateProfile(_ request: PiqaeRuntimeProfileUpdateRequest) async throws
        -> PiqaePrintProfile
    {
        try requireStarted()
        guard let runtime = configuration.embeddedRuntime else {
            throw PiqaeNodeError.unsupportedOperation("Profiles require the durable native runtime.")
        }
        return Self.profile(try await runtime.updateProfile(request))
    }

    func deleteProfile(
        printerID: PiqaePrinterID,
        profileID: PiqaeProfileID,
        expectedRevision: UInt64
    ) async throws {
        try requireStarted()
        guard let runtime = configuration.embeddedRuntime else {
            throw PiqaeNodeError.unsupportedOperation("Profiles require the durable native runtime.")
        }
        try await runtime.deleteProfile(
            printerID: printerID,
            profileID: profileID,
            expectedRevision: expectedRevision
        )
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

    func reportHostLifecycle(_ event: PiqaeHostLifecycleEvent) async throws {
        if embeddedRuntimeStarted {
            try await configuration.embeddedRuntime?.report(event)
        }
        try await configuration.hostLifecycleReporter?.report(event)
        switch event {
        case .enteredForeground:
            updateExecutionContext(.foreground)
        case .enteredBackground:
            updateExecutionContext(
                PiqaeExecutionContext(phase: .background, source: .foreground)
            )
        case .suspendImminent, .sleeping, .shutdownRequested:
            updateExecutionContext(
                PiqaeExecutionContext(phase: .suspended, source: .foreground)
            )
        case .started, .woke, .networkAvailable, .networkConstrained, .networkUnavailable:
            break
        }
    }

    func registerRemoteNotifications(
        deviceToken: Data,
        environment: PiqaeAPNsEnvironment,
        bundleIdentifier: String
    ) async throws {
        try requireStarted()
        guard let provider = configuration.remoteNotificationProvider else {
            throw PiqaeNodeError.unsupportedOperation(
                "The host did not configure remote-notification registration."
            )
        }
        guard let installationID = snapshotValue.installationID else {
            throw PiqaeNodeError.notStarted
        }
        try await provider.register(
            PiqaeRemoteNotificationRegistration(
                installationID: installationID,
                token: try PiqaeSensitiveDeviceToken(deviceToken),
                environment: environment,
                bundleIdentifier: bundleIdentifier
            )
        )
    }

    func handleWakeHint(
        _ hint: PiqaeWakeHint,
        context: PiqaeExecutionContext
    ) async -> PiqaeWakeHintResult {
        guard started else { return .deferred(reason: "The node has not started.") }
        guard !Task.isCancelled else {
            return .deferred(reason: "The host execution budget expired.")
        }
        guard context.phase != .suspended else {
            return .deferred(reason: "The host application is suspended.")
        }
        executionContext = context
        do {
            try await refresh()
            guard !Task.isCancelled else {
                return .deferred(reason: "The host execution budget expired.")
            }
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
        acquiredInstallationID = installationID
        if let runtime = configuration.embeddedRuntime {
            try await runtime.start()
            embeddedRuntimeStarted = true
        }
        adaptersByID.removeAll(keepingCapacity: true)
        for adapter in configuration.printerAdapters {
            guard adaptersByID[adapter.adapterID] == nil else {
                throw PiqaeNodeError.invalidConfiguration(
                    "Printer adapter IDs must be unique; found \(adapter.adapterID) more than once."
                )
            }
            adaptersByID[adapter.adapterID] = adapter
        }
        let printers = try await refreshEmbeddedInventory()
        if let runtime = configuration.embeddedRuntime {
            for adapterID in adaptersByID.keys.sorted() {
                try await drainAdapter(adapterID, runtime: runtime)
            }
        }
        let initialConnections: [PiqaeConnection]
        switch configuration.connectivity {
        case .localOnly:
            if let runtime = configuration.embeddedRuntime {
                let persisted = try await runtime.connectors().map(Self.connection)
                initialConnections = persisted.isEmpty ? [.localOnly] : persisted
            } else {
                initialConnections = [.localOnly]
            }
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

    private func refreshEmbeddedInventory() async throws -> [PiqaePrinter] {
        let discovered = try await discoverEmbeddedPrinters()
        guard let runtime = configuration.embeddedRuntime else {
            localPrintersByLogicalID = Dictionary(uniqueKeysWithValues: discovered.map { ($0.id, $0) })
            return discovered
        }
        var projected: [PiqaePrinter] = []
        localPrintersByLogicalID.removeAll(keepingCapacity: true)
        for adapter in configuration.printerAdapters {
            guard adapter.runtimeFingerprint.adapterID == adapter.adapterID else {
                throw PiqaeNodeError.invalidConfiguration(
                    "Adapter runtime fingerprint must use the adapter's stable ID."
                )
            }
            try await runtime.registerAdapter(
                PiqaeRuntimeAdapterRegistration(
                    fingerprint: adapter.runtimeFingerprint,
                    capabilityContract: .init(descriptor: adapter.descriptor)
                )
            )
            let local = discovered.filter { $0.adapterID == adapter.adapterID }
            let observations = local.map {
                PiqaeRuntimePrinterObservation(
                    nativeID: $0.nativeID,
                    name: $0.displayName,
                    state: $0.state.rawValue
                )
            }
            let snapshots = try await runtime.observePrinterInventory(
                adapterID: adapter.adapterID,
                printers: observations
            )
            let localByNativeID = Dictionary(uniqueKeysWithValues: local.map { ($0.nativeID, $0) })
            for runtimePrinter in snapshots {
                guard let source = localByNativeID[runtimePrinter.nativeID] else { continue }
                let logicalID = PiqaePrinterID(rawValue: runtimePrinter.printerID)
                let printer = PiqaePrinter(
                    id: logicalID,
                    adapterID: source.adapterID,
                    adapterFingerprint: source.adapterFingerprint,
                    nativeID: source.nativeID,
                    displayName: runtimePrinter.name,
                    model: source.model,
                    location: source.location,
                    state: PiqaePrinterState(rawValue: runtimePrinter.state) ?? .unknown,
                    capabilities: source.capabilities,
                    queue: source.queue,
                    loadedMedia: source.loadedMedia,
                    alerts: source.alerts,
                    observedAt: Date(
                        timeIntervalSince1970:
                            TimeInterval(runtimePrinter.observedUnixMilliseconds) / 1_000
                    ),
                    freshUntil: source.freshUntil
                )
                projected.append(printer)
                localPrintersByLogicalID[logicalID] = printer
            }
        }
        return projected.sorted {
            $0.displayName.localizedCaseInsensitiveCompare($1.displayName) == .orderedAscending
        }
    }

    private func drainAdapter(
        _ adapterID: String,
        runtime: any PiqaeEmbeddedNodeRuntime
    ) async throws {
        guard executingAdapters.insert(adapterID).inserted else { return }
        do {
            for _ in 0..<32 {
                guard let operation = try await runtime.nextOperation(adapterID: adapterID) else {
                    break
                }
                switch operation.phase {
                case .accepted:
                    try await reconcileAccepted(operation, runtime: runtime)
                    executingAdapters.remove(adapterID)
                    return
                case .handoffStarted:
                    _ = try await runtime.complete(
                        operation,
                        outcome: .ambiguous(code: "recovered_after_handoff")
                    )
                case .claimed:
                    try await executeClaimed(operation, runtime: runtime)
                }
            }
            executingAdapters.remove(adapterID)
        } catch {
            executingAdapters.remove(adapterID)
            throw error
        }
    }

    private func canExecuteDurableHandoff() -> Bool {
        switch admissionPolicy.evaluate(
            PiqaePendingHandoff(
                payloadIsDurable: true,
                estimatedSecondsToNativeAcceptance: 10
            ),
            context: executionContext,
            availability: snapshotValue.availability
        ) {
        case .admit, .finishAlreadyStarted: true
        case .deferUntilForeground: false
        }
    }

    private func reconcileAccepted(
        _ operation: PiqaeRuntimeAdapterOperation,
        runtime: any PiqaeEmbeddedNodeRuntime
    ) async throws {
        guard let nativeJobID = operation.nativeJobID,
            let adapter = adaptersByID[operation.adapterID],
            let printer = localPrintersByLogicalID[PiqaePrinterID(rawValue: operation.printerID)]
        else { return }
        switch try await adapter.observeNativeJob(nativeJobID: nativeJobID, printer: printer) {
        case .accepted, .printing, .unknown:
            return
        case .completedReported:
            _ = try await runtime.complete(
                operation,
                outcome: .completedReported(nativeJobID: nativeJobID)
            )
        case let .failedTerminal(code):
            _ = try await runtime.complete(
                operation,
                outcome: .failedTerminal(nativeJobID: nativeJobID, code: code)
            )
        }
    }

    private func executeClaimed(
        _ operation: PiqaeRuntimeAdapterOperation,
        runtime: any PiqaeEmbeddedNodeRuntime
    ) async throws {
        guard let adapter = adaptersByID[operation.adapterID],
            let printer = localPrintersByLogicalID[PiqaePrinterID(rawValue: operation.printerID)]
        else {
            _ = try await runtime.complete(
                operation,
                outcome: .rejectedBeforeHandoff(code: "adapter_or_printer_unavailable", retryable: true)
            )
            return
        }
        let request: PiqaePrintRequest
        do {
            request = try Self.request(for: operation)
            try await adapter.validate(request, for: printer)
        } catch {
            _ = try await runtime.complete(
                operation,
                outcome: .rejectedBeforeHandoff(code: "adapter_validation_failed", retryable: false)
            )
            return
        }
        let started: PiqaeRuntimeAdapterOperation
        do {
            started = try await runtime.beginHandoff(operation)
        } catch {
            _ = try await runtime.complete(
                operation,
                outcome: .rejectedBeforeHandoff(code: "handoff_deadline_elapsed", retryable: true)
            )
            return
        }
        do {
            let receipt = try await adapter.submit(request, to: printer)
            if receipt.handoffState == .acceptedBySpooler,
                let nativeJobID = receipt.nativeJobID,
                !nativeJobID.isEmpty
            {
                _ = try await runtime.complete(started, outcome: .accepted(nativeJobID: nativeJobID))
            } else {
                _ = try await runtime.complete(
                    started,
                    outcome: .ambiguous(code: "native_acceptance_unverifiable")
                )
            }
        } catch {
            _ = try await runtime.complete(
                started,
                outcome: .ambiguous(code: "native_handoff_error")
            )
        }
    }

    private func persistedProfiles(for printerID: PiqaePrinterID) async throws
        -> [PiqaePrintProfile]
    {
        guard let runtime = configuration.embeddedRuntime else { return [] }
        return try await runtime.profiles(printerID: printerID).map(Self.profile)
    }

    private static func runtimeJobRequest(
        _ request: PiqaePrintRequest,
        adapterID: String
    ) throws -> PiqaeRuntimeJobRequest {
        let content: Data
        let kind: String
        switch request.content {
        case let .pdf(data):
            content = data
            kind = "pdf"
        case let .image(data, typeIdentifier):
            content = data
            kind = "image.\(typeIdentifier)"
        case let .raw(data, mediaType):
            content = data
            kind = "raw.\(mediaType)"
        }
        let options = RuntimePrintOptions(intent: request.intent, profileID: request.profileID?.rawValue)
        let optionsData = try JSONEncoder().encode(options)
        guard let optionsJSON = String(data: optionsData, encoding: .utf8) else {
            throw PiqaeNodeError.invalidConfiguration("Print options could not be encoded.")
        }
        return PiqaeRuntimeJobRequest(
            adapterID: adapterID,
            idempotencyKey: request.idempotencyKey,
            printerID: request.printerID,
            title: request.title,
            contentKind: kind,
            content: content,
            optionsJSON: optionsJSON
        )
    }

    private static func request(for operation: PiqaeRuntimeAdapterOperation) throws
        -> PiqaePrintRequest
    {
        let file = try FileHandle(forReadingFrom: URL(fileURLWithPath: operation.contentPath))
        defer { try? file.close() }
        let maximum = 16 * 1024 * 1024
        guard let data = try file.read(upToCount: maximum + 1), !data.isEmpty,
            data.count <= maximum
        else {
            throw PiqaeNodeError.submissionRejected("Durable content is unavailable or unbounded.")
        }
        let digest = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
        guard digest == operation.contentSHA256 else {
            throw PiqaeNodeError.submissionRejected("Durable content integrity check failed.")
        }
        let options = try JSONDecoder().decode(
            RuntimePrintOptions.self,
            from: Data(operation.optionsJSON.utf8)
        )
        let content: PiqaePrintContent
        if operation.contentKind == "pdf" {
            content = .pdf(data)
        } else if operation.contentKind.hasPrefix("image.") {
            content = .image(data, typeIdentifier: String(operation.contentKind.dropFirst(6)))
        } else if operation.contentKind.hasPrefix("raw.") {
            content = .raw(data, mediaType: String(operation.contentKind.dropFirst(4)))
        } else {
            throw PiqaeNodeError.submissionRejected("Durable content kind is unsupported.")
        }
        return try PiqaePrintRequest(
            printerID: PiqaePrinterID(rawValue: operation.printerID),
            title: operation.title,
            content: content,
            intent: options.intent,
            profileID: options.profileID.map(PiqaeProfileID.init(rawValue:)),
            idempotencyKey: operation.idempotencyKey
        )
    }

    private static func receipt(from job: PiqaeRuntimeJobSnapshot) throws -> PiqaeJobReceipt {
        try receipt(
            jobID: PiqaeJobID(rawValue: job.jobID),
            state: job.state,
            nativeJobID: job.nativeJobID
        )
    }

    private static func receipt(
        jobID: PiqaeJobID,
        state: String,
        nativeJobID: String?
    ) throws -> PiqaeJobReceipt {
        let handoff: PiqaeNativeHandoffState
        switch state {
        case "accepted_by_spooler", "completed_reported": handoff = .acceptedBySpooler
        case "delivery_uncertain": handoff = .deliveryUncertain
        case "queued", "pending", "spool_intent", "failed_retryable": handoff = .queuedLocally
        case "failed_terminal":
            throw PiqaeNodeError.submissionRejected("The durable runtime rejected the print job.")
        default: handoff = .queuedLocally
        }
        return PiqaeJobReceipt(
            jobID: jobID,
            nativeJobID: nativeJobID,
            handoffState: handoff,
            acceptedAt: Date()
        )
    }

    private static func profile(_ snapshot: PiqaeRuntimeProfileSnapshot) -> PiqaePrintProfile {
        PiqaePrintProfile(
            id: .init(rawValue: snapshot.profileID),
            printerID: .init(rawValue: snapshot.printerID),
            name: snapshot.name,
            revision: snapshot.revision,
            isDefault: snapshot.isDefault
        )
    }

    private static func connection(_ snapshot: PiqaeRuntimeConnectorSnapshot) -> PiqaeConnection {
        PiqaeConnection(
            id: .init(rawValue: snapshot.connectorID),
            authorityURL: snapshot.controlPlaneURL,
            workspaceName: snapshot.workspaceName ?? snapshot.displayName,
            state: snapshot.enabled ? .connected : .offline
        )
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

private struct RuntimePrintOptions: Codable {
    let intent: PiqaePortablePrintIntent
    let profileID: String?
    enum CodingKeys: String, CodingKey { case intent; case profileID = "profile_id" }
}
