import Foundation
import IOKit.pwr_mgt

public enum PiqaeMacActiveWorkPhase: String, Sendable {
    case download
    case render
    case nativeHandoff = "native handoff"
}

public protocol PiqaePowerAssertionDriver: Sendable {
    func acquire(reason: String) throws -> UInt32
    func release(_ identifier: UInt32)
}

public protocol PiqaePowerAssertionScheduler: Sendable {
    func schedule(
        after seconds: TimeInterval,
        _ operation: @escaping @Sendable () -> Void
    ) -> @Sendable () -> Void
}

public enum PiqaeMacPowerAssertionError: Error, Equatable, Sendable {
    case invalidDuration
    case iokit(IOReturn)
}

/// A best-effort, bounded no-idle-sleep assertion used only while an active
/// download, render, or native spooler handoff is in progress. It is never
/// held while the node is idle, waiting for work, or waiting for a printer.
public enum PiqaeMacActiveWorkPowerGuard {
    public static func withAssertion<T>(
        phase: PiqaeMacActiveWorkPhase,
        maximumDuration: TimeInterval,
        driver: any PiqaePowerAssertionDriver = PiqaeIOKitPowerAssertionDriver(),
        scheduler: any PiqaePowerAssertionScheduler = PiqaeDispatchPowerAssertionScheduler(),
        operation: () throws -> T
    ) throws -> T {
        guard maximumDuration > 0, maximumDuration <= 300 else {
            throw PiqaeMacPowerAssertionError.invalidDuration
        }
        let identifier: UInt32
        do {
            identifier = try driver.acquire(reason: "Piqae \(phase.rawValue)")
        } catch {
            // Power assertions improve reliability but are not the print
            // boundary. A sandbox or entitlement failure must not suppress
            // the one and only handoff attempt.
            return try operation()
        }
        let lease = PiqaePowerAssertionLease(identifier: identifier, driver: driver)
        let cancelExpiry = scheduler.schedule(after: maximumDuration) { lease.finish() }
        defer {
            cancelExpiry()
            lease.finish()
        }
        return try operation()
    }
}

public struct PiqaeIOKitPowerAssertionDriver: PiqaePowerAssertionDriver {
    public init() {}

    public func acquire(reason: String) throws -> UInt32 {
        var identifier = IOPMAssertionID(0)
        let result = IOPMAssertionCreateWithName(
            kIOPMAssertionTypeNoIdleSleep as CFString,
            IOPMAssertionLevel(kIOPMAssertionLevelOn),
            String(reason.prefix(128)) as CFString,
            &identifier
        )
        guard result == kIOReturnSuccess else {
            throw PiqaeMacPowerAssertionError.iokit(result)
        }
        return identifier
    }

    public func release(_ identifier: UInt32) {
        IOPMAssertionRelease(IOPMAssertionID(identifier))
    }
}

public struct PiqaeDispatchPowerAssertionScheduler: PiqaePowerAssertionScheduler {
    public init() {}

    public func schedule(
        after seconds: TimeInterval,
        _ operation: @escaping @Sendable () -> Void
    ) -> @Sendable () -> Void {
        let item = DispatchWorkItem(block: operation)
        let scheduled = PiqaeScheduledPowerAssertionExpiry(item: item)
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + seconds, execute: item)
        return { scheduled.cancel() }
    }
}

private final class PiqaeScheduledPowerAssertionExpiry: @unchecked Sendable {
    private let item: DispatchWorkItem

    init(item: DispatchWorkItem) {
        self.item = item
    }

    func cancel() {
        item.cancel()
    }
}

private final class PiqaePowerAssertionLease: @unchecked Sendable {
    private let lock = NSLock()
    private var identifier: UInt32?
    private let driver: any PiqaePowerAssertionDriver

    init(identifier: UInt32, driver: any PiqaePowerAssertionDriver) {
        self.identifier = identifier
        self.driver = driver
    }

    func finish() {
        let identifier: UInt32? = lock.withLock {
            defer { self.identifier = nil }
            return self.identifier
        }
        if let identifier { driver.release(identifier) }
    }
}
