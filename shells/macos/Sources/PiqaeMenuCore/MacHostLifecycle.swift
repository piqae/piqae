@preconcurrency import AppKit
import Foundation
@preconcurrency import Network
import PiqaeNodeKit

public enum PiqaeMacNetworkPathState: Equatable, Sendable {
    case available
    case constrained
    case unavailable
}

@MainActor
public protocol PiqaeMacNetworkPathSource: AnyObject {
    var onChange: (@Sendable (PiqaeMacNetworkPathState) -> Void)? { get set }
    func start()
    func cancel()
}

/// Bridges Apple host facts into the shared HostLifecycle contract. Use
/// `NSWorkspace.notificationCenter`, not the default notification center.
@MainActor
public final class PiqaeMacHostLifecycleMonitor {
    private let reporter: any PiqaeHostLifecycleReporter
    private let workspaceCenter: NotificationCenter
    private let network: any PiqaeMacNetworkPathSource
    private var observers: [NSObjectProtocol] = []
    private var reportTail: Task<Void, Never>?

    public init(
        reporter: any PiqaeHostLifecycleReporter,
        workspaceCenter: NotificationCenter = NSWorkspace.shared.notificationCenter,
        network: (any PiqaeMacNetworkPathSource)? = nil
    ) {
        self.reporter = reporter
        self.workspaceCenter = workspaceCenter
        self.network = network ?? PiqaeNWPathSource()
    }

    deinit {
        reportTail?.cancel()
    }

    public func start() {
        guard observers.isEmpty else { return }
        enqueue(.started)
        observers.append(
            workspaceCenter.addObserver(
                forName: NSWorkspace.willSleepNotification,
                object: nil,
                queue: .main
            ) { [weak self] _ in
                MainActor.assumeIsolated {
                    self?.enqueue(.suspendImminent)
                    self?.enqueue(.sleeping)
                }
            }
        )
        observers.append(
            workspaceCenter.addObserver(
                forName: NSWorkspace.didWakeNotification,
                object: nil,
                queue: .main
            ) { [weak self] _ in
                MainActor.assumeIsolated { self?.enqueue(.woke) }
            }
        )
        network.onChange = { [weak self] state in
            Task { @MainActor [weak self] in
                switch state {
                case .available: self?.enqueue(.networkAvailable)
                case .constrained: self?.enqueue(.networkConstrained)
                case .unavailable: self?.enqueue(.networkUnavailable)
                }
            }
        }
        network.start()
    }

    public func stop() {
        for observer in observers { workspaceCenter.removeObserver(observer) }
        observers.removeAll()
        network.onChange = nil
        network.cancel()
        reportTail?.cancel()
        reportTail = nil
    }

    /// Waits for reports already queued by synchronous Apple notifications.
    public func flushForTesting() async {
        await reportTail?.value
    }

    private func enqueue(_ event: PiqaeHostLifecycleEvent) {
        let previous = reportTail
        let reporter = reporter
        reportTail = Task { @MainActor in
            await previous?.value
            guard !Task.isCancelled else { return }
            try? await reporter.report(event)
        }
    }
}

@MainActor
public final class PiqaeNWPathSource: PiqaeMacNetworkPathSource {
    public var onChange: (@Sendable (PiqaeMacNetworkPathState) -> Void)?

    private var monitor: NWPathMonitor?
    private let queue = DispatchQueue(label: "com.piqae.nodekit.network-path")
    private var started = false

    public init() {}

    public func start() {
        guard !started else { return }
        started = true
        let monitor = NWPathMonitor()
        self.monitor = monitor
        monitor.pathUpdateHandler = { [weak self] path in
            guard let self else { return }
            let state: PiqaeMacNetworkPathState
            if path.status != .satisfied { state = .unavailable }
            else if path.isConstrained { state = .constrained }
            else { state = .available }
            Task { @MainActor [weak self] in self?.onChange?(state) }
        }
        monitor.start(queue: queue)
    }

    public func cancel() {
        guard started else { return }
        started = false
        monitor?.cancel()
        monitor = nil
    }
}
