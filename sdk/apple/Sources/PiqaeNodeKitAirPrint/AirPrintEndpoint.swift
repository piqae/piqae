import Foundation
import PiqaeNodeKit

enum PiqaeAirPrintEndpoint {
    /// Returns a credential-free route URL and the transient canonical bytes
    /// used as input to the shared runtime's installation-keyed identity
    /// primitive. Callers must never persist or publish `identityInput`.
    static func canonicalize(_ source: URL) throws -> (route: URL, identityInput: Data) {
        guard var components = URLComponents(url: source, resolvingAgainstBaseURL: false) else {
            throw invalidEndpoint()
        }
        let scheme = components.scheme?.lowercased()
        guard let scheme, ["ipp", "ipps"].contains(scheme), components.host != nil else {
            throw invalidEndpoint()
        }

        components.scheme = scheme
        components.host = components.host?.lowercased()
        components.user = nil
        components.password = nil
        components.query = nil
        components.fragment = nil
        if components.percentEncodedPath.isEmpty {
            components.percentEncodedPath = "/"
        }
        guard let route = components.url else { throw invalidEndpoint() }
        return (route, Data(route.absoluteString.utf8))
    }

    private static func invalidEndpoint() -> PiqaeNodeError {
        PiqaeNodeError.invalidConfiguration(
            "AirPrint printers must use a valid ipp or ipps URL with a host."
        )
    }
}
