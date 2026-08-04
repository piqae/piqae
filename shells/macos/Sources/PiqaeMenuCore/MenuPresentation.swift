import Foundation

public enum MenuPresentation {
    public static let cloudAndAPIAccessTitle = "Cloud & API access"

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
}
