import Foundation

public enum UpdateMenuPresentation: Equatable, Sendable {
    case unavailable
    case readyToCheck
    case available(version: String)
    case waitingForIdle(version: String)

    public var title: String {
        switch self {
        case .unavailable:
            "Updates unavailable in this build"
        case .readyToCheck:
            "Check for Piqae Update…"
        case let .available(version):
            "Piqae \(Self.displayVersion(version)) Available…"
        case let .waitingForIdle(version):
            "Piqae \(Self.displayVersion(version)) Waiting for Idle"
        }
    }

    public var canOpenUpdater: Bool {
        switch self {
        case .readyToCheck, .available:
            true
        case .unavailable, .waitingForIdle:
            false
        }
    }

    private static func displayVersion(_ version: String) -> String {
        let oneLine = version
            .replacingOccurrences(of: "\r", with: "")
            .replacingOccurrences(of: "\n", with: "")
            .trimmingCharacters(in: .whitespaces)
        let bounded = String(oneLine.prefix(32))
        return bounded.isEmpty ? "Update" : bounded
    }
}
