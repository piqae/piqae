import Foundation

#if os(iOS)
import UIKit
#elseif os(macOS)
import AppKit
#endif

public enum PiqaeNodeHostProduct: String, Codable, Sendable {
    /// A general-purpose Piqae node whose operator manages its connections.
    case standalone
    /// A node runtime hosted by another application.
    case embedded
}

public enum PiqaeInstalledHostPolicy: String, Codable, Sendable {
    /// Attach to an approved installed desktop node. If none exists, an
    /// isolated app-scoped runtime may be created. Authentication or consent
    /// failures still fail closed.
    case preferInstalled = "prefer_installed"
    /// Require an approved installed desktop node and never create a second
    /// queue as fallback.
    case requireInstalled = "require_installed"
    /// Deliberately create an application-scoped installation and queue.
    case isolatedApplication = "isolated_application"
}

public enum PiqaeConnectionManagement: String, Codable, Sendable {
    /// The standalone node UI lets its operator add and remove connections.
    case userManaged = "user_managed"
    /// The embedding application or its backend supplies invitations.
    case hostManaged = "host_managed"
}

public struct PiqaeNodeIdentityConfiguration: Codable, Equatable, Sendable {
    public let displayName: String
    public let site: String?
    public let location: String?
    public let labels: [String]

    public init(
        displayName: String,
        site: String? = nil,
        location: String? = nil,
        labels: [String] = []
    ) throws {
        self.displayName = try Self.required(displayName, field: "Node name", maximum: 120)
        self.site = try Self.optional(site, field: "Site", maximum: 120)
        self.location = try Self.optional(location, field: "Location", maximum: 120)
        guard labels.count <= 16 else {
            throw PiqaeNodeError.invalidConfiguration("A node can have at most 16 labels.")
        }
        var unique = Set<String>()
        self.labels = try labels.map { value in
            let label = try Self.required(value, field: "Label", maximum: 64)
            guard unique.insert(label).inserted else {
                throw PiqaeNodeError.invalidConfiguration("Node labels must be unique.")
            }
            return label
        }
    }

    private static func required(_ value: String, field: String, maximum: Int) throws -> String {
        let value = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty, value.utf8.count <= maximum else {
            throw PiqaeNodeError.invalidConfiguration(
                "\(field) must contain 1 to \(maximum) UTF-8 bytes."
            )
        }
        return value
    }

    private static func optional(_ value: String?, field: String, maximum: Int) throws -> String? {
        guard let value else { return nil }
        let value = value.trimmingCharacters(in: .whitespacesAndNewlines)
        if value.isEmpty { return nil }
        guard value.utf8.count <= maximum else {
            throw PiqaeNodeError.invalidConfiguration(
                "\(field) must contain at most \(maximum) UTF-8 bytes."
            )
        }
        return value
    }

    enum CodingKeys: String, CodingKey {
        case displayName = "display_name"
        case site, location, labels
    }

    public init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        try self.init(
            displayName: values.decode(String.self, forKey: .displayName),
            site: values.decodeIfPresent(String.self, forKey: .site),
            location: values.decodeIfPresent(String.self, forKey: .location),
            labels: values.decodeIfPresent([String].self, forKey: .labels) ?? []
        )
    }
}

public struct PiqaeConnectionPolicy: Codable, Equatable, Sendable {
    public let management: PiqaeConnectionManagement
    /// This is explicit contract truth rather than a licensing switch. Both
    /// standalone and embedded products can safely host many connectors.
    public let allowsMultiple: Bool
    /// Exact HTTPS origins. Empty means a user-managed UI may explicitly pick
    /// any otherwise-valid HTTPS authority.
    public let allowedAuthorityOrigins: [URL]

    public init(
        management: PiqaeConnectionManagement,
        allowsMultiple: Bool = true,
        allowedAuthorityOrigins: [URL] = []
    ) throws {
        var normalized: [URL] = []
        var seen = Set<String>()
        guard allowedAuthorityOrigins.count <= 32 else {
            throw PiqaeNodeError.invalidConfiguration(
                "A connection policy can allow at most 32 authority origins."
            )
        }
        for origin in allowedAuthorityOrigins {
            let origin = try Self.exactHTTPSOrigin(origin)
            if seen.insert(origin.absoluteString).inserted { normalized.append(origin) }
        }
        if management == .hostManaged, normalized.isEmpty {
            throw PiqaeNodeError.invalidConfiguration(
                "Host-managed connections require at least one pinned HTTPS authority origin."
            )
        }
        self.management = management
        self.allowsMultiple = allowsMultiple
        self.allowedAuthorityOrigins = normalized
    }

    public static var standaloneUserManaged: PiqaeConnectionPolicy {
        PiqaeConnectionPolicy(
            validatedManagement: .userManaged,
            allowsMultiple: true,
            allowedAuthorityOrigins: []
        )
    }

    public static func integratorManaged(
        allowedAuthorityOrigins: [URL],
        allowsMultiple: Bool = true
    ) throws -> PiqaeConnectionPolicy {
        try PiqaeConnectionPolicy(
            management: .hostManaged,
            allowsMultiple: allowsMultiple,
            allowedAuthorityOrigins: allowedAuthorityOrigins
        )
    }

    public func validateAuthority(_ authorityURL: URL) throws {
        let origin = try Self.exactHTTPSOrigin(authorityURL)
        guard management == .userManaged || !allowedAuthorityOrigins.isEmpty else {
            throw PiqaeNodeError.invalidConfiguration(
                "The embedding host did not configure a connection authority."
            )
        }
        guard allowedAuthorityOrigins.isEmpty || allowedAuthorityOrigins.contains(origin) else {
            throw PiqaeNodeError.invalidConfiguration(
                "The connection authority is outside this host's pinned policy."
            )
        }
    }

    private static func exactHTTPSOrigin(_ url: URL) throws -> URL {
        guard
            url.scheme?.lowercased() == "https",
            let host = url.host?.lowercased(), !host.isEmpty,
            url.user == nil, url.password == nil,
            url.path.isEmpty || url.path == "/",
            url.query == nil, url.fragment == nil
        else {
            throw PiqaeNodeError.invalidConfiguration(
                "Connection policies require an exact HTTPS authority origin."
            )
        }
        var components = URLComponents()
        components.scheme = "https"
        components.host = host
        components.port = url.port
        guard let normalized = components.url else {
            throw PiqaeNodeError.invalidConfiguration("The connection authority is invalid.")
        }
        return normalized
    }

    private init(
        validatedManagement management: PiqaeConnectionManagement,
        allowsMultiple: Bool,
        allowedAuthorityOrigins: [URL]
    ) {
        self.management = management
        self.allowsMultiple = allowsMultiple
        self.allowedAuthorityOrigins = allowedAuthorityOrigins
    }

    enum CodingKeys: String, CodingKey {
        case management
        case allowsMultiple = "allows_multiple"
        case allowedAuthorityOrigins = "allowed_authority_origins"
    }

    public init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        try self.init(
            management: values.decode(PiqaeConnectionManagement.self, forKey: .management),
            allowsMultiple: values.decode(Bool.self, forKey: .allowsMultiple),
            allowedAuthorityOrigins: values.decode([URL].self, forKey: .allowedAuthorityOrigins)
        )
    }
}

public struct PiqaeHostConfiguration: Codable, Equatable, Sendable {
    public let contract: UInt8
    public let product: PiqaeNodeHostProduct
    public let applicationID: String
    public let identity: PiqaeNodeIdentityConfiguration
    public let installedHostPolicy: PiqaeInstalledHostPolicy
    public let connectionPolicy: PiqaeConnectionPolicy

    public init(
        product: PiqaeNodeHostProduct,
        applicationID: String,
        identity: PiqaeNodeIdentityConfiguration,
        installedHostPolicy: PiqaeInstalledHostPolicy,
        connectionPolicy: PiqaeConnectionPolicy
    ) throws {
        let applicationID = applicationID.trimmingCharacters(in: .whitespacesAndNewlines)
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: ".-"))
        guard
            applicationID.utf8.count >= 3, applicationID.utf8.count <= 255,
            applicationID.unicodeScalars.allSatisfy({ allowed.contains($0) })
        else {
            throw PiqaeNodeError.invalidConfiguration(
                "Application IDs must be bounded reverse-DNS identifiers."
            )
        }
        contract = 1
        self.product = product
        self.applicationID = applicationID
        self.identity = identity
        self.installedHostPolicy = installedHostPolicy
        self.connectionPolicy = connectionPolicy
    }

    public var effectiveStartupMode: PiqaeNodeStartupMode {
        #if os(iOS)
        // App sandboxing prevents an iOS application from becoming or using a
        // persistent cross-application daemon.
        .embedded
        #else
        switch installedHostPolicy {
        case .preferInstalled: .automatic
        case .requireInstalled: .attach
        case .isolatedApplication: .embedded
        }
        #endif
    }

    public var allowsEmbeddedFallback: Bool {
        #if os(iOS)
        true
        #else
        installedHostPolicy == .preferInstalled
        #endif
    }

    enum CodingKeys: String, CodingKey {
        case contract, product, identity
        case applicationID = "application_id"
        case installedHostPolicy = "installed_host_policy"
        case connectionPolicy = "connection_policy"
    }

    public init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let contract = try values.decode(UInt8.self, forKey: .contract)
        guard contract == 1 else {
            throw PiqaeNodeError.invalidConfiguration("Unsupported host configuration contract.")
        }
        try self.init(
            product: values.decode(PiqaeNodeHostProduct.self, forKey: .product),
            applicationID: values.decode(String.self, forKey: .applicationID),
            identity: values.decode(PiqaeNodeIdentityConfiguration.self, forKey: .identity),
            installedHostPolicy: values.decode(
                PiqaeInstalledHostPolicy.self,
                forKey: .installedHostPolicy
            ),
            connectionPolicy: values.decode(PiqaeConnectionPolicy.self, forKey: .connectionPolicy)
        )
    }
}

public enum PiqaeLocalNodeNameSuggestion {
    /// Returns an operator-visible suggestion only. It never reads a login,
    /// contacts, postal address, advertising identifier, or serial number.
    public static func make(productName: String = "Piqae Node") -> String {
        #if os(iOS)
        // On iOS 16+, UIDevice.name is generic unless Apple grants a special
        // entitlement. Piqae intentionally does not request that entitlement.
        let model = UIDevice.current.userInterfaceIdiom == .pad ? "iPad" : "iPhone"
        return "\(productName) on this \(model)"
        #elseif os(macOS)
        if let name = Host.current().localizedName?
            .trimmingCharacters(in: .whitespacesAndNewlines), !name.isEmpty
        {
            return String(name.prefix(120))
        }
        return "\(productName) on this Mac"
        #else
        return productName
        #endif
    }
}
