import Foundation

public enum UpdateMenuPresentation: Equatable, Sendable {
    case unavailable
    case readyToCheck
    case available(version: String)
    case waitingForIdle(version: String)

    public var title: String {
        switch self {
        case .unavailable:
            "App updates unavailable in this build"
        case .readyToCheck:
            "Check for Piqae App Update…"
        case let .available(version):
            "Piqae App \(Self.displayVersion(version)) Available…"
        case let .waitingForIdle(version):
            "Piqae App \(Self.displayVersion(version)) Waiting for Idle"
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
