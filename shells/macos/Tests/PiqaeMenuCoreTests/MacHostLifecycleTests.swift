import AppKit
import PiqaeNodeKit
@testable import PiqaeMenuCore
import XCTest

@MainActor
final class MacHostLifecycleTests: XCTestCase {
    func testSleepWakeAndNetworkFactsAreReportedInOrder() async {
        let center = NotificationCenter()
        let reporter = LifecycleReporterSpy()
        let network = NetworkPathSourceFake()
        let monitor = PiqaeMacHostLifecycleMonitor(
            reporter: reporter,
            workspaceCenter: center,
            network: network
        )

        monitor.start()
        network.emit(.constrained)
        center.post(name: NSWorkspace.willSleepNotification, object: nil)
        center.post(name: NSWorkspace.didWakeNotification, object: nil)
        await monitor.flushForTesting()

        let events = await reporter.events()
        XCTAssertEqual(
            events,
            [.started, .suspendImminent, .sleeping, .woke, .networkConstrained]
        )
        monitor.stop()
        XCTAssertEqual(network.cancelCount, 1)
    }
}

private actor LifecycleReporterSpy: PiqaeHostLifecycleReporter {
    private var values: [PiqaeHostLifecycleEvent] = []
    func report(_ event: PiqaeHostLifecycleEvent) { values.append(event) }
    func events() -> [PiqaeHostLifecycleEvent] { values }
}

@MainActor
private final class NetworkPathSourceFake: PiqaeMacNetworkPathSource {
    var onChange: (@Sendable (PiqaeMacNetworkPathState) -> Void)?
    private(set) var cancelCount = 0
    func start() {}
    func cancel() { cancelCount += 1 }
    func emit(_ state: PiqaeMacNetworkPathState) { onChange?(state) }
}
