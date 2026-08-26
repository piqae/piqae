import Foundation
import XCTest
@testable import PiqaeNodeKit
import PiqaeNodeKitTesting

final class PiqaeNodeKitTests: XCTestCase {
    func testLinkedNativeRuntimeArtifactStartsWhenPresent() async throws {
        let applicationID = "com.piqae.tests.linked.\(UUID().uuidString.lowercased())"
        let stateDirectory = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/Piqae/embedded", isDirectory: true)
            .appendingPathComponent(applicationID, isDirectory: true)
        let runtime = PiqaeNativeRuntime(
            configuration: PiqaeNativeRuntimeConfiguration(
                applicationID: applicationID,
                dataDirectory: "linked-test",
                availability: .continuousWhileAwake,
                localOnly: true
            ),
            keyStore: PiqaeFixedHostKeyStore()
        )
        do {
            try await runtime.start()
        } catch PiqaeNativeRuntimeError.libraryUnavailable {
            throw XCTSkip("The source-only package intentionally has no linked native artifact.")
        }
        addTeardownBlock {
            try await runtime.stop()
            try? FileManager.default.removeItem(at: stateDirectory)
        }
        let opaque = try await runtime.deriveOpaqueID(
            namespace: "linked-test",
            canonicalIdentity: Data("printer-fixture".utf8)
        )
        XCTAssertTrue(opaque.hasPrefix("pid_"))
        try await runtime.stop()
    }

    func testNativeRuntimeBindsLifecycleAndOpaqueEvidenceWhenArtifactIsProvided() async throws {
        guard
            let path = ProcessInfo.processInfo.environment["PIQAE_NODE_FFI_LIBRARY"],
            FileManager.default.fileExists(atPath: path)
        else {
            throw XCTSkip("Set PIQAE_NODE_FFI_LIBRARY after building piqae-node-ffi.")
        }
        let applicationID = "com.piqae.tests.\(UUID().uuidString.lowercased())"
        let stateDirectory = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/Piqae/embedded", isDirectory: true)
            .appendingPathComponent(applicationID, isDirectory: true)
        let runtime = PiqaeNativeRuntime(
            configuration: PiqaeNativeRuntimeConfiguration(
                applicationID: applicationID,
                dataDirectory: "ffi-test-\(UUID().uuidString.lowercased())",
                availability: .backgroundOpportunistic,
                localOnly: true,
                libraryURL: URL(fileURLWithPath: path)
            ),
            keyStore: PiqaeFixedHostKeyStore()
        )
        addTeardownBlock {
            try await runtime.stop()
            try? FileManager.default.removeItem(at: stateDirectory)
        }
        try await runtime.start()
        let first = try await runtime.deriveOpaqueID(
            namespace: "airprint",
            canonicalIdentity: Data("ipps://printer.local/ipp/print".utf8)
        )
        let again = try await runtime.deriveOpaqueID(
            namespace: "airprint",
            canonicalIdentity: Data("ipps://printer.local/ipp/print".utf8)
        )
        let other = try await runtime.deriveOpaqueID(
            namespace: "ble",
            canonicalIdentity: Data("ipps://printer.local/ipp/print".utf8)
        )
        XCTAssertTrue(first.hasPrefix("pid_"))
        XCTAssertEqual(first, again)
        XCTAssertNotEqual(first, other)
        XCTAssertFalse(first.contains("printer.local"))
        try await runtime.report(.suspendImminent)
        try await runtime.stop()
    }
    func testLifecycleEventsUseSharedRuntimeWireNamesAndReporter() async throws {
        XCTAssertEqual(PiqaeHostLifecycleEvent.suspendImminent.rawValue, "suspend_imminent")
        XCTAssertEqual(PiqaeHostLifecycleEvent.networkConstrained.rawValue, "network_constrained")

        let reporter = PiqaeFakeLifecycleReporter()
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: "ins_apple_lifecycle")
                ),
                hostLifecycleReporter: reporter
            )
        )
        try await node.start()
        try await node.reportHostLifecycle(.enteredBackground)
        try await node.reportHostLifecycle(.suspendImminent)

        let events = await reporter.events
        let snapshot = await node.snapshot()
        XCTAssertEqual(events, [.enteredBackground, .suspendImminent])
        XCTAssertEqual(snapshot.phase, .suspended)
        await node.stop()
    }

    func testEmbeddedRuntimeOwnsLifecycleAndStopsWithFacade() async throws {
        let runtime = PiqaeFakeEmbeddedRuntime()
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: "ins_native_runtime_lifecycle")
                ),
                embeddedRuntime: runtime
            )
        )
        try await node.start()
        try await node.reportHostLifecycle(.enteredBackground)
        await node.stop()

        let startCount = await runtime.startCount
        let stopCount = await runtime.stopCount
        let events = await runtime.lifecycleEvents
        XCTAssertEqual(startCount, 1)
        XCTAssertEqual(stopCount, 1)
        XCTAssertEqual(events, [.enteredBackground])
    }

    func testRemoteNotificationRegistrationIsExplicitAndRedacted() async throws {
        let provider = PiqaeFakeRemoteNotificationProvider()
        let identity = PiqaeMemoryInstallationIdentityStore(
            id: .init(rawValue: "ins_apple_push")
        )
        let node = PiqaeNode(
            PiqaeNodeConfiguration(
                startupMode: .embedded,
                identityStore: identity,
                remoteNotificationProvider: provider
            )
        )
        try await node.start()
        let bytes = Data([0xde, 0xad, 0xbe, 0xef])
        try await node.remoteNotifications.register(
            deviceToken: bytes,
            environment: .development,
            bundleIdentifier: "com.example.print"
        )

        let registrations = await provider.registrations
        XCTAssertEqual(registrations.count, 1)
        XCTAssertEqual(registrations[0].installationID.rawValue, "ins_apple_push")
        XCTAssertEqual(registrations[0].token.description, "<redacted>")
        XCTAssertEqual(
            registrations[0].token.withBytes { $0 },
            bytes
        )
        await node.stop()
    }

    func testRemoteNotificationRegistrationIsOptIn() async throws {
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: "ins_apple_no_push")
                )
            )
        )
        try await node.start()
        await XCTAssertThrowsErrorAsync(
            try await node.remoteNotifications.register(
                deviceToken: Data([1]),
                environment: .production,
                bundleIdentifier: "com.example.print"
            )
        )
        XCTAssertEqual(
            node.remoteNotifications.availability,
            .opportunisticWhileInstalled
        )
        await node.stop()
    }
    func testLocalOnlyEmbeddedNodeDiscoversFakePrinter() async throws {
        let printer = PiqaeFakePrinterAdapter.printer()
        let adapter = PiqaeFakePrinterAdapter(printers: [printer])
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: "ins_local_test")
                ),
                printerAdapters: [adapter]
            )
        )
        try await node.start()
        defer { Task { await node.stop() } }

        let snapshot = await node.snapshot()
        XCTAssertEqual(snapshot.hostMode, .embeddedApplication)
        XCTAssertEqual(snapshot.phase, .ready)
        XCTAssertEqual(snapshot.printers, [printer])
        XCTAssertEqual(snapshot.connections, [.localOnly])
        let connections = try await node.connections.list()
        XCTAssertEqual(connections, [.localOnly])
        let descriptors = try await node.printers.adapters()
        XCTAssertEqual(descriptors, [adapter.descriptor])
        XCTAssertEqual(snapshot.printers.first?.adapterFingerprint?.adapterVersion, "1")
    }

    func testExplicitCloudConnectionReplacesLocalOnlySentinel() async throws {
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: "ins_explicit_connect")
                )
            )
        )
        try await node.start()
        defer { Task { await node.stop() } }
        let connection = PiqaeConnection(
            id: .init(rawValue: "ncon_explicit"),
            authorityURL: URL(string: "https://api.piqae.com"),
            workspaceName: "Explicit workspace",
            state: .connected
        )
        let cloud = try PiqaeCloudConfiguration(
            authorityURL: XCTUnwrap(URL(string: "https://api.piqae.com")),
            invitation: PiqaeSensitiveString("invitation"),
            provider: PiqaeFakeEnrollmentProvider(connection: connection)
        )

        _ = try await node.connections.connect(cloud)
        let connections = try await node.connections.list()
        XCTAssertEqual(connections, [connection])
    }

    func testAutomaticDesktopModeAttachesBeforeStartingEmbeddedRuntime() async throws {
        let remote = attachedSnapshot()
        let ipc = PiqaeFakeInstalledNodeIPC(protocolVersion: 1, snapshot: remote)
        let embeddedAdapter = PiqaeFakePrinterAdapter(
            printers: [PiqaeFakePrinterAdapter.printer(id: "must_not_be_used")]
        )
        let node = PiqaeNode(
            .localOnly(
                startupMode: .automatic,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: "ins_unused")
                ),
                installedNodeIPC: ipc,
                printerAdapters: [embeddedAdapter]
            )
        )
        try await node.start()
        defer { Task { await node.stop() } }

        let snapshot = await node.snapshot()
        XCTAssertEqual(snapshot.hostMode, .attachedClient)
        XCTAssertEqual(snapshot.installationID, remote.installationID)
        XCTAssertEqual(snapshot.printers, remote.printers)
    }

    func testIncompatibleInstalledNodeFailsClosedInsteadOfCreatingDuplicate() async throws {
        let ipc = PiqaeFakeInstalledNodeIPC(protocolVersion: 99, snapshot: attachedSnapshot())
        let node = PiqaeNode(
            .localOnly(
                startupMode: .automatic,
                identityStore: PiqaeMemoryInstallationIdentityStore(),
                installedNodeIPC: ipc,
                printerAdapters: []
            )
        )

        do {
            try await node.start()
            XCTFail("Expected protocol rejection")
        } catch let error as PiqaeNodeError {
            XCTAssertEqual(
                error,
                .incompatibleInstalledNode(found: 99, supported: 1 ... 1)
            )
        }
        let degradedSnapshot = await node.snapshot()
        XCTAssertEqual(degradedSnapshot.phase, .degraded)
    }

    func testCloudEnrollmentIsAdditiveToEmbeddedIdentity() async throws {
        let connection = PiqaeConnection(
            id: .init(rawValue: "ncon_cloud"),
            authorityURL: URL(string: "https://api.piqae.com"),
            workspaceName: "Managed shop",
            state: .connected
        )
        let provider = PiqaeFakeEnrollmentProvider(connection: connection)
        let invitation = try PiqaeSensitiveString("one-use-invitation")
        let cloud = try PiqaeCloudConfiguration(
            authorityURL: XCTUnwrap(URL(string: "https://api.piqae.com")),
            invitation: invitation,
            provider: provider
        )
        let node = PiqaeNode(
            PiqaeNodeConfiguration(
                startupMode: .embedded,
                connectivity: .cloud(cloud),
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: "ins_cloud_test")
                )
            )
        )
        try await node.start()
        defer { Task { await node.stop() } }

        let connections = try await node.connections.list()
        let enrollmentRequestCount = await provider.requestCount()
        XCTAssertEqual(connections, [connection])
        XCTAssertEqual(enrollmentRequestCount, 1)
        XCTAssertEqual(invitation.description, "<redacted>")
        XCTAssertEqual(invitation.debugDescription, "<redacted>")
    }

    func testFailedCloudStartReleasesEmbeddedOwnershipAndCanRetry() async throws {
        let identity = PiqaeMemoryInstallationIdentityStore(
            id: .init(rawValue: "ins_failed_cloud_start")
        )
        let cloud = try PiqaeCloudConfiguration(
            authorityURL: XCTUnwrap(URL(string: "https://api.piqae.com")),
            invitation: PiqaeSensitiveString("single-use-invitation"),
            provider: RejectingEnrollmentProvider()
        )
        let failed = PiqaeNode(
            PiqaeNodeConfiguration(
                startupMode: .embedded,
                connectivity: .cloud(cloud),
                identityStore: identity
            )
        )

        await XCTAssertThrowsErrorAsync(try await failed.start())
        await XCTAssertThrowsErrorAsync(try await failed.start()) { error in
            XCTAssertNotEqual(error as? PiqaeNodeError, .alreadyStarted)
        }

        let replacement = PiqaeNode(.localOnly(startupMode: .embedded, identityStore: identity))
        try await replacement.start()
        await replacement.stop()
    }

    func testCloudAuthorityRejectsCredentialsAndQuerySecrets() throws {
        let invitation = try PiqaeSensitiveString("invitation")
        let provider = RejectingEnrollmentProvider()
        XCTAssertThrowsError(
            try PiqaeCloudConfiguration(
                authorityURL: XCTUnwrap(URL(string: "https://api.piqae.com?token=secret")),
                invitation: invitation,
                provider: provider
            )
        )
        XCTAssertThrowsError(
            try PiqaeCloudConfiguration(
                authorityURL: XCTUnwrap(URL(string: "https://user:pass@api.piqae.com")),
                invitation: invitation,
                provider: provider
            )
        )
    }

    func testEmbeddedSubmissionCannotBypassDurableRuntime() async throws {
        let printer = PiqaeFakePrinterAdapter.printer()
        let adapter = PiqaeFakePrinterAdapter(printers: [printer])
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: "ins_idempotency_test")
                ),
                printerAdapters: [adapter]
            )
        )
        try await node.start()
        defer { Task { await node.stop() } }
        let request = try PiqaePrintRequest(
            printerID: printer.id,
            title: "Virtual receipt",
            content: .pdf(Data("%PDF-fake".utf8)),
            idempotencyKey: "order-42-receipt"
        )

        for _ in 0..<2 {
            do {
                _ = try await node.jobs.submit(request)
                XCTFail("embedded submission must fail closed until the durable executor ABI is bound")
            } catch let error as PiqaeNodeError {
                guard case .unsupportedOperation = error else {
                    XCTFail("unexpected error: \(error)")
                    continue
                }
            }
        }
        let submissionCount = await adapter.submissionCount()
        XCTAssertEqual(submissionCount, 0)
    }

    func testBackgroundSubmissionDefersWithoutDurablePayload() async throws {
        let printer = PiqaeFakePrinterAdapter.printer()
        let adapter = PiqaeFakePrinterAdapter(printers: [printer])
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                availability: .backgroundOpportunistic,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: "ins_background_test")
                ),
                printerAdapters: [adapter]
            )
        )
        try await node.start()
        defer { Task { await node.stop() } }
        await node.updateExecutionContext(
            .init(phase: .background, source: .backgroundPush, remainingSeconds: 25)
        )
        let request = try PiqaePrintRequest(
            printerID: printer.id,
            title: "Deferred virtual receipt",
            content: .pdf(Data("%PDF-fake".utf8)),
            idempotencyKey: "background-receipt"
        )

        await XCTAssertThrowsErrorAsync(try await node.jobs.submit(request)) { error in
            XCTAssertEqual(error as? PiqaeNodeError, .backgroundExecutionUnavailable)
        }
        let submissionCount = await adapter.submissionCount()
        XCTAssertEqual(submissionCount, 0)
    }

    func testWakeHintOnlyReconcilesAndNeverSubmits() async throws {
        let adapter = PiqaeFakePrinterAdapter(
            printers: [PiqaeFakePrinterAdapter.printer()]
        )
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                availability: .backgroundOpportunistic,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: "ins_wake_test")
                ),
                printerAdapters: [adapter]
            )
        )
        try await node.start()
        defer { Task { await node.stop() } }
        let hint = try PiqaeWakeHint(collapseID: "work-available", source: .backgroundPush)

        let result = await node.handleWakeHint(
            hint,
            context: .init(phase: .background, source: .backgroundPush, remainingSeconds: 30)
        )
        XCTAssertEqual(result, .reconciledWithoutLeasing)
        let submissionCount = await adapter.submissionCount()
        XCTAssertEqual(submissionCount, 0)
    }

    func testProcessRegistryPreventsTwoEmbeddedOwners() async throws {
        let sharedIdentity = PiqaeMemoryInstallationIdentityStore(
            id: .init(rawValue: "ins_collision_test")
        )
        let first = PiqaeNode(.localOnly(startupMode: .embedded, identityStore: sharedIdentity))
        let second = PiqaeNode(.localOnly(startupMode: .embedded, identityStore: sharedIdentity))
        try await first.start()
        defer { Task { await first.stop(); await second.stop() } }

        await XCTAssertThrowsErrorAsync(try await second.start()) { error in
            XCTAssertEqual(error as? PiqaeNodeError, .nodeAlreadyRunning)
        }
        await first.stop()
        try await second.start()
        let restartedSnapshot = await second.snapshot()
        XCTAssertEqual(restartedSnapshot.phase, .ready)
    }

    func testAdmissionPolicyOnlyFinishesWorkThatMayAlreadyHaveCrossedBoundary() {
        let policy = PiqaeBackgroundAdmissionPolicy(safetyMarginSeconds: 5)
        let shortBudget = PiqaeExecutionContext(
            phase: .background,
            source: .backgroundPush,
            remainingSeconds: 3
        )
        XCTAssertEqual(
            policy.evaluate(
                .init(
                    payloadIsDurable: true,
                    nativeBoundaryMayHaveBeenCrossed: true,
                    estimatedSecondsToNativeAcceptance: 20
                ),
                context: shortBudget,
                availability: .backgroundOpportunistic
            ),
            .finishAlreadyStarted
        )
        XCTAssertEqual(
            policy.evaluate(
                .init(payloadIsDurable: true, estimatedSecondsToNativeAcceptance: 20),
                context: shortBudget,
                availability: .backgroundOpportunistic
            ),
            .deferUntilForeground(
                reason: "The remaining execution budget is too short for a safe native handoff."
            )
        )
    }

    private func attachedSnapshot() -> PiqaeNodeSnapshot {
        let printer = PiqaeFakePrinterAdapter.printer(id: "prn_attached")
        return PiqaeNodeSnapshot(
            installationID: .init(rawValue: "ins_desktop_node"),
            hostMode: .userAgent,
            availability: .continuousWhileAwake,
            phase: .ready,
            connections: [.localOnly],
            printers: [printer],
            lastUpdatedAt: Date(timeIntervalSince1970: 1_700_000_000)
        )
    }
}

private actor RejectingEnrollmentProvider: PiqaeCloudEnrollmentProvider {
    struct Rejected: Error {}

    func enroll(_ request: PiqaeEnrollmentRequest) async throws -> PiqaeConnection {
        throw Rejected()
    }
}

private extension XCTestCase {
    func XCTAssertThrowsErrorAsync<T>(
        _ expression: @autoclosure () async throws -> T,
        _ handler: (Error) -> Void = { _ in }
    ) async {
        do {
            _ = try await expression()
            XCTFail("Expected expression to throw")
        } catch {
            handler(error)
        }
    }
}
