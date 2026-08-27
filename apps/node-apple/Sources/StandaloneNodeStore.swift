import Foundation
import PiqaeNodeKit

struct StandaloneNodeSettings: Equatable, Sendable {
    var name: String
    var site: String
    var location: String
    var labels: [String]
}

@MainActor
final class StandaloneNodeStore {
    private enum Key {
        static let configured = "standalone.configured"
        static let name = "standalone.name"
        static let site = "standalone.site"
        static let location = "standalone.location"
        static let labels = "standalone.labels"
        static let identityRevision = "standalone.identity.revision"
        static let printers = "standalone.airprint.urls"
    }

    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    var isConfigured: Bool { defaults.bool(forKey: Key.configured) }
    var identityRevision: UInt64 {
        let value = defaults.object(forKey: Key.identityRevision) as? NSNumber
        return max(1, value?.uint64Value ?? 1)
    }

    func load() -> StandaloneNodeSettings {
        StandaloneNodeSettings(
            name: defaults.string(forKey: Key.name) ?? PiqaeLocalNodeNameSuggestion.make(),
            site: defaults.string(forKey: Key.site) ?? "",
            location: defaults.string(forKey: Key.location) ?? "",
            labels: defaults.stringArray(forKey: Key.labels) ?? []
        )
    }

    func save(_ settings: StandaloneNodeSettings) throws -> PiqaeNodeIdentityConfiguration {
        let identity = try PiqaeNodeIdentityConfiguration(
            displayName: settings.name,
            site: settings.site,
            location: settings.location,
            labels: settings.labels
        )
        defaults.set(identity.displayName, forKey: Key.name)
        defaults.set(identity.site, forKey: Key.site)
        defaults.set(identity.location, forKey: Key.location)
        defaults.set(identity.labels, forKey: Key.labels)
        defaults.set(true, forKey: Key.configured)
        return identity
    }

    func save(_ identity: PiqaeNodeIdentityConfiguration, revision: UInt64) {
        defaults.set(identity.displayName, forKey: Key.name)
        defaults.set(identity.site, forKey: Key.site)
        defaults.set(identity.location, forKey: Key.location)
        defaults.set(identity.labels, forKey: Key.labels)
        defaults.set(max(1, revision), forKey: Key.identityRevision)
        defaults.set(true, forKey: Key.configured)
    }

    func saveIdentityRevision(_ revision: UInt64) {
        defaults.set(max(1, revision), forKey: Key.identityRevision)
    }

    func printerURLs() -> [URL] {
        (defaults.stringArray(forKey: Key.printers) ?? []).compactMap(URL.init(string:))
    }

    func addPrinterURL(_ url: URL) throws {
        let safe = try Self.safePrinterURL(url)
        var values = Set(defaults.stringArray(forKey: Key.printers) ?? [])
        values.insert(safe.absoluteString)
        defaults.set(values.sorted(), forKey: Key.printers)
    }

    static func safePrinterURL(_ url: URL) throws -> URL {
        guard var components = URLComponents(url: url, resolvingAgainstBaseURL: false),
            let scheme = components.scheme?.lowercased(), ["ipp", "ipps"].contains(scheme),
            components.host != nil
        else {
            throw PiqaeNodeError.invalidConfiguration("The selected printer route is invalid.")
        }
        components.user = nil
        components.password = nil
        components.query = nil
        components.fragment = nil
        guard let safe = components.url else {
            throw PiqaeNodeError.invalidConfiguration("The selected printer route is invalid.")
        }
        return safe
    }
}
