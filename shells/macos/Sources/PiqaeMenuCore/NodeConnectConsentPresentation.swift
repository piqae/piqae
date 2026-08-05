import Foundation

public struct NodeConnectConsentPresentation: Equatable, Sendable {
    public let title: String
    public let detailText: String
    public let permissionsText: String
    public let defaultGrant: NodePrinterGrant

    public init(preview: NodeConnectPreview) {
        let workspace = preview.workspaceName.trimmingCharacters(in: .whitespacesAndNewlines)
        if let service = preview.requestingServiceName?.trimmingCharacters(in: .whitespacesAndNewlines),
            !service.isEmpty
        {
            title = "Allow \(service) to print?"
            detailText = "For the \(workspace) workspace. Choose which printers \(service) may use. You can change or remove access later."
            permissionsText = Self.permissionSummary(preview.requestedScopes)
            defaultGrant = .allLocalPrinters
        } else {
            title = "Allow \(workspace) to print?"
            detailText = "Choose which printers this workspace may use. You can change or remove access later."
            permissionsText = Self.permissionSummary(preview.requestedScopes)
            defaultGrant = .allLocalPrinters
        }
    }

    private static func permissionSummary(_ scopes: [String]) -> String {
        var lines = [String]()
        if scopes.contains("discover_printers") { lines.append("• See approved printers") }
        if scopes.contains("print") { lines.append("• Send print jobs") }
        if scopes.contains("monitor_jobs") { lines.append("• View print status") }
        if lines.isEmpty { lines.append("• Use approved printing features") }
        return lines.joined(separator: "   ")
    }
}
