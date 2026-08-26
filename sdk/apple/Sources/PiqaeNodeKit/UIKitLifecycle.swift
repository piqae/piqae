@preconcurrency import Foundation

#if os(iOS)
import BackgroundTasks
import UIKit

/// App delegates forward lifecycle and background-push events here. The
/// coordinator reports execution truth; it does not claim continuous runtime.
@MainActor
public final class PiqaeUIKitLifecycleCoordinator {
    private let node: PiqaeNode
    private var observers: [NSObjectProtocol] = []

    public init(node: PiqaeNode) {
        self.node = node
    }

    deinit {
        for observer in observers { NotificationCenter.default.removeObserver(observer) }
    }

    public func startObserving() {
        guard observers.isEmpty else { return }
        observers.append(
            NotificationCenter.default.addObserver(
                forName: UIApplication.didEnterBackgroundNotification,
                object: nil,
                queue: .main
            ) { [weak self] _ in
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    let remaining = UIApplication.shared.backgroundTimeRemaining
                    try? await self.node.reportHostLifecycle(.enteredBackground)
                    await self.node.updateExecutionContext(
                        PiqaeExecutionContext(
                            phase: .background,
                            source: .foreground,
                            remainingSeconds: remaining.isFinite ? remaining : nil
                        )
                    )
                }
            }
        )
        observers.append(
            NotificationCenter.default.addObserver(
                forName: UIApplication.willEnterForegroundNotification,
                object: nil,
                queue: .main
            ) { [weak self] _ in
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    try? await self.node.reportHostLifecycle(.woke)
                    try? await self.node.reportHostLifecycle(.enteredForeground)
                    await self.node.updateExecutionContext(.foreground)
                }
            }
        )
    }

    public func stopObserving() {
        for observer in observers { NotificationCenter.default.removeObserver(observer) }
        observers.removeAll()
    }

    /// Call from the app delegate's background notification handler. Return the
    /// result through Apple's fetch completion mapping. The hint contains no job
    /// metadata and only reconciles; it never accepts a job by itself.
    public func handleBackgroundPush(collapseID: String) async -> PiqaeWakeHintResult {
        guard let hint = try? PiqaeWakeHint(collapseID: collapseID, source: .backgroundPush) else {
            return .deferred(reason: "The wake hint was invalid.")
        }
        let remaining = UIApplication.shared.backgroundTimeRemaining
        guard remaining.isFinite, remaining >= 5 else {
            return .deferred(reason: "iPadOS did not grant a safe reconciliation budget.")
        }
        try? await node.reportHostLifecycle(.enteredBackground)
        let worker = Task { [node] in
            await node.handleWakeHint(
                hint,
                context: PiqaeExecutionContext(
                    phase: .background,
                    source: .backgroundPush,
                    remainingSeconds: remaining
                )
            )
        }
        let lifecycleNode = node
        let identifier = UIApplication.shared.beginBackgroundTask(withName: "Piqae reconcile") {
            worker.cancel()
            Task { try? await lifecycleNode.reportHostLifecycle(.suspendImminent) }
        }
        guard identifier != .invalid else {
            worker.cancel()
            return .deferred(reason: "iPadOS did not grant background execution.")
        }
        defer { UIApplication.shared.endBackgroundTask(identifier) }
        return await worker.value
    }

    /// Requests bounded continuation time only for work that already started in
    /// the foreground. It must not be used as an always-on listener.
    public func finishStartedWork<T: Sendable>(
        named name: String,
        operation: @escaping @Sendable () async throws -> T
    ) async throws -> T {
        let expiration = PiqaeExpirationState()
        let identifier = UIApplication.shared.beginBackgroundTask(withName: name) {
            Task { await expiration.expire() }
        }
        guard identifier != .invalid else {
            throw PiqaeNodeError.backgroundExecutionUnavailable
        }
        defer { UIApplication.shared.endBackgroundTask(identifier) }
        let result = try await operation()
        guard !(await expiration.isExpired) else {
            throw PiqaeNodeError.backgroundExecutionUnavailable
        }
        return result
    }
}

private actor PiqaeExpirationState {
    private var expired = false
    var isExpired: Bool { expired }
    func expire() { expired = true }
}

/// Schedules best-effort reconciliation, never immediate delivery. The host
/// must declare the identifier in BGTaskSchedulerPermittedIdentifiers and
/// enable Background fetch for the target.
@MainActor
public final class PiqaeUIKitMaintenanceScheduler {
    private let node: PiqaeNode
    private let identifier: String

    public init(node: PiqaeNode, identifier: String) throws {
        let trimmed = identifier.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.contains("."), trimmed.utf8.count <= 128 else {
            throw PiqaeNodeError.invalidConfiguration(
                "Background task identifiers must be bounded reverse-DNS names."
            )
        }
        self.node = node
        self.identifier = trimmed
    }

    /// Register during application launch, before scheduling a request.
    @discardableResult
    public func register() -> Bool {
        BGTaskScheduler.shared.register(forTaskWithIdentifier: identifier, using: nil) {
            [weak self] task in
            Task { @MainActor in
                guard let self else {
                    task.setTaskCompleted(success: false)
                    return
                }
                await self.perform(task)
            }
        }
    }

    public func schedule(earliestBeginDate: Date? = nil) throws {
        let request = BGAppRefreshTaskRequest(identifier: identifier)
        request.earliestBeginDate = earliestBeginDate
        try BGTaskScheduler.shared.submit(request)
    }

    private func perform(_ task: BGTask) async {
        guard task is BGAppRefreshTask else {
            task.setTaskCompleted(success: false)
            return
        }
        let worker = Task { [node, identifier] in
            guard let hint = try? PiqaeWakeHint(
                collapseID: "scheduled-maintenance",
                source: .scheduledMaintenance
            ) else { return false }
            let result = await node.handleWakeHint(
                hint,
                context: PiqaeExecutionContext(
                    phase: .background,
                    source: .scheduledMaintenance,
                    remainingSeconds: nil
                )
            )
            if !Task.isCancelled {
                try? scheduleNext(identifier: identifier)
            }
            return result == .reconciledWithoutLeasing && !Task.isCancelled
        }
        task.expirationHandler = { worker.cancel() }
        task.setTaskCompleted(success: await worker.value)
    }
}

private func scheduleNext(identifier: String) throws {
    let request = BGAppRefreshTaskRequest(identifier: identifier)
    request.earliestBeginDate = Date().addingTimeInterval(15 * 60)
    try BGTaskScheduler.shared.submit(request)
}
#endif
