import PiqaeNodeKit
import XCTest
@testable import PiqaeNode

@MainActor
final class StandaloneNodeStoreTests: XCTestCase {
    func testOnboardingUsesPrivacySafeBoundedDefaultAndPersistsExplicitDetails() throws {
        let suite = "com.piqae.tests.store.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defaults.removePersistentDomain(forName: suite)
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = StandaloneNodeStore(defaults: defaults)

        XCTAssertFalse(store.isConfigured)
        XCTAssertTrue(store.load().name.contains("Piqae Node"))
        let identity = try store.save(
            StandaloneNodeSettings(
                name: "Kitchen iPad",
                site: "Central",
                location: "Pass",
                labels: ["receipts", "labels"]
            )
        )

        XCTAssertTrue(store.isConfigured)
        XCTAssertEqual(identity.displayName, "Kitchen iPad")
        XCTAssertEqual(store.load().labels, ["receipts", "labels"])
    }

    func testPrinterURLPersistenceRemovesCredentialsQueryAndFragment() throws {
        let url = try XCTUnwrap(URL(string: "ipps://user:secret@printer.local/ipp/print?token=no#part"))
        let safe = try StandaloneNodeStore.safePrinterURL(url)

        XCTAssertNil(safe.user)
        XCTAssertNil(safe.password)
        XCTAssertNil(safe.query)
        XCTAssertNil(safe.fragment)
        XCTAssertEqual(safe.absoluteString, "ipps://printer.local/ipp/print")
    }

    func testPrinterURLPersistenceRejectsWebAndFileRoutes() throws {
        XCTAssertThrowsError(try StandaloneNodeStore.safePrinterURL(
            XCTUnwrap(URL(string: "https://printer.local"))
        ))
        XCTAssertThrowsError(try StandaloneNodeStore.safePrinterURL(
            XCTUnwrap(URL(string: "file:///tmp/printer"))
        ))
    }

    func testIOSCollisionPolicyNeverCreatesAnIsolatedQueueWhenAttachmentIsRequired() throws {
        let identity = try PiqaeNodeIdentityConfiguration(displayName: "Embedded POS")
        let policy = try PiqaeConnectionPolicy.integratorManaged(
            allowedAuthorityOrigins: [XCTUnwrap(URL(string: "https://api.piqae.com"))]
        )
        let requireInstalled = try PiqaeHostConfiguration(
            product: .embedded,
            applicationID: "com.example.pos",
            identity: identity,
            installedHostPolicy: .requireInstalled,
            connectionPolicy: policy
        )
        let isolated = try PiqaeHostConfiguration(
            product: .embedded,
            applicationID: "com.example.pos",
            identity: identity,
            installedHostPolicy: .isolatedApplication,
            connectionPolicy: policy
        )

        XCTAssertEqual(requireInstalled.effectiveStartupMode, .attach)
        XCTAssertFalse(requireInstalled.allowsEmbeddedFallback)
        XCTAssertEqual(isolated.effectiveStartupMode, .embedded)
        XCTAssertFalse(isolated.allowsEmbeddedFallback)
    }
}
