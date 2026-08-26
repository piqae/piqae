import Foundation

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
    /// Inventory and connector state were reconciled. A wake hint never leases
    /// or accepts a job by itself.
    case reconciledWithoutLeasing
    case deferred(reason: String)
}
