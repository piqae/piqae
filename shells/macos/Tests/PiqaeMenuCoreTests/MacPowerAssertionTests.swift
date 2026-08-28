@testable import PiqaeMenuCore
import XCTest

final class MacPowerAssertionTests: XCTestCase {
    func testAssertionIsReleasedAfterActiveWork() throws {
        let driver = PowerDriverSpy()
        let scheduler = PowerSchedulerFake()
        let value = try PiqaeMacActiveWorkPowerGuard.withAssertion(
            phase: .nativeHandoff,
            maximumDuration: 30,
            driver: driver,
            scheduler: scheduler
        ) { 42 }

        XCTAssertEqual(value, 42)
        XCTAssertEqual(driver.acquiredReasons, ["Piqae native handoff"])
        XCTAssertEqual(driver.released, [7])
    }

    func testExpiryAndScopeExitReleaseOnlyOnce() throws {
        let driver = PowerDriverSpy()
        let scheduler = PowerSchedulerFake()
        try PiqaeMacActiveWorkPowerGuard.withAssertion(
            phase: .render,
            maximumDuration: 10,
            driver: driver,
            scheduler: scheduler
        ) {
            scheduler.fire()
        }
        XCTAssertEqual(driver.released, [7])
    }

    func testIdleLengthAssertionIsRejected() {
        XCTAssertThrowsError(
            try PiqaeMacActiveWorkPowerGuard.withAssertion(
                phase: .download,
                maximumDuration: 600,
                driver: PowerDriverSpy(),
                scheduler: PowerSchedulerFake()
            ) {}
        )
    }

    func testAcquisitionFailureRunsOperationOnceAndPropagatesItsError() {
        let driver = PowerDriverSpy(acquisitionError: TestFailure.acquisition)
        var attempts = 0
        XCTAssertThrowsError(
            try PiqaeMacActiveWorkPowerGuard.withAssertion(
                phase: .nativeHandoff,
                maximumDuration: 30,
                driver: driver,
                scheduler: PowerSchedulerFake()
            ) {
                attempts += 1
                throw TestFailure.operation
            }
        ) { error in
            XCTAssertEqual(error as? TestFailure, .operation)
        }
        XCTAssertEqual(attempts, 1)
        XCTAssertTrue(driver.released.isEmpty)
    }
}

private enum TestFailure: Error { case acquisition, operation }

private final class PowerDriverSpy: @unchecked Sendable, PiqaePowerAssertionDriver {
    private let lock = NSLock()
    private(set) var acquiredReasons: [String] = []
    private(set) var released: [UInt32] = []
    private let acquisitionError: Error?

    init(acquisitionError: Error? = nil) {
        self.acquisitionError = acquisitionError
    }

    func acquire(reason: String) throws -> UInt32 {
        lock.withLock { acquiredReasons.append(reason) }
        if let acquisitionError { throw acquisitionError }
        return 7
    }

    func release(_ identifier: UInt32) {
        lock.withLock { released.append(identifier) }
    }
}

private final class PowerSchedulerFake: @unchecked Sendable, PiqaePowerAssertionScheduler {
    private let lock = NSLock()
    private var operation: (@Sendable () -> Void)?

    func schedule(
        after seconds: TimeInterval,
        _ operation: @escaping @Sendable () -> Void
    ) -> @Sendable () -> Void {
        lock.withLock { self.operation = operation }
        return { [weak self] in self?.lock.withLock { self?.operation = nil } }
    }

    func fire() {
        let operation = lock.withLock { self.operation }
        operation?()
    }
}
