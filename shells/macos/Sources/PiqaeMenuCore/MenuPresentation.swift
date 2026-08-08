import Foundation

public enum MenuPresentation {
    public static let cloudAndAPIAccessTitle = "Available to connected services"
    public static let connectionsTitle = "Connections"
    public static let queueTitle = "Queue"

    public static func printerActivityTitle(
        state: String,
        queued: UInt32?,
        active: UInt32?
    ) -> String {
        let stateTitle = state.replacingOccurrences(of: "_", with: " ").capitalized
        var activity: [String] = []
        if let queued, queued > 0 {
            activity.append("\(queued) queued")
        }
        if let active, active > 0 {
            activity.append("\(active) active")
        }
        return ([stateTitle] + activity).joined(separator: " · ")
    }

    public static func printPresetSectionTitle(count: Int) -> String {
        "PRINT PRESETS (\(count))"
    }

    public static func connectionStatusTitle(connection: String) -> String {
        switch connection {
        case "connected": "Connected"
        case "connecting": "Connecting…"
        case "degraded": "Connection needs attention"
        case "local_only": "No cloud connections"
        default: "Connection status unavailable"
        }
    }

    public static func testPresetTitle(_ presetName: String) -> String {
        "Test “\(presetName)”…"
    }
}
