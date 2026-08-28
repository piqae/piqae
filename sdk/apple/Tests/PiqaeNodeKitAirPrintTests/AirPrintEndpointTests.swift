import PiqaeNodeKit
import XCTest
@testable import PiqaeNodeKitAirPrint

final class AirPrintEndpointTests: XCTestCase {
    func testCanonicalEndpointStripsCredentialsQueryAndFragment() throws {
        let source = try XCTUnwrap(
            URL(string: "IPPS://user:secret@Printer.Example:631/ipp/print?token=secret#local")
        )
        let canonical = try PiqaeAirPrintEndpoint.canonicalize(source)

        XCTAssertEqual(canonical.route.absoluteString, "ipps://printer.example/ipp/print")
        XCTAssertEqual(String(data: canonical.identityInput, encoding: .utf8), canonical.route.absoluteString)
        XCTAssertNil(canonical.route.user)
        XCTAssertNil(canonical.route.password)
        XCTAssertNil(canonical.route.query)
        XCTAssertNil(canonical.route.fragment)
    }

    func testCanonicalEndpointNormalizesDefaultPortAndTrailingSlash() throws {
        let source = try XCTUnwrap(URL(string: "ipp://Printer.Example:631/ipp/print///"))
        let canonical = try PiqaeAirPrintEndpoint.canonicalize(source)
        XCTAssertEqual(canonical.route.absoluteString, "ipp://printer.example/ipp/print")
    }

    func testCanonicalEndpointNormalizesEmptyPath() throws {
        let source = try XCTUnwrap(URL(string: "ipp://Printer.Example"))
        let canonical = try PiqaeAirPrintEndpoint.canonicalize(source)
        XCTAssertEqual(canonical.route.absoluteString, "ipp://printer.example/")
    }

    func testCanonicalEndpointRejectsNonIPPURL() throws {
        let source = try XCTUnwrap(URL(string: "https://printer.example/ipp/print"))
        XCTAssertThrowsError(try PiqaeAirPrintEndpoint.canonicalize(source))
    }

    #if os(iOS)
    func testAdapterRejectsInvalidKnownRouteInsteadOfDroppingIt() throws {
        let source = try XCTUnwrap(URL(string: "https://printer.example/ipp/print"))
        XCTAssertThrowsError(
            try PiqaeAirPrintAdapter(
                identityProvider: OpaqueIdentityProviderFake(),
                knownPrinterURLs: [source]
            )
        )
    }
    #endif
}

private actor OpaqueIdentityProviderFake: PiqaeOpaqueIdentityProvider {
    func deriveOpaqueID(namespace: String, canonicalIdentity: Data) -> String {
        "pid_0123456789abcdef"
    }
}
