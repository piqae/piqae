import Foundation

public enum UpdateHandoffReadiness: Equatable, Sendable {
    case ready
    case busy(queuedJobs: UInt32, activeJobs: UInt32, foregroundOperation: Bool)
    case agentUnavailable

    public init(status: LocalStatus?, foregroundOperation: Bool) {
        guard let status else {
            self = .agentUnavailable
            return
        }
        if status.queuedJobs == 0, status.activeJobs == 0, !foregroundOperation {
            self = .ready
        } else {
            self = .busy(
                queuedJobs: status.queuedJobs,
                activeJobs: status.activeJobs,
                foregroundOperation: foregroundOperation
            )
        }
    }

    public var canReplaceNativeComponents: Bool {
        self == .ready
    }
}
