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
    var onChange: (@MainActor @Sendable (PiqaeMacNetworkPathState) -> Void)? { get set }
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
    private var active = false
    private var generation: UInt64 = 0

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
        guard !active else { return }
        generation &+= 1
        active = true
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
            switch state {
            case .available: self?.enqueue(.networkAvailable)
            case .constrained: self?.enqueue(.networkConstrained)
            case .unavailable: self?.enqueue(.networkUnavailable)
            }
        }
        network.start()
    }

    public func stop() {
        active = false
        generation &+= 1
        for observer in observers { workspaceCenter.removeObserver(observer) }
        observers.removeAll()
        network.onChange = nil
        network.cancel()
        reportTail?.cancel()
        // Retain the cancelled tail as the next generation's completion
        // predecessor. A reporter call already in flight cannot be retracted,
        // and restarted facts must not overtake its completion.
    }

    /// Waits for reports already enqueued on the main actor. It does not admit
    /// a background callback that has not reached the main actor yet.
    public func flushForTesting() async {
        await reportTail?.value
    }

    private func enqueue(_ event: PiqaeHostLifecycleEvent) {
        let previous = reportTail
        let reporter = reporter
        let generation = generation
        reportTail = Task { @MainActor [weak self] in
            await previous?.value
            // A report already executing at stop may finish before the next
            // generation begins. An event that was only queued by an older
            // monitor generation must not begin reporting after stop.
            guard
                !Task.isCancelled,
                let self,
                self.active,
                self.generation == generation
            else { return }
            try? await reporter.report(event)
        }
    }
}

@MainActor
public final class PiqaeNWPathSource: PiqaeMacNetworkPathSource {
    public var onChange: (@MainActor @Sendable (PiqaeMacNetworkPathState) -> Void)?

    private var monitor: NWPathMonitor?
    private let queue = DispatchQueue(label: "com.piqae.nodekit.network-path")
    private var started = false
    private var generation: UInt64 = 0

    public init() {}

    public func start() {
        guard !started else { return }
        started = true
        generation &+= 1
        let generation = generation
        let monitor = NWPathMonitor()
        self.monitor = monitor
        monitor.pathUpdateHandler = { [weak self] path in
            guard let self else { return }
            let state: PiqaeMacNetworkPathState
            if path.status != .satisfied { state = .unavailable }
            else if path.isConstrained { state = .constrained }
            else { state = .available }
            Task { @MainActor [weak self] in
                guard
                    let self,
                    self.started,
                    self.generation == generation
                else { return }
                self.onChange?(state)
            }
        }
        monitor.start(queue: queue)
    }

    public func cancel() {
        guard started else { return }
        started = false
        generation &+= 1
        monitor?.cancel()
        monitor = nil
    }
}
