import AppKit
import PiqaeNodeKit
@testable import PiqaeMenuCore
import XCTest

@MainActor
final class MacHostLifecycleTests: XCTestCase {
    func testSleepWakeAndNetworkFactsAreReportedInOrder() async {
        // Exercise the ordered report boundary repeatedly. Network-path facts
        // and workspace notifications are both enqueued synchronously once
        // they reach the main actor.
        for iteration in 0..<100 {
            let center = NotificationCenter()
            let reporter = LifecycleReporterSpy()
            let network = NetworkPathSourceFake()
            let monitor = PiqaeMacHostLifecycleMonitor(
                reporter: reporter,
                workspaceCenter: center,
                network: network
            )
            let initialReports = await reporter.expectEventCount(
                5,
                description: "initial lifecycle reports \(iteration)"
            )
            let restartedReports = await reporter.expectEventCount(
                7,
                description: "restarted lifecycle reports \(iteration)"
            )

            monitor.start()
            network.emit(.constrained)
            center.post(name: NSWorkspace.willSleepNotification, object: nil)
            center.post(name: NSWorkspace.didWakeNotification, object: nil)
            await fulfillment(of: [initialReports], timeout: 1)

            let events = await reporter.events()
            XCTAssertEqual(
                events,
                [.started, .networkConstrained, .suspendImminent, .sleeping, .woke],
                "iteration \(iteration)"
            )
            monitor.stop()
            XCTAssertEqual(network.cancelCount, 1, "iteration \(iteration)")

            monitor.start()
            network.emit(.available)
            await fulfillment(of: [restartedReports], timeout: 1)
            let restartedEvents = await reporter.events()
            XCTAssertEqual(
                restartedEvents,
                [
                    .started, .networkConstrained, .suspendImminent, .sleeping, .woke,
                    .started, .networkAvailable,
                ],
                "iteration \(iteration)"
            )
            monitor.stop()
            XCTAssertEqual(network.cancelCount, 2, "iteration \(iteration)")
        }
    }

    func testNetworkFactCannotCrossImmediateStopAndRestart() async {
        for iteration in 0..<100 {
            let reporter = LifecycleReporterSpy()
            let network = NetworkPathSourceFake()
            let monitor = PiqaeMacHostLifecycleMonitor(
                reporter: reporter,
                workspaceCenter: NotificationCenter(),
                network: network
            )
            let initiallyStarted = await reporter.expectEventCount(
                1,
                description: "initial start report \(iteration)"
            )
            let restartedReports = await reporter.expectEventCount(
                3,
                description: "reports after immediate restart \(iteration)"
            )

            monitor.start()
            await fulfillment(of: [initiallyStarted], timeout: 1)
            network.emit(.unavailable)
            monitor.stop()
            monitor.start()
            network.emit(.available)
            await fulfillment(of: [restartedReports], timeout: 1)

            let events = await reporter.events()
            XCTAssertEqual(
                events,
                [.started, .started, .networkAvailable],
                "iteration \(iteration)"
            )
            monitor.stop()
            XCTAssertEqual(network.cancelCount, 2, "iteration \(iteration)")
        }
    }

    func testQueuedOldGenerationFactsCannotReportAfterRestart() async {
        for iteration in 0..<100 {
            let center = NotificationCenter()
            let reporter = LifecycleReporterSpy()
            let network = NetworkPathSourceFake()
            let monitor = PiqaeMacHostLifecycleMonitor(
                reporter: reporter,
                workspaceCenter: center,
                network: network
            )
            let restartedReports = await reporter.expectEventCount(
                2,
                description: "new-generation reports \(iteration)"
            )

            monitor.start()
            network.emit(.unavailable)
            center.post(name: NSWorkspace.willSleepNotification, object: nil)
            monitor.stop()
            monitor.start()
            network.emit(.available)
            await fulfillment(of: [restartedReports], timeout: 1)

            let events = await reporter.events()
            XCTAssertEqual(
                events,
                [.started, .networkAvailable],
                "iteration \(iteration)"
            )
            monitor.stop()
            XCTAssertEqual(network.cancelCount, 2, "iteration \(iteration)")
        }
    }

    func testRestartedReportsWaitForInFlightOldGenerationReport() async {
        let reporter = GatedLifecycleReporter()
        let network = NetworkPathSourceFake()
        let monitor = PiqaeMacHostLifecycleMonitor(
            reporter: reporter,
            workspaceCenter: NotificationCenter(),
            network: network
        )
        let initiallyStarted = await reporter.expectCompletedCount(
            1,
            description: "initial start completed"
        )

        monitor.start()
        await fulfillment(of: [initiallyStarted], timeout: 1)

        let oldReportBegan = expectation(description: "old-generation report began")
        let restartedReportBeganEarly = expectation(
            description: "restarted report began before old report completed"
        )
        restartedReportBeganEarly.isInverted = true
        await reporter.gateNextReport(
            began: oldReportBegan,
            concurrentReportBegan: restartedReportBeganEarly
        )
        let allReportsCompleted = await reporter.expectCompletedCount(
            4,
            description: "old and restarted reports completed"
        )

        network.emit(.unavailable)
        await fulfillment(of: [oldReportBegan], timeout: 1)
        monitor.stop()
        monitor.start()
        network.emit(.available)

        await fulfillment(of: [restartedReportBeganEarly], timeout: 0.1)
        await reporter.releaseGatedReport()
        await fulfillment(of: [allReportsCompleted], timeout: 1)

        let expected: [PiqaeHostLifecycleEvent] = [
            .started, .networkUnavailable, .started, .networkAvailable,
        ]
        let begunEvents = await reporter.begunEvents()
        let completedEvents = await reporter.completedEvents()
        XCTAssertEqual(begunEvents, expected)
        XCTAssertEqual(completedEvents, expected)
        monitor.stop()
        XCTAssertEqual(network.cancelCount, 2)
    }
}

private actor LifecycleReporterSpy: PiqaeHostLifecycleReporter {
    private var values: [PiqaeHostLifecycleEvent] = []
    private var eventCountExpectations: [(count: Int, expectation: XCTestExpectation)] = []

    func report(_ event: PiqaeHostLifecycleEvent) {
        values.append(event)
        let ready = eventCountExpectations.filter { values.count >= $0.count }
        eventCountExpectations.removeAll { values.count >= $0.count }
        for waiter in ready { waiter.expectation.fulfill() }
    }

    func expectEventCount(_ count: Int, description: String) -> XCTestExpectation {
        let expectation = XCTestExpectation(description: description)
        if values.count >= count {
            expectation.fulfill()
        } else {
            eventCountExpectations.append((count, expectation))
        }
        return expectation
    }

    func events() -> [PiqaeHostLifecycleEvent] { values }
}

private actor GatedLifecycleReporter: PiqaeHostLifecycleReporter {
    private var begunValues: [PiqaeHostLifecycleEvent] = []
    private var completedValues: [PiqaeHostLifecycleEvent] = []
    private var completedCountExpectations: [(count: Int, expectation: XCTestExpectation)] = []
    private var shouldGateNextReport = false
    private var gatedReportBegan: XCTestExpectation?
    private var concurrentReportBegan: XCTestExpectation?
    private var gateContinuation: CheckedContinuation<Void, Never>?

    func report(_ event: PiqaeHostLifecycleEvent) async {
        begunValues.append(event)
        if gateContinuation != nil, let expectation = concurrentReportBegan {
            concurrentReportBegan = nil
            expectation.fulfill()
        }
        if shouldGateNextReport {
            shouldGateNextReport = false
            await withCheckedContinuation { continuation in
                gateContinuation = continuation
                gatedReportBegan?.fulfill()
            }
        }
        completedValues.append(event)
        let ready = completedCountExpectations.filter { completedValues.count >= $0.count }
        completedCountExpectations.removeAll { completedValues.count >= $0.count }
        for waiter in ready { waiter.expectation.fulfill() }
    }

    func gateNextReport(
        began: XCTestExpectation,
        concurrentReportBegan: XCTestExpectation
    ) {
        shouldGateNextReport = true
        gatedReportBegan = began
        self.concurrentReportBegan = concurrentReportBegan
    }

    func releaseGatedReport() {
        let continuation = gateContinuation
        gateContinuation = nil
        gatedReportBegan = nil
        concurrentReportBegan = nil
        continuation?.resume()
    }

    func expectCompletedCount(_ count: Int, description: String) -> XCTestExpectation {
        let expectation = XCTestExpectation(description: description)
        if completedValues.count >= count {
            expectation.fulfill()
        } else {
            completedCountExpectations.append((count, expectation))
        }
        return expectation
    }

    func begunEvents() -> [PiqaeHostLifecycleEvent] { begunValues }
    func completedEvents() -> [PiqaeHostLifecycleEvent] { completedValues }
}

@MainActor
private final class NetworkPathSourceFake: PiqaeMacNetworkPathSource {
    var onChange: (@MainActor @Sendable (PiqaeMacNetworkPathState) -> Void)?
    private(set) var cancelCount = 0
    func start() {}
    func cancel() { cancelCount += 1 }
    func emit(_ state: PiqaeMacNetworkPathState) { onChange?(state) }
}
