import Foundation

/// Wire names match `node-host-api::LifecycleEvent`. Apple lifecycle adapters
/// report facts through this contract; they do not independently decide lease
/// admission.
public enum PiqaeHostLifecycleEvent: String, Codable, Sendable, CaseIterable {
    case started
    case enteredForeground = "entered_foreground"
    case enteredBackground = "entered_background"
    case suspendImminent = "suspend_imminent"
    case sleeping
    case woke
    case networkAvailable = "network_available"
    case networkConstrained = "network_constrained"
    case networkUnavailable = "network_unavailable"
    case shutdownRequested = "shutdown_requested"
}

public protocol PiqaeHostLifecycleReporter: Sendable {
    func report(_ event: PiqaeHostLifecycleEvent) async throws
}

public enum PiqaeExecutionPhase: String, Codable, Sendable {
    case foreground
    case background
    case suspended
}

public enum PiqaeWakeSource: String, Codable, Sendable {
    case foreground
    case backgroundPush = "background_push"
    case scheduledMaintenance = "scheduled_maintenance"
    case bluetoothAccessory = "bluetooth_accessory"
    case externalAccessory = "external_accessory"
}

public struct PiqaeExecutionContext: Codable, Equatable, Sendable {
    public let phase: PiqaeExecutionPhase
    public let source: PiqaeWakeSource
    public let remainingSeconds: TimeInterval?

    public init(
        phase: PiqaeExecutionPhase,
        source: PiqaeWakeSource,
        remainingSeconds: TimeInterval? = nil
    ) {
        self.phase = phase
        self.source = source
        self.remainingSeconds = remainingSeconds
    }

    public static let foreground = PiqaeExecutionContext(
        phase: .foreground,
        source: .foreground
    )
}

public struct PiqaePendingHandoff: Equatable, Sendable {
    public let payloadIsDurable: Bool
    public let nativeBoundaryMayHaveBeenCrossed: Bool
    public let estimatedSecondsToNativeAcceptance: TimeInterval

    public init(
        payloadIsDurable: Bool,
        nativeBoundaryMayHaveBeenCrossed: Bool = false,
        estimatedSecondsToNativeAcceptance: TimeInterval
    ) {
        self.payloadIsDurable = payloadIsDurable
        self.nativeBoundaryMayHaveBeenCrossed = nativeBoundaryMayHaveBeenCrossed
        self.estimatedSecondsToNativeAcceptance = max(0, estimatedSecondsToNativeAcceptance)
    }
}

public enum PiqaeHandoffAdmission: Equatable, Sendable {
    case admit
    case finishAlreadyStarted
    case deferUntilForeground(reason: String)
}

public struct PiqaeBackgroundAdmissionPolicy: Sendable {
    public let safetyMarginSeconds: TimeInterval

    public init(safetyMarginSeconds: TimeInterval = 5) {
        self.safetyMarginSeconds = max(0, safetyMarginSeconds)
    }

    public func evaluate(
        _ handoff: PiqaePendingHandoff,
        context: PiqaeExecutionContext,
        availability: PiqaeNodeAvailabilityClass
    ) -> PiqaeHandoffAdmission {
        if handoff.nativeBoundaryMayHaveBeenCrossed {
            return .finishAlreadyStarted
        }
        if context.phase == .foreground { return .admit }
        if context.phase == .suspended {
            return .deferUntilForeground(reason: "The host application is suspended.")
        }
        guard availability != .foregroundOnly else {
            return .deferUntilForeground(
                reason: "This route is intentionally foreground-only."
            )
        }
        guard handoff.payloadIsDurable else {
            return .deferUntilForeground(
                reason: "Background work cannot accept a job before its payload is durable."
            )
        }
        guard let remaining = context.remainingSeconds else {
            return .deferUntilForeground(
                reason: "The system did not provide a measurable execution budget."
            )
        }
        let required = handoff.estimatedSecondsToNativeAcceptance + safetyMarginSeconds
        guard remaining >= required else {
            return .deferUntilForeground(
                reason: "The remaining execution budget is too short for a safe native handoff."
            )
        }
        return .admit
    }
}

public struct PiqaeWakeHint: Equatable, Sendable {
    /// Opaque collapse identifier only. It must not encode tenant, job, printer,
    /// document, or content-location information.
    public let collapseID: String
    public let source: PiqaeWakeSource

    public init(collapseID: String, source: PiqaeWakeSource) throws {
        let trimmed = collapseID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, trimmed.utf8.count <= 64 else {
            throw PiqaeNodeError.invalidConfiguration(
                "Wake-hint identifiers must contain 1 to 64 UTF-8 bytes."
            )
        }
        self.collapseID = trimmed
        self.source = source
    }
}

public enum PiqaeWakeHintResult: Equatable, Sendable {
    /// Inventory and durable adapter work were reconciled within the execution
    /// budget. The hint carries no job data and grants no eligibility itself.
    case reconciled
    case deferred(reason: String)
}

public enum PiqaeRemoteNotificationAvailability: String, Codable, Sendable {
    /// The OS may launch or briefly resume the app. Delivery and runtime are
    /// never guaranteed.
    case opportunisticWhileInstalled = "opportunistic_while_installed"
    /// iPadOS does not launch an app that the user force-quit, and an off or
    /// unreachable device cannot receive the hint.
    case unavailableWhenTerminated = "unavailable_when_terminated"
}

public enum PiqaeAPNsEnvironment: String, Codable, Sendable {
    case development
    case production
}

public struct PiqaeSensitiveDeviceToken: Sendable, CustomStringConvertible,
    CustomDebugStringConvertible
{
    private let data: Data

    public init(_ data: Data) throws {
        guard !data.isEmpty, data.count <= 256 else {
            throw PiqaeNodeError.invalidConfiguration(
                "The APNs device token must contain 1 to 256 bytes."
            )
        }
        self.data = data
    }

    public var description: String { "<redacted>" }
    public var debugDescription: String { "<redacted>" }

    public func withBytes<T>(_ body: (Data) throws -> T) rethrows -> T {
        try body(data)
    }
}

public struct PiqaeRemoteNotificationRegistration: Sendable {
    public let installationID: PiqaeInstallationID
    public let token: PiqaeSensitiveDeviceToken
    public let environment: PiqaeAPNsEnvironment
    public let bundleIdentifier: String

    public init(
        installationID: PiqaeInstallationID,
        token: PiqaeSensitiveDeviceToken,
        environment: PiqaeAPNsEnvironment,
        bundleIdentifier: String
    ) throws {
        let bundleIdentifier = bundleIdentifier.trimmingCharacters(in: .whitespacesAndNewlines)
        guard bundleIdentifier.contains("."), bundleIdentifier.utf8.count <= 255 else {
            throw PiqaeNodeError.invalidConfiguration(
                "The notification bundle identifier must be a bounded reverse-DNS name."
            )
        }
        self.installationID = installationID
        self.token = token
        self.environment = environment
        self.bundleIdentifier = bundleIdentifier
    }
}

/// Implemented by the host app's backend client. APNs signing keys and
/// platform service credentials must remain on that backend.
public protocol PiqaeRemoteNotificationRegistrationProvider: Sendable {
    func register(_ request: PiqaeRemoteNotificationRegistration) async throws
}
