import CryptoKit
import Foundation

public final class PiqaeNode: @unchecked Sendable {
    private let engine: PiqaeNodeEngine
    private let executionFence: PiqaeExecutionFence

    public let connections: PiqaeConnectionsService
    public let printers: PiqaePrintersService
    public let jobs: PiqaeJobsService
    public let profiles: PiqaeProfilesService
    public let remoteNotifications: PiqaeRemoteNotificationsService

    public init(_ configuration: PiqaeNodeConfiguration) {
        let executionFence = PiqaeExecutionFence()
        let engine = PiqaeNodeEngine(configuration: configuration, executionFence: executionFence)
        self.engine = engine
        self.executionFence = executionFence
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

    /// Closes handoff admission synchronously from an OS expiration callback.
    /// Actor cleanup follows asynchronously, but no later generation can begin
    /// a native handoff after this returns.
    func expireExecutionSynchronously() {
        executionFence.suspend()
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

    public func history(offset: Int = 0, limit: Int = 50) async throws -> PiqaeJobHistoryPage {
        try await engine.jobHistory(offset: offset, limit: limit)
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

fileprivate final class PiqaeExecutionFence: @unchecked Sendable {
    private let lock = NSLock()
    private var generation: UInt64 = 1
    private var open = true

    @discardableResult
    func resume() -> UInt64 {
        lock.withLock {
            if !open {
                generation &+= 1
                open = true
            }
            return generation
        }
    }

    @discardableResult
    func suspend() -> UInt64 {
        lock.withLock {
            if open {
                generation &+= 1
                open = false
            }
            return generation
        }
    }

    func permits(_ expectedGeneration: UInt64) -> Bool {
        lock.withLock { open && generation == expectedGeneration }
    }

    func currentGeneration() -> UInt64 {
        lock.withLock { generation }
    }
}

private enum PiqaeWakeReconciliationError: Error {
    case failed(retryable: Bool)

    var retryable: Bool {
        switch self {
        case let .failed(retryable): retryable
        }
    }
}

/// Lets one caller detach from a coalesced wake pass without cancelling the
/// shared work still awaited by another background task or foreground caller.
private final class PiqaeWakeTaskWaiter: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<PiqaeWakeHintResult, Never>?
    private var resolved: PiqaeWakeHintResult?

    func wait(for task: Task<PiqaeWakeHintResult, Never>) async -> PiqaeWakeHintResult {
        await withTaskCancellationHandler {
            await withCheckedContinuation { continuation in
                let immediate = lock.withLock { () -> PiqaeWakeHintResult? in
                    if let resolved { return resolved }
                    self.continuation = continuation
                    return nil
                }
                if let immediate { continuation.resume(returning: immediate) }
                Task { [weak self] in
                    self?.resolve(await task.value)
                }
            }
        } onCancel: {
            resolve(.deferred(reason: "The host execution budget expired."))
        }
    }

    private func resolve(_ result: PiqaeWakeHintResult) {
        let pending = lock.withLock { () -> CheckedContinuation<PiqaeWakeHintResult, Never>? in
            guard resolved == nil else { return nil }
            resolved = result
            defer { continuation = nil }
            return continuation
        }
        pending?.resume(returning: result)
    }
}

actor PiqaeNodeEngine {
    private let configuration: PiqaeNodeConfiguration
    private let executionFence: PiqaeExecutionFence
    private let admissionPolicy = PiqaeBackgroundAdmissionPolicy()
    private var started = false
    private var ownsEmbeddedRuntime = false
    private var acquiredInstallationID: PiqaeInstallationID?
    private var embeddedRuntimeStarted = false
    private var executionContext = PiqaeExecutionContext.foreground
    private var executionDeadline: Date?
    private var selectedIPC: (any PiqaeInstalledNodeIPC)?
    private var adaptersByID: [String: any PiqaePrinterAdapter] = [:]
    private var localPrintersByLogicalID: [PiqaePrinterID: PiqaePrinter] = [:]
    private var executingAdapters: Set<String> = []
    private var automaticDrainRequested = false
    private var automaticDrainTask: Task<Void, Never>?
    private var nativeObservationRequested = false
    private var nativeObservationTask: Task<Void, Never>?
    private struct WakeTask {
        let token: UUID
        let task: Task<PiqaeWakeHintResult, Never>
    }
    private var wakeTasks: [String: WakeTask] = [:]
    private var observers: [UUID: AsyncStream<PiqaeNodeSnapshot>.Continuation] = [:]
    private var snapshotValue: PiqaeNodeSnapshot

    fileprivate init(
        configuration: PiqaeNodeConfiguration,
        executionFence: PiqaeExecutionFence
    ) {
        self.configuration = configuration
        self.executionFence = executionFence
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
        executionFence.resume()

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
                        guard configuration.allowsEmbeddedFallback else {
                            throw PiqaeNodeError.installedNodeUnavailable
                        }
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
                    guard configuration.allowsEmbeddedFallback else {
                        throw PiqaeNodeError.installedNodeUnavailable
                    }
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
            startAutomaticDrainIfPossible()
            requestNativeObservation()
        } catch {
            started = false
            await cancelRuntimeWork()
            if embeddedRuntimeStarted {
                try? await configuration.embeddedRuntime?.stop()
                embeddedRuntimeStarted = false
            }
            if ownsEmbeddedRuntime, let id = acquiredInstallationID {
                await PiqaeProcessRuntimeRegistry.shared.release(id)
                ownsEmbeddedRuntime = false
                acquiredInstallationID = nil
            }
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
        started = false
        await cancelRuntimeWork()
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
                    version: "local-protocol-4",
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
                _ = try await drainAdapter(adapterID, runtime: runtime)
            }
            requestNativeObservation()
        }
    }

    func connect(_ cloud: PiqaeCloudConfiguration) async throws -> PiqaeConnection {
        try requireStarted()
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
            guard let runtime = configuration.embeddedRuntime else {
                throw PiqaeNodeError.unsupportedOperation(
                    "Cloud invitations require the shared durable native runtime."
                )
            }
            connection = Self.connection(try await runtime.connectInvitation(request))
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
            context: effectiveExecutionContext,
            availability: snapshotValue.availability
        ) {
        case .admit, .finishAlreadyStarted:
            _ = try await drainAdapter(printer.adapterID, runtime: runtime)
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

    func jobHistory(offset: Int, limit: Int) async throws -> PiqaeJobHistoryPage {
        try requireStarted()
        if let selectedIPC { return try await selectedIPC.jobHistory(offset: offset, limit: limit) }
        throw PiqaeNodeError.unsupportedOperation(
            "Embedded print history pagination is not exposed by this runtime contract."
        )
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
        guard selectedIPC == nil else {
            throw PiqaeNodeError.unsupportedOperation(
                "Profile changes must be made in the installed Piqae node."
            )
        }
        guard let runtime = configuration.embeddedRuntime else {
            throw PiqaeNodeError.unsupportedOperation("Profiles require the durable native runtime.")
        }
        return Self.profile(try await runtime.createProfile(request))
    }

    func updateProfile(_ request: PiqaeRuntimeProfileUpdateRequest) async throws
        -> PiqaePrintProfile
    {
        try requireStarted()
        guard selectedIPC == nil else {
            throw PiqaeNodeError.unsupportedOperation(
                "Profile changes must be made in the installed Piqae node."
            )
        }
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
        guard selectedIPC == nil else {
            throw PiqaeNodeError.unsupportedOperation(
                "Profile changes must be made in the installed Piqae node."
            )
        }
        guard let runtime = configuration.embeddedRuntime else {
            throw PiqaeNodeError.unsupportedOperation("Profiles require the durable native runtime.")
        }
        try await runtime.deleteProfile(
            printerID: printerID,
            profileID: profileID,
            expectedRevision: expectedRevision
        )
    }

    func updateExecutionContext(_ context: PiqaeExecutionContext) async {
        executionContext = context
        if context.phase == .suspended {
            executionFence.suspend()
        } else {
            executionFence.resume()
        }
        if context.phase == .background, let remaining = context.remainingSeconds {
            executionDeadline = Date().addingTimeInterval(max(0, remaining))
        } else {
            executionDeadline = nil
        }
        if context.phase == .suspended {
            snapshotValue = replacingSnapshot(phase: .suspended, statusMessage: nil)
        } else if started {
            snapshotValue = replacingSnapshot(phase: .ready, statusMessage: nil)
        }
        emit()
        if context.phase == .suspended {
            await cancelRuntimeWork(closeFence: false)
        } else if !canObserveNativeStatus(), let task = nativeObservationTask {
            nativeObservationTask = nil
            task.cancel()
            await task.value
        }
        if context.phase != .suspended {
            requestAutomaticDrain()
            requestNativeObservation()
        }
    }

    func reportHostLifecycle(_ event: PiqaeHostLifecycleEvent) async throws {
        if embeddedRuntimeStarted {
            try await configuration.embeddedRuntime?.report(event)
        }
        try await configuration.hostLifecycleReporter?.report(event)
        switch event {
        case .enteredForeground:
            await updateExecutionContext(.foreground)
        case .enteredBackground:
            await updateExecutionContext(
                PiqaeExecutionContext(phase: .background, source: .foreground)
            )
        case .suspendImminent, .sleeping, .shutdownRequested:
            await updateExecutionContext(
                PiqaeExecutionContext(phase: .suspended, source: .foreground)
            )
        case .woke:
            await updateExecutionContext(.foreground)
        case .networkAvailable:
            requestAutomaticDrain()
            requestNativeObservation()
        case .started, .networkConstrained, .networkUnavailable:
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
        if let existing = wakeTasks[hint.collapseID] {
            return await PiqaeWakeTaskWaiter().wait(for: existing.task)
        }
        let token = UUID()
        let task = Task<PiqaeWakeHintResult, Never> { [weak self] in
            guard let self else {
                return PiqaeWakeHintResult.deferred(reason: "The node stopped.")
            }
            let result = await self.performWakeHint(context: context)
            await self.removeWakeTask(collapseID: hint.collapseID, token: token)
            return result
        }
        wakeTasks[hint.collapseID] = WakeTask(token: token, task: task)
        return await PiqaeWakeTaskWaiter().wait(for: task)
    }

    private func removeWakeTask(collapseID: String, token: UUID) {
        if wakeTasks[collapseID]?.token == token {
            wakeTasks.removeValue(forKey: collapseID)
        }
    }

    private func performWakeHint(context: PiqaeExecutionContext) async -> PiqaeWakeHintResult {
        await updateExecutionContext(context)
        let generation = executionFence.currentGeneration()
        let policy = configuration.wakeRetryPolicy
        var attempt = 0
        while attempt < policy.maximumAttempts {
            attempt += 1
            do {
                guard executionFence.permits(generation) else {
                    return .deferred(reason: "The host execution budget expired.")
                }
                if selectedIPC == nil, let runtime = configuration.embeddedRuntime {
                    let available = effectiveExecutionContext.remainingSeconds.map {
                        max(0, $0 - policy.executionSafetyMarginSeconds)
                    }
                    let timeout = min(
                        policy.cloudCycleTimeoutSeconds,
                        available ?? policy.cloudCycleTimeoutSeconds
                    )
                    guard timeout > 0 else {
                        return .deferred(reason: "The host execution budget expired.")
                    }
                    let milliseconds = UInt64(
                        min(timeout * 1_000, Double(UInt64.max))
                    )
                    let outcome = try await runtime.reconcileCloudOutcome(
                        timeoutMilliseconds: milliseconds
                    )
                    guard outcome.loopCompleted, outcome.failedCount == 0 else {
                        throw PiqaeWakeReconciliationError.failed(
                            retryable: outcome.retryable
                        )
                    }
                }
                guard executionFence.permits(generation) else {
                    return .deferred(reason: "The host execution budget expired.")
                }
                try await refresh()
                guard !Task.isCancelled, executionFence.permits(generation) else {
                    return .deferred(reason: "The host execution budget expired.")
                }
                return .reconciled
            } catch is CancellationError {
                return .deferred(reason: "The host execution budget expired.")
            } catch let error as PiqaeWakeReconciliationError {
                guard error.retryable else {
                    return .deferred(reason: "Reconciliation requires operator attention.")
                }
                let delay = policy.delay(
                    after: attempt,
                    remainingSeconds: effectiveExecutionContext.remainingSeconds
                )
                guard let delay, delay > 0 else { break }
                do {
                    try await Task.sleep(
                        nanoseconds: UInt64(min(delay * 1_000_000_000, Double(UInt64.max)))
                    )
                } catch {
                    return .deferred(reason: "The host execution budget expired.")
                }
            } catch {
                // Only a generation outcome can safely classify a connector
                // failure as transient. Unknown runtime/ABI errors fail closed
                // instead of consuming the execution budget with blind retries.
                return .deferred(reason: "Reconciliation is temporarily unavailable.")
            }
        }
        return .deferred(reason: "Reconciliation is temporarily unavailable.")
    }

    private func startEmbedded() async throws {
        let installationID = try await configuration.identityStore.loadOrCreateInstallationID()
        guard await PiqaeProcessRuntimeRegistry.shared.acquire(installationID) else {
            throw PiqaeNodeError.nodeAlreadyRunning
        }
        ownsEmbeddedRuntime = true
        acquiredInstallationID = installationID
        if let runtime = configuration.embeddedRuntime {
            try await runtime.setWorkAvailableHandler { [weak self] in
                Task { await self?.workBecameAvailable() }
            }
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
                _ = try await drainAdapter(adapterID, runtime: runtime)
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
        try await ipc.prepareForAttachment()
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
        var nextLocalPrintersByLogicalID: [PiqaePrinterID: PiqaePrinter] = [:]
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
                nextLocalPrintersByLogicalID[logicalID] = printer
            }
        }
        localPrintersByLogicalID = nextLocalPrintersByLogicalID
        return projected.sorted {
            $0.displayName.localizedCaseInsensitiveCompare($1.displayName) == .orderedAscending
        }
    }

    private enum AdapterDrainResult: Equatable {
        case empty
        case blockedOnNativeObservation
        case deferred
        case alreadyExecuting
        case capped
    }

    private enum NativeObservationReconciliation: Equatable {
        case pending
        case terminal
        case unavailable
    }

    private func drainAdapter(
        _ adapterID: String,
        runtime: any PiqaeEmbeddedNodeRuntime
    ) async throws -> AdapterDrainResult {
        let generation = executionFence.currentGeneration()
        guard executionFence.permits(generation) else { return .deferred }
        guard executingAdapters.insert(adapterID).inserted else { return .alreadyExecuting }
        defer { executingAdapters.remove(adapterID) }
        for _ in 0..<32 {
            try Task.checkCancellation()
            guard executionFence.permits(generation), canExecuteDurableHandoff() else {
                return .deferred
            }
            guard let operation = try await runtime.nextOperation(adapterID: adapterID) else {
                return .empty
            }
            guard !Task.isCancelled, executionFence.permits(generation),
                canExecuteDurableHandoff()
            else {
                _ = try await runtime.complete(
                    operation,
                    outcome: .rejectedBeforeHandoff(
                        code: "host_execution_ended",
                        retryable: true
                    )
                )
                return .deferred
            }
            switch operation.phase {
            case .accepted:
                switch try await reconcileAccepted(operation, runtime: runtime) {
                case .pending:
                    requestNativeObservation()
                    return .blockedOnNativeObservation
                case .unavailable:
                    return .blockedOnNativeObservation
                case .terminal:
                    break
                }
            case .handoffStarted:
                _ = try await runtime.complete(
                    operation,
                    outcome: .ambiguous(code: "recovered_after_handoff")
                )
            case .claimed:
                try await executeClaimed(operation, runtime: runtime, generation: generation)
            }
        }
        return .capped
    }

    private func canExecuteDurableHandoff() -> Bool {
        switch admissionPolicy.evaluate(
            PiqaePendingHandoff(
                payloadIsDurable: true,
                estimatedSecondsToNativeAcceptance: 10
            ),
            context: effectiveExecutionContext,
            availability: snapshotValue.availability
        ) {
        case .admit, .finishAlreadyStarted: true
        case .deferUntilForeground: false
        }
    }

    private var effectiveExecutionContext: PiqaeExecutionContext {
        guard executionContext.phase == .background, let executionDeadline else {
            return executionContext
        }
        return PiqaeExecutionContext(
            phase: .background,
            source: executionContext.source,
            remainingSeconds: max(0, executionDeadline.timeIntervalSinceNow)
        )
    }

    private func workBecameAvailable() {
        guard ownsEmbeddedRuntime else { return }
        automaticDrainRequested = true
        startAutomaticDrainIfPossible()
    }

    private func requestAutomaticDrain() {
        guard ownsEmbeddedRuntime else { return }
        automaticDrainRequested = true
        startAutomaticDrainIfPossible()
    }

    private func requestNativeObservation() {
        guard ownsEmbeddedRuntime else { return }
        nativeObservationRequested = true
        startNativeObservationIfPossible()
    }

    private func startAutomaticDrainIfPossible() {
        guard started, embeddedRuntimeStarted, selectedIPC == nil,
            automaticDrainRequested, automaticDrainTask == nil,
            canExecuteDurableHandoff()
        else { return }
        let generation = executionFence.currentGeneration()
        automaticDrainTask = Task { [weak self] in
            await self?.runAutomaticDrain(generation: generation)
        }
    }

    private func runAutomaticDrain(generation: UInt64) async {
        while !Task.isCancelled,
            executionFence.permits(generation),
            started,
            automaticDrainRequested,
            canExecuteDurableHandoff(),
            let runtime = configuration.embeddedRuntime
        {
            automaticDrainRequested = false
            do {
                let printers = try await refreshEmbeddedInventory()
                snapshotValue = replacingSnapshot(printers: printers, statusMessage: nil)
                emit()
                var capped = false
                for adapterID in adaptersByID.keys.sorted() {
                    let result = try await drainAdapter(adapterID, runtime: runtime)
                    capped = capped || result == .capped
                }
                if capped {
                    automaticDrainRequested = true
                    await Task.yield()
                }
            } catch is CancellationError {
                break
            } catch {
                break
            }
        }
        guard executionFence.permits(generation) else { return }
        automaticDrainTask = nil
        startAutomaticDrainIfPossible()
    }

    private func startNativeObservationIfPossible() {
        guard started, embeddedRuntimeStarted, selectedIPC == nil,
            nativeObservationRequested, nativeObservationTask == nil,
            canObserveNativeStatus()
        else { return }
        let generation = executionFence.currentGeneration()
        nativeObservationTask = Task { [weak self] in
            await self?.runNativeObservation(generation: generation)
        }
    }

    private func runNativeObservation(generation: UInt64) async {
        var delayNanoseconds: UInt64 = 50_000_000
        while !Task.isCancelled,
            executionFence.permits(generation),
            started,
            nativeObservationRequested,
            canObserveNativeStatus(),
            let runtime = configuration.embeddedRuntime
        {
            nativeObservationRequested = false
            var pending = false
            do {
                observationAdapters: for adapterID in adaptersByID.keys.sorted() {
                    for operation in try await runtime.nativeObservations(adapterID: adapterID) {
                        try Task.checkCancellation()
                        guard canObserveNativeStatus() else {
                            pending = true
                            break observationAdapters
                        }
                        if try await reconcileAccepted(operation, runtime: runtime) == .pending {
                            pending = true
                        }
                    }
                }
                if pending {
                    nativeObservationRequested = true
                    guard try await sleepBeforeNativeObservationRetry(delayNanoseconds) else {
                        break
                    }
                    delayNanoseconds = min(delayNanoseconds * 2, 1_000_000_000)
                }
            } catch is CancellationError {
                break
            } catch {
                // Native status APIs and runtime reads can fail transiently.
                // Retain the observation request and retry with a capped delay
                // while the host still has an explicit execution budget.
                nativeObservationRequested = true
                do {
                    guard try await sleepBeforeNativeObservationRetry(delayNanoseconds) else {
                        break
                    }
                } catch {
                    break
                }
                delayNanoseconds = min(delayNanoseconds * 2, 1_000_000_000)
            }
        }
        guard executionFence.permits(generation) else { return }
        nativeObservationTask = nil
        startNativeObservationIfPossible()
    }

    private func canObserveNativeStatus() -> Bool {
        switch effectiveExecutionContext.phase {
        case .foreground:
            return true
        case .suspended:
            return false
        case .background:
            guard snapshotValue.availability != .foregroundOnly,
                let remaining = effectiveExecutionContext.remainingSeconds
            else { return false }
            return remaining > 0.25
        }
    }

    private func sleepBeforeNativeObservationRetry(_ requestedNanoseconds: UInt64) async throws
        -> Bool
    {
        let delayNanoseconds: UInt64
        switch effectiveExecutionContext.phase {
        case .foreground:
            delayNanoseconds = requestedNanoseconds
        case .suspended:
            return false
        case .background:
            guard snapshotValue.availability != .foregroundOnly,
                let remaining = effectiveExecutionContext.remainingSeconds
            else { return false }
            let retryBudget = max(0, remaining - 0.25)
            guard retryBudget > 0 else { return false }
            let budgetNanoseconds = UInt64(
                min(retryBudget * 1_000_000_000, Double(UInt64.max))
            )
            delayNanoseconds = min(requestedNanoseconds, budgetNanoseconds)
        }
        try await Task.sleep(nanoseconds: delayNanoseconds)
        return canObserveNativeStatus()
    }

    private func cancelRuntimeWork(closeFence: Bool = true) async {
        if closeFence { executionFence.suspend() }
        automaticDrainRequested = false
        nativeObservationRequested = false
        let pendingWakeTasks = wakeTasks.values.map(\.task)
        wakeTasks.removeAll(keepingCapacity: true)
        let drainTask = automaticDrainTask
        let observationTask = nativeObservationTask
        automaticDrainTask = nil
        nativeObservationTask = nil
        drainTask?.cancel()
        observationTask?.cancel()
        for task in pendingWakeTasks { task.cancel() }
        await drainTask?.value
        await observationTask?.value
        for task in pendingWakeTasks { _ = await task.value }
    }

    private func reconcileAccepted(
        _ operation: PiqaeRuntimeAdapterOperation,
        runtime: any PiqaeEmbeddedNodeRuntime
    ) async throws -> NativeObservationReconciliation {
        guard let nativeJobID = operation.nativeJobID,
            let adapter = adaptersByID[operation.adapterID],
            let printer = localPrintersByLogicalID[PiqaePrinterID(rawValue: operation.printerID)]
        else { return .unavailable }
        switch try await adapter.observeNativeJob(nativeJobID: nativeJobID, printer: printer) {
        case .accepted, .printing, .unknown:
            return .pending
        case .completedReported:
            _ = try await runtime.complete(
                operation,
                outcome: .completedReported(nativeJobID: nativeJobID)
            )
            requestAutomaticDrain()
            return .terminal
        case let .failedTerminal(code):
            _ = try await runtime.complete(
                operation,
                outcome: .failedTerminal(nativeJobID: nativeJobID, code: code)
            )
            requestAutomaticDrain()
            return .terminal
        }
    }

    private func executeClaimed(
        _ operation: PiqaeRuntimeAdapterOperation,
        runtime: any PiqaeEmbeddedNodeRuntime,
        generation: UInt64
    ) async throws {
        guard executionFence.permits(generation) else {
            _ = try await runtime.complete(
                operation,
                outcome: .rejectedBeforeHandoff(code: "host_execution_ended", retryable: true)
            )
            return
        }
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
        guard executionFence.permits(generation), !Task.isCancelled else {
            // The durable handoff intent exists but the native API was not
            // called. Preserve the ambiguity boundary rather than making a
            // later generation guess whether the adapter saw the request.
            _ = try await runtime.complete(
                started,
                outcome: .ambiguous(code: "handoff_execution_expired")
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
                do {
                    let accepted = try await runtime.nativeObservations(
                        adapterID: operation.adapterID
                    ).first { $0.operationID == operation.operationID }
                    if let accepted {
                        if try await reconcileAccepted(accepted, runtime: runtime) == .pending {
                            requestNativeObservation()
                        }
                    } else {
                        requestNativeObservation()
                    }
                } catch {
                    // The spooler acceptance is already durable. A transient
                    // status-read error must never rewrite it as ambiguous.
                    requestNativeObservation()
                }
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
