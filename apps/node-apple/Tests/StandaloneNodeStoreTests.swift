import PiqaeNodeKit
import UIKit
import XCTest
@testable import PiqaeNode

@MainActor
final class StandaloneNodeStoreTests: XCTestCase {
    func testStandaloneBundleContainsAValidPrivacyManifest() throws {
        let manifestURL = try XCTUnwrap(
            Bundle.main.url(forResource: "PrivacyInfo", withExtension: "xcprivacy")
        )
        let data = try Data(contentsOf: manifestURL)
        var format = PropertyListSerialization.PropertyListFormat.xml
        let value = try XCTUnwrap(
            PropertyListSerialization.propertyList(
                from: data,
                options: [],
                format: &format
            ) as? [String: Any]
        )

        XCTAssertEqual(value["NSPrivacyTracking"] as? Bool, false)
        XCTAssertFalse((value["NSPrivacyCollectedDataTypes"] as? [[String: Any]] ?? []).isEmpty)
        let accessed = value["NSPrivacyAccessedAPITypes"] as? [[String: Any]] ?? []
        XCTAssertTrue(accessed.contains {
            $0["NSPrivacyAccessedAPIType"] as? String
                == "NSPrivacyAccessedAPICategoryUserDefaults"
        })
    }

    func testColdLaunchWakeIsBufferedUntilTheModelStarts() async {
        let delegate = PiqaeNodeAppDelegate(
            wakeDeadlineSeconds: 1,
            maximumPendingHints: 4
        )
        let model = FakeStandaloneWakeHandler()
        let completed = expectation(description: "wake completion")
        var result: UIBackgroundFetchResult?

        delegate.application(
            UIApplication.shared,
            didReceiveRemoteNotification: wakePayload("cold-launch"),
            fetchCompletionHandler: {
                result = $0
                completed.fulfill()
            }
        )
        await Task.yield()
        XCTAssertEqual(model.handledCollapseIDs, [])

        delegate.install(model: model)
        delegate.modelDidStart()
        await fulfillment(of: [completed], timeout: 1)

        XCTAssertEqual(model.handledCollapseIDs, ["cold-launch"])
        XCTAssertEqual(result, .newData)
    }

    func testDuplicateColdLaunchWakeHintsCoalesceButCompleteEveryHandler() async {
        let delegate = PiqaeNodeAppDelegate(
            wakeDeadlineSeconds: 1,
            maximumPendingHints: 4
        )
        let model = FakeStandaloneWakeHandler()
        let completed = expectation(description: "duplicate wake completions")
        completed.expectedFulfillmentCount = 2

        for _ in 0 ..< 2 {
            delegate.application(
                UIApplication.shared,
                didReceiveRemoteNotification: wakePayload("same-generation"),
                fetchCompletionHandler: { _ in completed.fulfill() }
            )
        }
        delegate.install(model: model)
        delegate.modelDidStart()
        await fulfillment(of: [completed], timeout: 1)

        XCTAssertEqual(model.handledCollapseIDs, ["same-generation"])
    }

    func testDuplicateWakeCompletionHandlersAreBounded() async {
        let delegate = PiqaeNodeAppDelegate(
            wakeDeadlineSeconds: 1,
            maximumPendingHints: 4,
            maximumCompletionsPerHint: 2
        )
        let model = FakeStandaloneWakeHandler()
        let completed = expectation(description: "bounded duplicate completions")
        completed.expectedFulfillmentCount = 3
        var results: [UIBackgroundFetchResult] = []

        for _ in 0 ..< 3 {
            delegate.application(
                UIApplication.shared,
                didReceiveRemoteNotification: wakePayload("same-generation"),
                fetchCompletionHandler: {
                    results.append($0)
                    completed.fulfill()
                }
            )
        }
        XCTAssertEqual(results, [.noData])
        delegate.install(model: model)
        delegate.modelDidStart()
        await fulfillment(of: [completed], timeout: 1)

        XCTAssertEqual(model.handledCollapseIDs, ["same-generation"])
        XCTAssertEqual(results.filter { $0 == .newData }.count, 2)
    }

    func testColdLaunchWakeDeadlineCompletesWithoutRetainingTheHint() async {
        let delegate = PiqaeNodeAppDelegate(
            wakeDeadlineSeconds: 0.01,
            maximumPendingHints: 1
        )
        let completed = expectation(description: "expired wake completion")
        var result: UIBackgroundFetchResult?
        delegate.application(
            UIApplication.shared,
            didReceiveRemoteNotification: wakePayload("expires"),
            fetchCompletionHandler: {
                result = $0
                completed.fulfill()
            }
        )

        await fulfillment(of: [completed], timeout: 1)
        XCTAssertEqual(result, .noData)

        let model = FakeStandaloneWakeHandler()
        delegate.install(model: model)
        delegate.modelDidStart()
        await Task.yield()
        XCTAssertEqual(model.handledCollapseIDs, [])
    }

    func testWakeHintContainsOnlyOpaqueReconciliationMetadata() {
        let valid: [AnyHashable: Any] = [
            "aps": ["content-available": 1],
            "piqae_wake_hint": "inventory-changed",
        ]
        XCTAssertEqual(
            StandaloneWakeHintEnvelope.collapseID(from: valid),
            "inventory-changed"
        )
        XCTAssertNil(StandaloneWakeHintEnvelope.collapseID(from: [
            "aps": ["content-available": 1],
            "piqae_wake_hint": "inventory-changed",
            "job_id": "job_secret",
        ]))
        XCTAssertNil(StandaloneWakeHintEnvelope.collapseID(from: [
            "aps": ["content-available": 1, "alert": "Print this"],
            "piqae_wake_hint": "inventory-changed",
        ]))
        XCTAssertNil(StandaloneWakeHintEnvelope.collapseID(from: [
            "aps": ["content-available": 1],
            "piqae_wake_hint": "inventory\u{0000}changed",
        ]))
    }

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
        XCTAssertEqual(store.identityRevision, 1)

        let renamed = try PiqaeNodeIdentityConfiguration(
            displayName: "Kitchen pass iPad",
            site: "Central",
            location: "Pass",
            labels: ["receipts"]
        )
        store.save(renamed, revision: 7)
        XCTAssertEqual(store.identityRevision, 7)
        XCTAssertEqual(store.load().name, "Kitchen pass iPad")
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

    private func wakePayload(_ collapseID: String) -> [AnyHashable: Any] {
        [
            "aps": ["content-available": 1],
            "piqae_wake_hint": collapseID,
        ]
    }
}

@MainActor
private final class FakeStandaloneWakeHandler: StandaloneWakeHandling {
    private(set) var handledCollapseIDs: [String] = []

    func handleBackgroundPush(collapseID: String) async -> Bool {
        handledCollapseIDs.append(collapseID)
        return true
    }
}
