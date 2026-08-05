import Foundation

public struct NodeConnectConsentPresentation: Equatable, Sendable {
    public let title: String
    public let detailText: String
    public let preselectCurrentPrinters: Bool

    public init(preview: NodeConnectPreview) {
        let workspace = preview.workspaceName.trimmingCharacters(in: .whitespacesAndNewlines)
        if let service = preview.requestingServiceName?.trimmingCharacters(in: .whitespacesAndNewlines),
            !service.isEmpty
        {
            title = "Allow \(service) to print?"
            detailText = "\(service) is requesting printer access for \(workspace).\n\n\(Self.permissionSummary(preview.requestedScopes))\n\nChoose the printers it may use. You can change or remove this access later."
            preselectCurrentPrinters = false
        } else {
            title = "Connect \(workspace) to this computer?"
            detailText = "This lets your \(workspace) workspace use printers connected to this computer.\n\n\(Self.permissionSummary(preview.requestedScopes))\n\nConfirm the printers it may use. You can change or remove this access later."
            preselectCurrentPrinters = true
        }
    }

    private static func permissionSummary(_ scopes: [String]) -> String {
        var lines = [String]()
        if scopes.contains("discover_printers") { lines.append("• See approved printers") }
        if scopes.contains("print") { lines.append("• Send print jobs") }
        if scopes.contains("monitor_jobs") { lines.append("• View print status") }
        if lines.isEmpty { lines.append("• Use approved printing features") }
        return "This connection can:\n" + lines.joined(separator: "\n")
    }
}
