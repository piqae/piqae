import Foundation
import CryptoKit

public enum NodeConnectApplicationLinkError: Error, Equatable {
    case invalidURL
    case unsupportedRoute
    case invalidCapability
    case invalidReturnURL
}

public enum NodeConnectLinkTransport: Equatable, Sendable {
    /// An Associated Domains verified HTTPS Universal Link. This is the only
    /// transport emitted for newly-created sessions.
    case universalLink
    /// Compatibility for preview builds released before Universal Links.
    case deprecatedCustomScheme
}

/// A strictly parsed application link. The capability is intentionally kept
/// out of `CustomStringConvertible` and all diagnostic values.
public struct NodeConnectApplicationLink: Equatable, @unchecked Sendable {
    public let enrolmentCapability: String
    public let controlPlaneURL: URL
    public let returnURL: URL?
    public let transport: NodeConnectLinkTransport

    public init(url: URL) throws {
        guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false),
            components.user == nil,
            components.password == nil,
            components.query == nil,
            let fragment = components.percentEncodedFragment
        else {
            throw NodeConnectApplicationLinkError.unsupportedRoute
        }

        switch (components.scheme?.lowercased(), components.host?.lowercased(), components.path) {
        case ("https", "app.piqae.com", "/connect"):
            transport = .universalLink
        case ("piqae", "connect", ""):
            transport = .deprecatedCustomScheme
        default:
            throw NodeConnectApplicationLinkError.unsupportedRoute
        }

        let items = URLComponents(string: "x://x/?\(fragment)")?.queryItems ?? []
        guard items.count == Set(items.map(\.name)).count,
            let tokenItem = items.first(where: { $0.name == "enrolment_token" }),
            items.allSatisfy({ ["enrolment_token", "control_plane_url", "return_url"].contains($0.name) }),
            let capability = tokenItem.value,
            capability.range(
                of: #"^piq_enr_[A-Za-z0-9_-]{32}$"#,
                options: .regularExpression
            ) != nil
        else {
            throw NodeConnectApplicationLinkError.invalidCapability
        }
        guard let rawControlPlaneURL = items.first(where: { $0.name == "control_plane_url" })?.value,
            let parsedControlPlaneURL = URL(string: rawControlPlaneURL),
            Self.isSafeControlPlaneURL(parsedControlPlaneURL)
        else { throw NodeConnectApplicationLinkError.invalidURL }
        if let rawReturnURL = items.first(where: { $0.name == "return_url" })?.value {
            guard let parsed = URL(string: rawReturnURL), Self.isSafeReturnURL(parsed) else {
                throw NodeConnectApplicationLinkError.invalidReturnURL
            }
            returnURL = parsed
        } else {
            returnURL = nil
        }
        enrolmentCapability = capability
        controlPlaneURL = parsedControlPlaneURL
    }

    /// A non-secret, stable identifier suitable for replay suppression and UI
    /// bookkeeping. The one-time capability itself must never be persisted.
    public var capabilityFingerprint: String {
        SHA256.hash(data: Data(enrolmentCapability.utf8))
            .map { String(format: "%02x", $0) }
            .joined()
    }

    private static func isSafeReturnURL(_ url: URL) -> Bool {
        guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false),
            components.user == nil,
            components.password == nil,
            components.fragment == nil,
            components.host != nil
        else { return false }
        if components.scheme?.lowercased() == "https" { return true }
        return components.scheme?.lowercased() == "http"
            && ["localhost", "127.0.0.1", "::1"].contains(components.host?.lowercased())
    }

    private static func isSafeControlPlaneURL(_ url: URL) -> Bool {
        guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false),
            components.user == nil, components.password == nil,
            components.query == nil, components.fragment == nil, components.host != nil
        else { return false }
        if components.scheme?.lowercased() == "https" { return true }
        return components.scheme?.lowercased() == "http"
            && ["localhost", "127.0.0.1", "::1"].contains(components.host?.lowercased())
    }
}

public enum NodeConnectPermission: String, CaseIterable, Sendable {
    case discoverPrinters = "discover_printers"
    case print = "print"
    case monitorJobs = "monitor_jobs"
}

/// Consent captured locally. An empty printer selection is never interpreted
/// as access to every printer.
public struct NodeConnectConsent: Equatable, Sendable {
    public let printerIDs: [String]
    public let permissions: Set<NodeConnectPermission>

    public init(printerIDs: [String], permissions: Set<NodeConnectPermission>) throws {
        let normalized = printerIDs.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
        guard !normalized.isEmpty,
            normalized.allSatisfy({ !$0.isEmpty && $0.count <= 200 }),
            Set(normalized).count == normalized.count,
            !permissions.isEmpty
        else { throw NodeConnectApplicationLinkError.invalidCapability }
        self.printerIDs = normalized
        self.permissions = permissions
    }
}

/// Process-local replay fence. Server-side single-use/expiry enforcement is
/// still authoritative; this prevents duplicate OS open events from showing
/// two consent dialogs or launching two consumers concurrently.
public actor NodeConnectReplayGuard {
    private var inFlight = Set<String>()
    private var completed = Set<String>()

    public init() {}

    public func begin(_ link: NodeConnectApplicationLink) -> Bool {
        let fingerprint = link.capabilityFingerprint
        guard !inFlight.contains(fingerprint), !completed.contains(fingerprint) else { return false }
        inFlight.insert(fingerprint)
        return true
    }

    public func finish(_ link: NodeConnectApplicationLink, consumed: Bool) {
        let fingerprint = link.capabilityFingerprint
        inFlight.remove(fingerprint)
        if consumed { completed.insert(fingerprint) }
    }
}
