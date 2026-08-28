import CryptoKit
import Foundation
import XCTest
@testable import PiqaeNodeKit
import PiqaeNodeKitTesting

private enum LinkedRuntimeRequired: Error { case unavailable }

final class PiqaeNodeKitTests: XCTestCase {
    func testApplicationIdentifiersMatchSharedContractFixture() throws {
        let fixtureURL = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("sdk/contracts/fixtures/node-host-application-ids.json")
        let fixture = try JSONDecoder().decode(
            ApplicationIDFixture.self,
            from: Data(contentsOf: fixtureURL)
        )
        let identity = try PiqaeNodeIdentityConfiguration(displayName: "Fixture node")
        let policy = PiqaeConnectionPolicy.standaloneUserManaged

        for applicationID in fixture.valid {
            XCTAssertNoThrow(try PiqaeHostConfiguration(
                product: .standalone,
                applicationID: applicationID,
                identity: identity,
                installedHostPolicy: .isolatedApplication,
                connectionPolicy: policy
            ))
        }
        for applicationID in fixture.invalid {
            XCTAssertThrowsError(try PiqaeHostConfiguration(
                product: .standalone,
                applicationID: applicationID,
                identity: identity,
                installedHostPolicy: .isolatedApplication,
                connectionPolicy: policy
            ))
        }
    }

    func testPortableHostConfigurationIsBoundedAndSnakeCase() throws {
        let identity = try PiqaeNodeIdentityConfiguration(
            displayName: "Dispatch iPad",
            site: "Warehouse",
            location: "Desk 4",
            labels: ["shipping", "backup"]
        )
        let policy = try PiqaeConnectionPolicy.integratorManaged(
            allowedAuthorityOrigins: [XCTUnwrap(URL(string: "https://api.piqae.com"))]
        )
        let host = try PiqaeHostConfiguration(
            product: .embedded,
            applicationID: "com.example.shipping",
            identity: identity,
            installedHostPolicy: .preferInstalled,
            connectionPolicy: policy
        )

        let data = try JSONEncoder().encode(host)
        let value = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(value["application_id"] as? String, "com.example.shipping")
        XCTAssertEqual(value["installed_host_policy"] as? String, "prefer_installed")
        XCTAssertEqual((value["connection_policy"] as? [String: Any])?["allows_multiple"] as? Bool, true)
        XCTAssertEqual(try JSONDecoder().decode(PiqaeHostConfiguration.self, from: data), host)
        XCTAssertEqual(host.effectiveStartupMode, .automatic)
        XCTAssertTrue(host.allowsEmbeddedFallback)
        XCTAssertThrowsError(try policy.validateAuthority(
            XCTUnwrap(URL(string: "https://other.example"))
        ))
        for invalid in [".com.example", "-com.example", "éxample.com"] {
            XCTAssertThrowsError(try PiqaeHostConfiguration(
                product: .embedded,
                applicationID: invalid,
                identity: identity,
                installedHostPolicy: .preferInstalled,
                connectionPolicy: policy
            ))
        }
        XCTAssertThrowsError(try PiqaeHostConfiguration(
            product: .standalone,
            applicationID: "com.example.standalone",
            identity: identity,
            installedHostPolicy: .isolatedApplication,
            connectionPolicy: policy
        ))
    }

    func testStandaloneAndEmbeddedConnectionPoliciesDoNotImposeSingleConnectorLimit() throws {
        XCTAssertTrue(PiqaeConnectionPolicy.standaloneUserManaged.allowsMultiple)
        let embedded = try PiqaeConnectionPolicy.integratorManaged(
            allowedAuthorityOrigins: [XCTUnwrap(URL(string: "https://api.piqae.com"))],
            allowsMultiple: true
        )
        XCTAssertTrue(embedded.allowsMultiple)
        XCTAssertThrowsError(try PiqaeConnectionPolicy(
            management: .hostManaged,
            allowedAuthorityOrigins: []
        ))
        XCTAssertThrowsError(try PiqaeConnectionPolicy(
            management: .userManaged,
            allowsMultiple: false
        ))
        XCTAssertThrowsError(try PiqaeConnectionPolicy.integratorManaged(
            allowedAuthorityOrigins: [
                XCTUnwrap(URL(string: "https://api.piqae.com")),
                XCTUnwrap(URL(string: "https://api.piqae.com/")),
            ]
        ))
        XCTAssertThrowsError(try PiqaeNodeIdentityConfiguration(
            displayName: "Node",
            labels: ["duplicate", "duplicate"]
        ))
        XCTAssertThrowsError(try PiqaeNodeIdentityConfiguration(
            displayName: "Node\u{0000}hidden"
        ))
    }

    func testNativeRuntimeConfigurationBoundsNamesByUTF8BytesWithoutSplittingCharacters() {
        let configuration = PiqaeNativeRuntimeConfiguration(
            applicationID: "com.piqae.tests.bounded-name",
            availability: .backgroundOpportunistic,
            localOnly: true,
            nodeName: String(repeating: "🖨️", count: 40),
            hostname: String(repeating: "é", count: 80)
        )

        XCTAssertLessThanOrEqual(configuration.nodeName.utf8.count, 120)
        XCTAssertLessThanOrEqual(configuration.hostname.utf8.count, 120)
        XCTAssertFalse(configuration.nodeName.unicodeScalars.isEmpty)
        XCTAssertFalse(configuration.hostname.unicodeScalars.isEmpty)
    }

    func testMultipleConnectionsSurviveRuntimeRestartAndRevokeIndependently() async throws {
        let runtime = PiqaeFakeEmbeddedRuntime()
        await runtime.queueConnector(runtimeConnector(id: "ncon_one", workspace: "Workspace one"))
        await runtime.queueConnector(runtimeConnector(id: "ncon_two", workspace: "Workspace two"))
        let identity = PiqaeMemoryInstallationIdentityStore(
            id: .init(rawValue: "ins_many_connectors")
        )
        let firstNode = PiqaeNode(.localOnly(
            startupMode: .embedded,
            identityStore: identity,
            embeddedRuntime: runtime
        ))
        try await firstNode.start()
        let cloud = try PiqaeCloudConfiguration(
            authorityURL: XCTUnwrap(URL(string: "https://api.piqae.com")),
            invitation: PiqaeSensitiveString("invitation")
        )
        _ = try await firstNode.connections.connect(cloud)
        _ = try await firstNode.connections.connect(cloud)
        let firstConnections = try await firstNode.connections.list()
        XCTAssertEqual(firstConnections.count, 2)
        await firstNode.stop()

        let restarted = PiqaeNode(.localOnly(
            startupMode: .embedded,
            identityStore: identity,
            embeddedRuntime: runtime
        ))
        try await restarted.start()
        let restoredConnections = try await restarted.connections.list()
        XCTAssertEqual(restoredConnections.count, 2)
        try await restarted.connections.disconnect(.init(rawValue: "ncon_one"))
        let remainingConnections = try await restarted.connections.list()
        XCTAssertEqual(remainingConnections.map(\.id.rawValue), ["ncon_two"])
        await restarted.stop()
    }

    func testNodeIdentityEditIsRevisionFencedWithoutChangingRuntimeIdentity() async throws {
        let runtime = PiqaeFakeEmbeddedRuntime()
        let node = PiqaeNode(.localOnly(
            startupMode: .embedded,
            identityStore: PiqaeMemoryInstallationIdentityStore(
                id: .init(rawValue: "ins_identity_edit")
            ),
            embeddedRuntime: runtime
        ))
        try await node.start()
        defer { Task { await node.stop() } }
        let identity = try PiqaeNodeIdentityConfiguration(
            displayName: "Kitchen iPad",
            site: "Main",
            location: "Pass",
            labels: ["receipts"]
        )

        let updated = try await node.identity.update(.init(
            expectedRevision: 1,
            identity: identity
        ))
        XCTAssertEqual(updated.revision, 2)
        XCTAssertEqual(updated.identity, identity)
        do {
            _ = try await node.identity.update(.init(
                expectedRevision: 1,
                identity: identity
            ))
            XCTFail("A stale identity edit must fail closed")
        } catch let PiqaeNativeRuntimeError.nodeIdentityRevisionConflict(currentRevision) {
            XCTAssertEqual(currentRevision, 2)
        }
    }

    func testLinkedNativeRuntimeArtifactStartsWhenPresent() async throws {
        let applicationID = "com.piqae.tests.linked.\(UUID().uuidString.lowercased())"
        let hostConfiguration = try PiqaeHostConfiguration(
            product: .embedded,
            applicationID: applicationID,
            identity: try .init(displayName: "Linked test node"),
            installedHostPolicy: .isolatedApplication,
            connectionPolicy: try .init(management: .userManaged)
        )
        let stateDirectory = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/Piqae/embedded", isDirectory: true)
            .appendingPathComponent(applicationID, isDirectory: true)
        let runtime = PiqaeNativeRuntime(
            configuration: PiqaeNativeRuntimeConfiguration(
                applicationID: applicationID,
                dataDirectory: "linked-test",
                availability: .continuousWhileAwake,
                localOnly: true,
                hostConfiguration: hostConfiguration
            ),
            keyStore: PiqaeFixedHostKeyStore()
        )
        let workSignals = LockedCounter()
        try await runtime.setWorkAvailableHandler { workSignals.increment() }
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
        let reconciliation = try await runtime.reconcileCloudOutcome(timeoutMilliseconds: 1_000)
        XCTAssertTrue(reconciliation.loopCompleted)
        XCTAssertFalse(reconciliation.cloudConfigured)
        let identity = try PiqaeNodeIdentityConfiguration(
            displayName: "Kitchen iPad", site: "Main", labels: ["pos"]
        )
        let updated = try await runtime.updateNodeIdentity(.init(
            expectedRevision: 1, identity: identity
        ))
        XCTAssertEqual(updated.revision, 2)
        do {
            _ = try await runtime.updateNodeIdentity(.init(
                expectedRevision: 1, identity: identity
            ))
            XCTFail("A stale native identity edit must fail closed")
        } catch let PiqaeNativeRuntimeError.nodeIdentityRevisionConflict(currentRevision) {
            XCTAssertEqual(currentRevision, 2)
        }
        try await runtime.stop()
        let restarted = PiqaeNativeRuntime(
            configuration: PiqaeNativeRuntimeConfiguration(
                applicationID: applicationID,
                dataDirectory: "linked-test",
                availability: .continuousWhileAwake,
                localOnly: true,
                hostConfiguration: hostConfiguration
            ),
            keyStore: PiqaeFixedHostKeyStore()
        )
        addTeardownBlock { try await restarted.stop() }
        try await restarted.setWorkAvailableHandler {}
        try await restarted.start()
        let afterRestart = try await restarted.updateNodeIdentity(.init(
            expectedRevision: 2,
            identity: try .init(displayName: "Dispatch iPad", location: "Counter 2")
        ))
        XCTAssertEqual(afterRestart.revision, 3)
        try await restarted.stop()
        XCTAssertEqual(workSignals.value, 0)
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
        try await runtime.setWorkAvailableHandler {}
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

    func testDurableRuntimeIdempotencyInvokesAdapterOnce() async throws {
        try requireLinkedRuntime()
        let fixture = nativeFixture("idempotency")
        let runtime = PiqaeNativeRuntime(
            configuration: fixture.configuration,
            keyStore: PiqaeFixedHostKeyStore()
        )
        let adapter = PiqaeFakePrinterAdapter(
            printers: [PiqaeFakePrinterAdapter.printer()]
        )
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: fixture.applicationID)
                ),
                embeddedRuntime: runtime,
                printerAdapters: [adapter]
            )
        )
        addTeardownBlock {
            await node.stop()
            try? FileManager.default.removeItem(at: fixture.stateDirectory)
        }
        try await node.start()
        let listedPrinters = try await node.printers.list()
        let printer = try XCTUnwrap(listedPrinters.first)
        let request = try PiqaePrintRequest(
            printerID: printer.id,
            title: "Durable receipt",
            content: .pdf(Data("%PDF-durable".utf8)),
            idempotencyKey: "order-42-copy-1"
        )

        let first = try await node.jobs.submit(request)
        let second = try await node.jobs.submit(request)
        let status = try await node.jobs.status(first.jobID)
        let history = try await node.jobs.history(offset: 0, limit: 20)

        XCTAssertEqual(first.jobID, second.jobID)
        XCTAssertEqual(first.handoffState, .acceptedBySpooler)
        XCTAssertEqual(second.handoffState, .acceptedBySpooler)
        XCTAssertEqual(status.state, "completed_reported")
        XCTAssertEqual(history.jobs.map(\.jobID), [first.jobID])
        let submissionCount = await adapter.submissionCount()
        XCTAssertEqual(submissionCount, 1)
    }

    func testPrintPacketValueDefaultsToPortablePDFAndOwnsResourceData() throws {
        var resource = Data([1, 2, 3])
        let packet = try PiqaePrintPacket(
            templateJSON: Data(
                #"{"format":"printpacket/v1","media":{"kind":"continuous","width_mm":80},"body":[]}"#.utf8
            ),
            resources: ["logo": resource]
        )
        resource[0] = 99

        XCTAssertEqual(packet.outputTarget, .pdf())
        XCTAssertEqual(packet.resources["logo"], Data([1, 2, 3]))
        XCTAssertThrowsError(try PiqaePrintPacket(templateJSON: Data("[]".utf8)))
        XCTAssertThrowsError(
            try PiqaePrintPacket(
                templateJSON: Data(
                    #"{"format":"printpacket/v0","media":{"kind":"continuous","width_mm":80},"body":[]}"#.utf8
                )
            )
        )
    }

    func testOnlyExplicitPrintPacketCoreErrorRequiresUpdate() {
        XCTAssertEqual(
            PiqaeNativeRuntime.mappedRuntimeError(
                code: "printpacket_core_update_required",
                message: "The renderer is too old."
            ),
            .nativeCoreUpdateRequired
        )
        XCTAssertEqual(
            PiqaeNativeRuntime.mappedRuntimeError(
                code: "invalid_command",
                message: "The command is not recognized."
            ),
            .rejected(code: "invalid_command", message: "The command is not recognized.")
        )
    }

    func testNativeABIRequiresExactlyContractTwo() {
        XCTAssertEqual(PiqaeNativeRuntime.nativeABIVersion, 1)
        XCTAssertEqual(PiqaeNativeRuntime.nativeContractVersion, 2)
        XCTAssertTrue(PiqaeNativeRuntime.supportsNativeContract(abi: 1, minimum: 2, maximum: 2))
        XCTAssertFalse(PiqaeNativeRuntime.supportsNativeContract(abi: 1, minimum: 1, maximum: 2))
        XCTAssertFalse(PiqaeNativeRuntime.supportsNativeContract(abi: 2, minimum: 2, maximum: 2))
    }

    func testPrintPacketFacadeValidatesReceiptAndLabelAndSubmitsIdempotentlyOffline() async throws {
        try requireLinkedRuntime()
        let fixture = nativeFixture("printpacket")
        let runtime = PiqaeNativeRuntime(
            configuration: fixture.configuration,
            keyStore: PiqaeFixedHostKeyStore()
        )
        let adapter = PiqaeFakePrinterAdapter(
            printers: [PiqaeFakePrinterAdapter.printer()]
        )
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: fixture.applicationID)
                ),
                embeddedRuntime: runtime,
                printerAdapters: [adapter]
            )
        )
        addTeardownBlock {
            await node.stop()
            try? FileManager.default.removeItem(at: fixture.stateDirectory)
        }
        try await node.start()
        let printers = try await node.printers.list()
        let printer = try XCTUnwrap(printers.first)
        let receipt = try printPacketFixture("receipt-80mm")
        let label = try printPacketFixture("production-label-100x50")

        let capabilities = try await node.printPackets.capabilities()
        XCTAssertEqual(capabilities.contract, "printpacket/v1")
        XCTAssertEqual(capabilities.rendererABI, "printpacket.pdf-renderer/v1")
        XCTAssertEqual(capabilities.resourceABI, "printpacket.resources/v1")
        XCTAssertEqual(capabilities.cacheProfile, "printpacket.render-cache/v1")
        XCTAssertTrue(capabilities.directOfflineRendering)
        XCTAssertGreaterThan(capabilities.hardLimits.maxPages, 0)

        let receiptValidation = try await node.printPackets.validate(receipt)
        let labelValidation = try await node.printPackets.validate(label)
        XCTAssertEqual(receiptValidation.manifest.specificationVersion, "printpacket/v1")
        XCTAssertEqual(receiptValidation.output.mediaType, "application/pdf")
        XCTAssertEqual(labelValidation.output.mediaType, "application/pdf")
        XCTAssertNotEqual(receiptValidation.cacheKey, labelValidation.cacheKey)

        let request = try PiqaePrintPacketSubmissionRequest(
            adapterID: adapter.descriptor.id,
            printerID: printer.id,
            idempotencyKey: "receipt-1042-copy-1",
            title: "Receipt 1042",
            packet: receipt
        )
        let first = try await node.printPackets.submit(request)
        let second = try await node.printPackets.submit(request)
        XCTAssertEqual(first.job.jobID, second.job.jobID)
        XCTAssertEqual(first.output.sha256, second.output.sha256)
        XCTAssertEqual(first.output.mediaType, "application/pdf")
        let job = try await node.jobs.status(.init(rawValue: first.job.jobID))
        XCTAssertEqual(job.state, "completed_reported")
        _ = await eventually { await adapter.submissionCount() >= 1 }
        let submissionCount = await adapter.submissionCount()
        XCTAssertEqual(submissionCount, 1, "job state: \(job.state)")

        let nativePacket = try PiqaePrintPacket(
            templateJSON: receipt.templateJSON,
            dataJSON: receipt.dataJSON,
            outputTarget: .printerNative(
                language: "zpl",
                profile: "zpl-raster/v1",
                dpi: 203,
                printableWidthDots: 812
            )
        )
        await XCTAssertThrowsErrorAsync(try await node.printPackets.validate(nativePacket)) { error in
            guard case let PiqaeNativeRuntimeError.rejected(code, _) = error else {
                return XCTFail("Expected the native runtime to reject an unsupported target.")
            }
            XCTAssertEqual(code, "printpacket_unsupported_target")
        }
    }

    func testLinkedRuntimeDrainsSecondPrinterWhileFirstAwaitsNativeStatus() async throws {
        try requireLinkedRuntime()
        let fixture = nativeFixture("observation-liveness")
        let runtime = PiqaeNativeRuntime(
            configuration: fixture.configuration,
            keyStore: PiqaeFixedHostKeyStore()
        )
        let adapter = PiqaeFakePrinterAdapter(
            printers: [
                PiqaeFakePrinterAdapter.printer(),
                PiqaeFakePrinterAdapter.printer(
                    id: "prn_second",
                    name: "Second virtual printer"
                ),
            ]
        )
        await adapter.setNativeObservations(
            [.unknown, .unknown, .completedReported],
            for: "native_fake_1"
        )
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: fixture.applicationID)
                ),
                embeddedRuntime: runtime,
                printerAdapters: [adapter]
            )
        )
        addTeardownBlock {
            await node.stop()
            try? FileManager.default.removeItem(at: fixture.stateDirectory)
        }
        try await node.start()
        let printers = try await node.printers.list()
        let firstPrinter = try XCTUnwrap(printers.first { $0.nativeID == "virtual://prn_fake" })
        let secondPrinter = try XCTUnwrap(printers.first { $0.nativeID == "virtual://prn_second" })

        let first = try await node.jobs.submit(
            PiqaePrintRequest(
                printerID: firstPrinter.id,
                title: "Observed first job",
                content: .pdf(Data("%PDF-observed-first".utf8)),
                idempotencyKey: "observed-first"
            )
        )
        let second = try await node.jobs.submit(
            PiqaePrintRequest(
                printerID: secondPrinter.id,
                title: "Runnable second job",
                content: .pdf(Data("%PDF-runnable-second".utf8)),
                idempotencyKey: "runnable-second"
            )
        )

        let initialSubmissionCount = await adapter.submissionCount()
        XCTAssertEqual(initialSubmissionCount, 2)
        let completed = await eventually {
            let firstStatus = try? await node.jobs.status(first.jobID)
            let secondStatus = try? await node.jobs.status(second.jobID)
            return firstStatus?.state == "completed_reported"
                && secondStatus?.state == "completed_reported"
        }
        let firstObservations = await adapter.nativeObservationCount(for: "native_fake_1")
        XCTAssertTrue(completed)
        XCTAssertGreaterThanOrEqual(firstObservations, 3)
        XCTAssertLessThan(firstObservations, 12)
        let finalSubmissionCount = await adapter.submissionCount()
        XCTAssertEqual(finalSubmissionCount, 2)
    }

    func testHandoffStartedRestartBecomesUncertainWithoutNativeReplay() async throws {
        try requireLinkedRuntime()
        let fixture = nativeFixture("restart")
        let firstRuntime = PiqaeNativeRuntime(
            configuration: fixture.configuration,
            keyStore: PiqaeFixedHostKeyStore()
        )
        addTeardownBlock {
            try await firstRuntime.stop()
            try? FileManager.default.removeItem(at: fixture.stateDirectory)
        }
        try await firstRuntime.start()
        let fingerprint = PiqaeAdapterFingerprint(
            platform: .iosNetwork,
            adapterID: "fake.printer",
            adapterVersion: "1"
        )
        let descriptor = PiqaePrinterAdapterDescriptor(
            id: "fake.printer",
            displayName: "Restart fake",
            version: "1",
            transports: [.vendorSDK],
            portableOptions: PiqaePortableOption.allCases,
            supportsProfiles: true
        )
        try await firstRuntime.registerAdapter(
            .init(fingerprint: fingerprint, capabilityContract: .init(descriptor: descriptor))
        )
        let printers = try await firstRuntime.observePrinterInventory(
            adapterID: "fake.printer",
            printers: [
                .init(
                    nativeID: "virtual://prn_restart",
                    name: "Restart printer",
                    state: "available"
                ),
            ]
        )
        let logicalPrinter = PiqaePrinterID(rawValue: try XCTUnwrap(printers.first?.printerID))
        let accepted = try await firstRuntime.enqueue(
            .init(
                adapterID: "fake.printer",
                idempotencyKey: "restart-boundary",
                printerID: logicalPrinter,
                title: "Restart boundary",
                contentKind: "pdf",
                content: Data("%PDF-restart".utf8),
                optionsJSON: #"{"intent":{"copies":1}}"#
            )
        )
        let nextOperation = try await firstRuntime.nextOperation(adapterID: "fake.printer")
        let claimed = try XCTUnwrap(nextOperation)
        let started = try await firstRuntime.beginHandoff(claimed)
        XCTAssertEqual(started.phase, .handoffStarted)
        try await firstRuntime.stop()

        let secondRuntime = PiqaeNativeRuntime(
            configuration: fixture.configuration,
            keyStore: PiqaeFixedHostKeyStore()
        )
        let adapter = PiqaeFakePrinterAdapter(
            printers: [PiqaeFakePrinterAdapter.printer(id: "prn_restart")]
        )
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: fixture.applicationID)
                ),
                embeddedRuntime: secondRuntime,
                printerAdapters: [adapter]
            )
        )
        addTeardownBlock { await node.stop() }
        try await node.start()

        let recovered = try await node.jobs.status(.init(rawValue: accepted.jobID))
        XCTAssertEqual(recovered.state, "delivery_uncertain")
        let submissionCount = await adapter.submissionCount()
        XCTAssertEqual(submissionCount, 0)
    }

    func testAdapterErrorAfterHandoffIsDeliveryUncertain() async throws {
        try requireLinkedRuntime()
        let fixture = nativeFixture("ambiguous")
        let runtime = PiqaeNativeRuntime(
            configuration: fixture.configuration,
            keyStore: PiqaeFixedHostKeyStore()
        )
        let adapter = PiqaeFakePrinterAdapter(
            printers: [PiqaeFakePrinterAdapter.printer()],
            submissionBehavior: .throwAfterHandoff
        )
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: fixture.applicationID)
                ),
                embeddedRuntime: runtime,
                printerAdapters: [adapter]
            )
        )
        addTeardownBlock {
            await node.stop()
            try? FileManager.default.removeItem(at: fixture.stateDirectory)
        }
        try await node.start()
        let listedPrinters = try await node.printers.list()
        let printer = try XCTUnwrap(listedPrinters.first)
        let receipt = try await node.jobs.submit(
            PiqaePrintRequest(
                printerID: printer.id,
                title: "Ambiguous receipt",
                content: .pdf(Data("%PDF-ambiguous".utf8)),
                idempotencyKey: "ambiguous-boundary"
            )
        )

        XCTAssertEqual(receipt.handoffState, .deliveryUncertain)
        let submissionCount = await adapter.submissionCount()
        let status = try await node.jobs.status(receipt.jobID)
        XCTAssertEqual(submissionCount, 1)
        XCTAssertEqual(status.state, "delivery_uncertain")
    }

    func testBackgroundWakeExecutesPreviouslyDurableJobWhenBudgetAllows() async throws {
        try requireLinkedRuntime()
        let fixture = nativeFixture("background")
        let runtime = PiqaeNativeRuntime(
            configuration: fixture.configuration,
            keyStore: PiqaeFixedHostKeyStore()
        )
        let adapter = PiqaeFakePrinterAdapter(
            printers: [PiqaeFakePrinterAdapter.printer()]
        )
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                availability: .backgroundOpportunistic,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: fixture.applicationID)
                ),
                embeddedRuntime: runtime,
                printerAdapters: [adapter]
            )
        )
        addTeardownBlock {
            await node.stop()
            try? FileManager.default.removeItem(at: fixture.stateDirectory)
        }
        try await node.start()
        let printers = try await node.printers.list()
        let printer = try XCTUnwrap(printers.first)
        await node.updateExecutionContext(
            .init(phase: .background, source: .backgroundPush, remainingSeconds: 5)
        )
        let queued = try await node.jobs.submit(
            PiqaePrintRequest(
                printerID: printer.id,
                title: "Deferred receipt",
                content: .pdf(Data("%PDF-deferred".utf8)),
                idempotencyKey: "background-durable"
            )
        )
        XCTAssertEqual(queued.handoffState, .queuedLocally)
        let countBeforeWake = await adapter.submissionCount()
        XCTAssertEqual(countBeforeWake, 0)

        let result = await node.handleWakeHint(
            try PiqaeWakeHint(collapseID: "job-available", source: .backgroundPush),
            context: .init(phase: .background, source: .backgroundPush, remainingSeconds: 30)
        )
        let completed = await eventually {
            (try? await node.jobs.status(queued.jobID))?.state == "completed_reported"
        }
        let status = try await node.jobs.status(queued.jobID)
        let count = await adapter.submissionCount()
        XCTAssertEqual(result, .reconciled)
        XCTAssertTrue(completed)
        XCTAssertEqual(status.state, "completed_reported")
        XCTAssertEqual(count, 1)
    }

    func testProfilesAreCreatedUpdatedAndDeletedByDurableRuntime() async throws {
        try requireLinkedRuntime()
        let fixture = nativeFixture("profiles")
        let runtime = PiqaeNativeRuntime(
            configuration: fixture.configuration,
            keyStore: PiqaeFixedHostKeyStore()
        )
        let adapter = PiqaeFakePrinterAdapter(
            printers: [PiqaeFakePrinterAdapter.printer()]
        )
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: fixture.applicationID)
                ),
                embeddedRuntime: runtime,
                printerAdapters: [adapter]
            )
        )
        addTeardownBlock {
            await node.stop()
            try? FileManager.default.removeItem(at: fixture.stateDirectory)
        }
        try await node.start()
        let printers = try await node.printers.list()
        let printer = try XCTUnwrap(printers.first)

        let created = try await node.profiles.create(
            .init(printerID: printer.id, name: "Receipt", isDefault: true)
        )
        let updated = try await node.profiles.update(
            .init(
                printerID: printer.id,
                profileID: created.id,
                expectedRevision: created.revision,
                name: "Receipt 80 mm",
                isDefault: true
            )
        )
        let profiles = try await node.profiles.list(for: printer.id)
        XCTAssertEqual(profiles, [updated])

        try await node.profiles.delete(
            printerID: printer.id,
            profileID: updated.id,
            expectedRevision: updated.revision
        )
        let remaining = try await node.profiles.list(for: printer.id)
        XCTAssertEqual(remaining, [])
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

    func testRemoteNotificationTokenRotationAndRegistrationRetryAreExplicit() async throws {
        let provider = PiqaeFakeRemoteNotificationProvider()
        await provider.failNextRegistrations(1)
        let node = PiqaeNode(
            PiqaeNodeConfiguration(
                startupMode: .embedded,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: "ins_apple_push_rotation")
                ),
                remoteNotificationProvider: provider
            )
        )
        try await node.start()
        defer { Task { await node.stop() } }

        await XCTAssertThrowsErrorAsync(
            try await node.remoteNotifications.register(
                deviceToken: Data([1]),
                environment: .development,
                bundleIdentifier: "com.example.print"
            )
        )
        try await node.remoteNotifications.register(
            deviceToken: Data([1]),
            environment: .development,
            bundleIdentifier: "com.example.print"
        )
        try await node.remoteNotifications.register(
            deviceToken: Data([2]),
            environment: .development,
            bundleIdentifier: "com.example.print"
        )

        let registrations = await provider.registrations
        XCTAssertEqual(registrations.count, 2)
        XCTAssertEqual(registrations[0].token.withBytes { $0 }, Data([1]))
        XCTAssertEqual(registrations[1].token.withBytes { $0 }, Data([2]))
        XCTAssertEqual(
            node.remoteNotifications.whenTerminated,
            .unavailableWhenTerminated
        )
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
        XCTAssertEqual(
            node.remoteNotifications.whenTerminated,
            .unavailableWhenTerminated
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
        let runtime = PiqaeFakeEmbeddedRuntime(
            connector: runtimeConnector(id: "ncon_explicit", workspace: "Explicit workspace")
        )
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: "ins_explicit_connect")
                ),
                embeddedRuntime: runtime
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
            invitation: PiqaeSensitiveString("invitation")
        )

        _ = try await node.connections.connect(cloud)
        let connections = try await node.connections.list()
        XCTAssertEqual(connections, [connection])
    }

    func testConnectionEnrollmentRequiresStartedNode() async throws {
        let node = PiqaeNode(.localOnly(startupMode: .embedded))
        let cloud = try PiqaeCloudConfiguration(
            authorityURL: XCTUnwrap(URL(string: "https://api.piqae.com")),
            invitation: PiqaeSensitiveString("invitation")
        )
        await XCTAssertThrowsErrorAsync(try await node.connections.connect(cloud)) { error in
            XCTAssertEqual(error as? PiqaeNodeError, .notStarted)
        }
    }

    func testAutomaticDesktopModeAttachesBeforeStartingEmbeddedRuntime() async throws {
        let remote = attachedSnapshot()
        let ipc = PiqaeFakeInstalledNodeIPC(protocolVersion: 4, snapshot: remote)
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
                .incompatibleInstalledNode(found: 99, supported: 4 ... 4)
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
        let runtime = PiqaeFakeEmbeddedRuntime(
            connector: runtimeConnector(id: "ncon_cloud", workspace: "Managed shop")
        )
        let invitation = try PiqaeSensitiveString("one-use-invitation")
        let cloud = try PiqaeCloudConfiguration(
            authorityURL: XCTUnwrap(URL(string: "https://api.piqae.com")),
            invitation: invitation
        )
        let node = PiqaeNode(
            PiqaeNodeConfiguration(
                startupMode: .embedded,
                connectivity: .cloud(cloud),
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: "ins_cloud_test")
                ),
                embeddedRuntime: runtime
            )
        )
        try await node.start()
        defer { Task { await node.stop() } }

        let connections = try await node.connections.list()
        let enrollmentRequestCount = await runtime.connectCount
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
            invitation: PiqaeSensitiveString("single-use-invitation")
        )
        let failed = PiqaeNode(
            PiqaeNodeConfiguration(
                startupMode: .embedded,
                connectivity: .cloud(cloud),
                identityStore: identity,
                embeddedRuntime: PiqaeFakeEmbeddedRuntime(failsToConnect: true)
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
        XCTAssertThrowsError(
            try PiqaeCloudConfiguration(
                authorityURL: XCTUnwrap(URL(string: "https://api.piqae.com?token=secret")),
                invitation: invitation
            )
        )
        XCTAssertThrowsError(
            try PiqaeCloudConfiguration(
                authorityURL: XCTUnwrap(URL(string: "https://user:pass@api.piqae.com")),
                invitation: invitation
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
            guard case .unsupportedOperation = error as? PiqaeNodeError else {
                XCTFail("submission without a durable runtime must fail closed")
                return
            }
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
        XCTAssertEqual(result, .reconciled)
        let submissionCount = await adapter.submissionCount()
        XCTAssertEqual(submissionCount, 0)
    }

    func testWakeHintRetriesCloudReconciliationWithinBound() async throws {
        let runtime = PiqaeFakeEmbeddedRuntime()
        await runtime.setCloudReconcileOutcomes([
            cloudOutcome(failed: 1, succeeded: 0, retryable: true, failure: .transient),
            cloudOutcome(failed: 1, succeeded: 0, retryable: true, failure: .transient),
            cloudOutcome(failed: 0, succeeded: 1, retryable: false, failure: .none),
        ])
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                availability: .backgroundOpportunistic,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: "ins_wake_retry_test")
                ),
                embeddedRuntime: runtime,
                wakeRetryPolicy: .init(
                    maximumAttempts: 4,
                    initialDelaySeconds: 0.001,
                    maximumDelaySeconds: 0.001,
                    executionSafetyMarginSeconds: 0.25,
                    cloudCycleTimeoutSeconds: 0.25
                )
            )
        )
        try await node.start()
        defer { Task { await node.stop() } }

        let result = await node.handleWakeHint(
            try PiqaeWakeHint(collapseID: "retryable-hint", source: .backgroundPush),
            context: .init(
                phase: .background,
                source: .backgroundPush,
                remainingSeconds: 10
            )
        )

        XCTAssertEqual(result, .reconciled)
        let reconcileCalls = await runtime.reconcileCallCount()
        XCTAssertEqual(reconcileCalls, 3)
    }

    func testLegacyBoolOnlyRuntimeAdaptsWithoutInventingConnectorIdentity() async throws {
        let runtime = LegacyBoolReconcileRuntime()
        let outcome = try await runtime.reconcileCloudOutcome(timeoutMilliseconds: 1_000)

        XCTAssertTrue(outcome.cloudConfigured)
        XCTAssertTrue(outcome.loopCompleted)
        XCTAssertEqual(outcome.connectorCount, 0)
        XCTAssertEqual(outcome.failedCount, 0)
    }

    func testWakeHintDoesNotRetryUnclassifiedRuntimeFailure() async throws {
        let runtime = PiqaeFakeEmbeddedRuntime()
        await runtime.failNextCloudReconciliations(2)
        let node = wakeTestNode(runtime: runtime, id: "ins_wake_unclassified")
        try await node.start()
        defer { Task { await node.stop() } }

        let result = await node.handleWakeHint(
            try PiqaeWakeHint(collapseID: "unclassified", source: .backgroundPush),
            context: .init(phase: .background, source: .backgroundPush, remainingSeconds: 10)
        )

        guard case .deferred = result else {
            XCTFail("An unclassified runtime failure must defer")
            return
        }
        let reconcileCalls = await runtime.reconcileCallCount()
        XCTAssertEqual(reconcileCalls, 1)
    }

    func testWakeHintDoesNotRetryNonRetryableConnectorFailure() async throws {
        let runtime = PiqaeFakeEmbeddedRuntime()
        await runtime.setCloudReconcileOutcome(
            cloudOutcome(failed: 1, succeeded: 0, retryable: false, failure: .authentication)
        )
        let node = wakeTestNode(runtime: runtime, id: "ins_wake_nonretryable")
        try await node.start()
        defer { Task { await node.stop() } }

        let result = await node.handleWakeHint(
            try PiqaeWakeHint(collapseID: "auth-failure", source: .backgroundPush),
            context: .init(phase: .background, source: .backgroundPush, remainingSeconds: 10)
        )

        guard case .deferred = result else {
            XCTFail("A non-retryable supervisor outcome must defer")
            return
        }
        let reconcileCalls = await runtime.reconcileCallCount()
        XCTAssertEqual(reconcileCalls, 1)
    }

    func testWakeHintRetriesPrivacySafePartialTransientOutcome() async throws {
        let runtime = PiqaeFakeEmbeddedRuntime()
        await runtime.setCloudReconcileOutcomes([
            cloudOutcome(failed: 1, succeeded: 1, retryable: true, failure: .transient),
            cloudOutcome(failed: 0, succeeded: 2, retryable: false, failure: .none),
        ])
        let node = wakeTestNode(runtime: runtime, id: "ins_wake_partial")
        try await node.start()
        defer { Task { await node.stop() } }

        let result = await node.handleWakeHint(
            try PiqaeWakeHint(collapseID: "partial", source: .backgroundPush),
            context: .init(phase: .background, source: .backgroundPush, remainingSeconds: 10)
        )

        XCTAssertEqual(result, .reconciled)
        let reconcileCalls = await runtime.reconcileCallCount()
        XCTAssertEqual(reconcileCalls, 2)
    }

    func testConcurrentDuplicateCollapseIDsShareOneReconciliation() async throws {
        let runtime = PiqaeFakeEmbeddedRuntime()
        await runtime.setCloudReconcileDelayNanoseconds(100_000_000)
        let node = wakeTestNode(runtime: runtime, id: "ins_wake_collapse")
        try await node.start()
        defer { Task { await node.stop() } }
        let hint = try PiqaeWakeHint(collapseID: "same-hint", source: .backgroundPush)
        let context = PiqaeExecutionContext(
            phase: .background,
            source: .backgroundPush,
            remainingSeconds: 10
        )

        async let first = node.handleWakeHint(hint, context: context)
        async let second = node.handleWakeHint(hint, context: context)
        let results = await [first, second]

        XCTAssertEqual(results, [.reconciled, .reconciled])
        let reconcileCalls = await runtime.reconcileCallCount()
        XCTAssertEqual(reconcileCalls, 1)
    }

    func testCancellingOneDuplicateWakeCallerDoesNotCancelSharedReconciliation() async throws {
        // Repetition covers cancellation arriving around waiter registration.
        // The explicit gate, rather than a short sleep, keeps the shared pass
        // pending until every assertion about the cancelled joiner is made.
        for iteration in 0..<25 {
            let runtime = GatedWakeReconcileRuntime()
            let node = wakeTestNode(
                runtime: runtime,
                id: "ins_wake_collapse_cancel_\(iteration)"
            )
            try await node.start()
            let hint = try PiqaeWakeHint(
                collapseID: "shared-hint-\(iteration)",
                source: .backgroundPush
            )
            let context = PiqaeExecutionContext(
                phase: .background,
                source: .backgroundPush,
                remainingSeconds: 10
            )

            let first = Task { await node.handleWakeHint(hint, context: context) }
            let started = await eventually { await runtime.reconcileCallCount() == 1 }
            XCTAssertTrue(started, "shared pass did not start at iteration \(iteration)")

            let second = Task { await node.handleWakeHint(hint, context: context) }
            let joined = await eventually {
                await node.wakeWaiterCountForTesting(collapseID: hint.collapseID) == 2
            }
            XCTAssertTrue(joined, "duplicate caller did not join at iteration \(iteration)")

            second.cancel()
            let result = WakeResultRecorder()
            let observeCancellation = Task {
                await result.record(second.value)
            }
            let detachedBeforeRelease = await eventually(attempts: 50) {
                await result.value != nil
            }
            let cancelledResult = await result.value

            // Releasing only after the prompt-detach observation makes a
            // cancellation propagation bug fail without hanging the suite.
            await runtime.release()
            let firstResult = await first.value
            await observeCancellation.value
            let reconcileCalls = await runtime.reconcileCallCount()
            await node.stop()

            XCTAssertTrue(
                detachedBeforeRelease,
                "cancelled joiner did not detach promptly at iteration \(iteration)"
            )
            guard case .deferred = cancelledResult else {
                XCTFail("cancelled joiner returned \(String(describing: cancelledResult))")
                continue
            }
            XCTAssertEqual(firstResult, .reconciled, "iteration \(iteration)")
            XCTAssertEqual(reconcileCalls, 1, "iteration \(iteration)")
        }
    }

    func testWakeCancellationReturnsPromptlyWhileCloudPassIsPending() async throws {
        let runtime = PiqaeFakeEmbeddedRuntime()
        await runtime.setCloudReconcileDelayNanoseconds(30_000_000_000)
        let node = wakeTestNode(runtime: runtime, id: "ins_wake_cancel")
        try await node.start()
        defer { Task { await node.stop() } }
        let task = Task {
            await node.handleWakeHint(
                try! PiqaeWakeHint(collapseID: "cancel", source: .backgroundPush),
                context: .init(
                    phase: .background,
                    source: .backgroundPush,
                    remainingSeconds: 30
                )
            )
        }
        let reconcileStarted = await eventually { await runtime.reconcileCallCount() == 1 }
        XCTAssertTrue(reconcileStarted)

        let started = ContinuousClock.now
        task.cancel()
        guard case .deferred = await task.value else {
            XCTFail("Cancellation must defer the wake pass")
            return
        }
        XCTAssertLessThan(started.duration(to: .now), .milliseconds(500))
    }

    func testWakeHintDoesNotRetryPastBackgroundBudget() async throws {
        let runtime = PiqaeFakeEmbeddedRuntime()
        await runtime.failNextCloudReconciliations(8)
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                availability: .backgroundOpportunistic,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: "ins_wake_budget_test")
                ),
                embeddedRuntime: runtime,
                wakeRetryPolicy: .init(
                    maximumAttempts: 8,
                    initialDelaySeconds: 0.1,
                    maximumDelaySeconds: 0.1,
                    executionSafetyMarginSeconds: 1,
                    cloudCycleTimeoutSeconds: 1
                )
            )
        )
        try await node.start()
        defer { Task { await node.stop() } }

        let result = await node.handleWakeHint(
            try PiqaeWakeHint(collapseID: "expired-budget", source: .backgroundPush),
            context: .init(
                phase: .background,
                source: .backgroundPush,
                remainingSeconds: 0.5
            )
        )

        guard case .deferred = result else {
            XCTFail("A budget below the safety margin must defer")
            return
        }
        let reconcileCalls = await runtime.reconcileCallCount()
        XCTAssertEqual(reconcileCalls, 0)
    }

    func testSynchronousExpirationFencePreventsTheNextNativeHandoff() async throws {
        let fixture = try automaticDrainFixture("expiration-before-handoff")
        defer { try? FileManager.default.removeItem(at: fixture.contentURL) }
        try await fixture.node.start()
        defer { Task { await fixture.node.stop() } }
        await fixture.runtime.setNextOperationDelayNanoseconds(150_000_000)
        let callsBeforeActivation = await fixture.runtime.nextOperationCallCount()

        await fixture.runtime.activateRemoteOperation(fixture.operation)
        let operationRequested = await eventually {
            await fixture.runtime.nextOperationCallCount() > callsBeforeActivation
        }
        XCTAssertTrue(operationRequested)
        fixture.node.expireExecutionSynchronously()
        try await Task.sleep(nanoseconds: 250_000_000)

        let submissionCount = await fixture.adapter.submissionCount()
        XCTAssertEqual(submissionCount, 0)
    }

    func testExpirationAfterHandoffIntentPreservesAmbiguousOutcome() async throws {
        let fixture = try automaticDrainFixture("expiration-after-intent")
        defer { try? FileManager.default.removeItem(at: fixture.contentURL) }
        await fixture.adapter.setSubmissionDelayNanoseconds(30_000_000_000)
        try await fixture.node.start()
        defer { Task { await fixture.node.stop() } }

        await fixture.runtime.activateRemoteOperation(fixture.operation)
        let submissionStarted = await eventually {
            await fixture.adapter.submissionCount() == 1
        }
        XCTAssertTrue(submissionStarted)
        fixture.node.expireExecutionSynchronously()
        await fixture.node.updateExecutionContext(
            .init(phase: .suspended, source: .backgroundPush)
        )

        let completionStates = await fixture.runtime.completionStates()
        XCTAssertTrue(completionStates.contains("delivery_uncertain"))
    }

    func testRemoteQueueActivationDrainsWithoutManualRefresh() async throws {
        let fixture = try automaticDrainFixture("remote-activation")
        defer { try? FileManager.default.removeItem(at: fixture.contentURL) }
        try await fixture.node.start()
        defer { Task { await fixture.node.stop() } }

        await fixture.runtime.activateRemoteOperation(fixture.operation)

        let submitted = await eventually { await fixture.adapter.submissionCount() == 1 }
        let submissionCount = await fixture.adapter.submissionCount()
        XCTAssertTrue(submitted)
        XCTAssertEqual(submissionCount, 1)
    }

    func testDuplicateWorkSignalsCoalesceIntoOneAdapterDrain() async throws {
        let fixture = try automaticDrainFixture("duplicate-signals")
        defer { try? FileManager.default.removeItem(at: fixture.contentURL) }
        try await fixture.node.start()
        defer { Task { await fixture.node.stop() } }
        await fixture.runtime.setNextOperationDelayNanoseconds(30_000_000)

        await fixture.runtime.activateRemoteOperation(
            fixture.operation,
            notificationCount: 20
        )

        let submitted = await eventually { await fixture.adapter.submissionCount() == 1 }
        let submissionCount = await fixture.adapter.submissionCount()
        let maximumConcurrentCalls = await fixture.runtime.maximumConcurrentNextOperationCalls()
        XCTAssertTrue(submitted)
        XCTAssertEqual(submissionCount, 1)
        XCTAssertEqual(maximumConcurrentCalls, 1)
    }

    func testAcceptedObservationDoesNotBlockLaterRunnableWork() async throws {
        let contentURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("piqae-observation-\(UUID().uuidString).pdf")
        try Data("%PDF-piqae-observation".utf8).write(to: contentURL, options: .atomic)
        defer { try? FileManager.default.removeItem(at: contentURL) }
        let runtime = PiqaeFakeEmbeddedRuntime()
        let adapter = PiqaeFakePrinterAdapter(
            printers: [
                PiqaeFakePrinterAdapter.printer(),
                PiqaeFakePrinterAdapter.printer(
                    id: "prn_second",
                    name: "Second virtual printer"
                ),
            ]
        )
        await adapter.setNativeObservations(
            [.unknown, .unknown, .completedReported],
            for: "native_fake_1"
        )
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: "ins_observation_liveness")
                ),
                embeddedRuntime: runtime,
                printerAdapters: [adapter]
            )
        )
        try await node.start()
        defer { Task { await node.stop() } }

        await runtime.activateRemoteOperation(
            try automaticOperation("observed-first", contentURL: contentURL)
        )
        let firstAwaitingObservation = await eventually {
            let submissions = await adapter.submissionCount()
            let observations = await adapter.nativeObservationCount(for: "native_fake_1")
            return submissions == 1 && observations >= 1
        }
        XCTAssertTrue(firstAwaitingObservation)
        await runtime.activateRemoteOperation(
            try automaticOperation(
                "runnable-second",
                contentURL: contentURL,
                printerID: "prn_second",
                printerNativeID: "virtual://prn_second"
            )
        )

        let bothSubmitted = await eventually { await adapter.submissionCount() == 2 }
        let firstCompleted = await eventually {
            ((try? await runtime.nativeObservations(adapterID: "fake.printer")) ?? []).isEmpty
        }
        let observationCount = await adapter.nativeObservationCount(for: "native_fake_1")
        XCTAssertTrue(bothSubmitted)
        XCTAssertTrue(firstCompleted)
        XCTAssertGreaterThanOrEqual(observationCount, 3)
        XCTAssertLessThan(observationCount, 12)
    }

    func testNativeObservationDefersWithoutBackgroundBudgetAndResumesInForeground() async throws {
        let fixture = try automaticDrainFixture(
            "observation-background",
            availability: .backgroundOpportunistic
        )
        defer { try? FileManager.default.removeItem(at: fixture.contentURL) }
        await fixture.adapter.setNativeObservations(
            Array(repeating: .unknown, count: 100),
            for: "native_fake_1"
        )
        try await fixture.node.start()
        defer { Task { await fixture.node.stop() } }
        await fixture.runtime.activateRemoteOperation(fixture.operation)
        let began = await eventually {
            await fixture.adapter.nativeObservationCount(for: "native_fake_1") >= 1
        }
        XCTAssertTrue(began)

        await fixture.node.updateExecutionContext(
            .init(phase: .background, source: .backgroundPush)
        )
        try await Task.sleep(nanoseconds: 120_000_000)
        let pausedCount = await fixture.adapter.nativeObservationCount(for: "native_fake_1")
        try await Task.sleep(nanoseconds: 120_000_000)
        let stillPausedCount = await fixture.adapter.nativeObservationCount(for: "native_fake_1")
        XCTAssertEqual(stillPausedCount, pausedCount)

        await fixture.node.updateExecutionContext(.foreground)
        let resumed = await eventually {
            await fixture.adapter.nativeObservationCount(for: "native_fake_1") > pausedCount
        }
        let finalCount = await fixture.adapter.nativeObservationCount(for: "native_fake_1")
        let runtimeCount = await fixture.runtime.nativeObservationCallCount()
        XCTAssertTrue(
            resumed,
            "observation did not resume (paused: \(pausedCount), final: \(finalCount), runtime reads: \(runtimeCount))"
        )
    }

    func testNativeObservationStopsAtExplicitBackgroundBudget() async throws {
        let fixture = try automaticDrainFixture(
            "observation-budget",
            availability: .backgroundOpportunistic
        )
        defer { try? FileManager.default.removeItem(at: fixture.contentURL) }
        await fixture.adapter.setNativeObservations(
            Array(repeating: .unknown, count: 100),
            for: "native_fake_1"
        )
        try await fixture.node.start()
        defer { Task { await fixture.node.stop() } }
        await fixture.runtime.activateRemoteOperation(fixture.operation)
        let began = await eventually {
            await fixture.adapter.nativeObservationCount(for: "native_fake_1") >= 1
        }
        XCTAssertTrue(began)

        await fixture.node.updateExecutionContext(
            .init(phase: .background, source: .backgroundPush, remainingSeconds: 0.4)
        )
        try await Task.sleep(nanoseconds: 500_000_000)
        let countAfterBudget = await fixture.adapter.nativeObservationCount(for: "native_fake_1")
        try await Task.sleep(nanoseconds: 150_000_000)
        let finalCount = await fixture.adapter.nativeObservationCount(for: "native_fake_1")
        XCTAssertEqual(finalCount, countAfterBudget)
    }

    func testUnobservableAcceptedOperationDoesNotSpin() async throws {
        let fixture = try automaticDrainFixture("observation-unavailable")
        defer { try? FileManager.default.removeItem(at: fixture.contentURL) }
        let missingPrinterOperation = try automaticOperation(
            "missing-printer",
            contentURL: fixture.contentURL,
            printerID: "prn_missing",
            printerNativeID: "virtual://prn_missing"
        )
        await fixture.runtime.activateNativeObservation(
            missingPrinterOperation,
            nativeJobID: "native_missing"
        )
        try await fixture.node.start()
        defer { Task { await fixture.node.stop() } }

        let observedOnce = await eventually {
            await fixture.runtime.nativeObservationCallCount() >= 1
        }
        XCTAssertTrue(observedOnce)
        let initialCount = await fixture.runtime.nativeObservationCallCount()
        try await Task.sleep(nanoseconds: 150_000_000)
        let finalCount = await fixture.runtime.nativeObservationCallCount()
        XCTAssertEqual(finalCount, initialCount)
    }

    func testStopCancelsAndJoinsNativeObservation() async throws {
        let fixture = try automaticDrainFixture("observation-stop")
        defer { try? FileManager.default.removeItem(at: fixture.contentURL) }
        await fixture.adapter.setNativeObservations(
            [.unknown],
            for: "native_fake_1"
        )
        await fixture.adapter.setNativeObservationDelayNanoseconds(60_000_000_000)
        try await fixture.node.start()
        await fixture.runtime.activateRemoteOperation(fixture.operation)
        let observationStarted = await eventually {
            await fixture.adapter.nativeObservationCount(for: "native_fake_1") == 1
        }
        XCTAssertTrue(observationStarted)

        await fixture.node.stop()

        let stopCount = await fixture.runtime.stopCount
        XCTAssertEqual(stopCount, 1)
        let countAfterStop = await fixture.adapter.nativeObservationCount(for: "native_fake_1")
        try await Task.sleep(nanoseconds: 20_000_000)
        let finalCount = await fixture.adapter.nativeObservationCount(for: "native_fake_1")
        XCTAssertEqual(finalCount, countAfterStop)
    }

    func testBackgroundWorkDefersUntilAnExplicitBudgetIsAvailable() async throws {
        let fixture = try automaticDrainFixture(
            "background-budget",
            availability: .backgroundOpportunistic
        )
        defer { try? FileManager.default.removeItem(at: fixture.contentURL) }
        try await fixture.node.start()
        defer { Task { await fixture.node.stop() } }
        await fixture.node.updateExecutionContext(
            .init(phase: .background, source: .backgroundPush, remainingSeconds: 3)
        )

        await fixture.runtime.activateRemoteOperation(fixture.operation)
        try await Task.sleep(nanoseconds: 50_000_000)
        let shortBudgetSubmissionCount = await fixture.adapter.submissionCount()
        XCTAssertEqual(shortBudgetSubmissionCount, 0)

        await fixture.node.updateExecutionContext(
            .init(phase: .background, source: .backgroundPush, remainingSeconds: 30)
        )
        let submittedWithBudget = await eventually {
            await fixture.adapter.submissionCount() == 1
        }
        XCTAssertTrue(submittedWithBudget)
    }

    func testForegroundWakeAndNetworkRestorationReconcilePendingWork() async throws {
        let fixture = try automaticDrainFixture("lifecycle-reconcile")
        defer { try? FileManager.default.removeItem(at: fixture.contentURL) }
        try await fixture.node.start()
        defer { Task { await fixture.node.stop() } }
        await fixture.node.updateExecutionContext(
            .init(phase: .background, source: .foreground)
        )

        await fixture.runtime.activateRemoteOperation(fixture.operation)
        try await Task.sleep(nanoseconds: 30_000_000)
        let backgroundSubmissionCount = await fixture.adapter.submissionCount()
        XCTAssertEqual(backgroundSubmissionCount, 0)
        try await fixture.node.reportHostLifecycle(.enteredForeground)
        let foregroundSubmitted = await eventually {
            await fixture.adapter.submissionCount() == 1
        }
        XCTAssertTrue(foregroundSubmitted)

        let networkOperation = try automaticOperation(
            "network-restored",
            contentURL: fixture.contentURL
        )
        await fixture.runtime.activateRemoteOperation(networkOperation, notificationCount: 0)
        try await fixture.node.reportHostLifecycle(.networkAvailable)
        let networkSubmitted = await eventually {
            await fixture.adapter.submissionCount() == 2
        }
        XCTAssertTrue(networkSubmitted)

        let wakeOperation = try automaticOperation(
            "wake-restored",
            contentURL: fixture.contentURL
        )
        try await fixture.node.reportHostLifecycle(.sleeping)
        await fixture.runtime.activateRemoteOperation(wakeOperation, notificationCount: 0)
        try await fixture.node.reportHostLifecycle(.woke)
        let wakeSubmitted = await eventually { await fixture.adapter.submissionCount() == 3 }
        XCTAssertTrue(wakeSubmitted)
    }

    func testStopCancelsAndJoinsAutomaticDrainBeforeReleasingHandler() async throws {
        let fixture = try automaticDrainFixture("stop-join")
        defer { try? FileManager.default.removeItem(at: fixture.contentURL) }
        try await fixture.node.start()
        await fixture.runtime.setNextOperationDelayNanoseconds(60_000_000_000)
        let callsBeforeActivation = await fixture.runtime.nextOperationCallCount()
        await fixture.runtime.activateRemoteOperation(fixture.operation)
        let drainStarted = await eventually {
            await fixture.runtime.nextOperationCallCount() > callsBeforeActivation
        }
        XCTAssertTrue(drainStarted)

        await fixture.node.stop()

        let hasHandler = await fixture.runtime.hasWorkAvailableHandler()
        let stopCount = await fixture.runtime.stopCount
        let submissionCount = await fixture.adapter.submissionCount()
        XCTAssertFalse(hasHandler)
        XCTAssertEqual(stopCount, 1)
        XCTAssertEqual(submissionCount, 0)
        await fixture.runtime.notifyWorkAvailable(count: 3)
        try await Task.sleep(nanoseconds: 20_000_000)
        let submissionCountAfterStop = await fixture.adapter.submissionCount()
        XCTAssertEqual(submissionCountAfterStop, 0)
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

    private func nativeFixture(_ label: String) -> (
        applicationID: String,
        configuration: PiqaeNativeRuntimeConfiguration,
        stateDirectory: URL
    ) {
        let applicationID = "com.piqae.tests.\(label).\(UUID().uuidString.lowercased())"
        let stateDirectory = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/Piqae/embedded", isDirectory: true)
            .appendingPathComponent(applicationID, isDirectory: true)
        return (
            applicationID,
            PiqaeNativeRuntimeConfiguration(
                applicationID: applicationID,
                dataDirectory: "nodekit-tests",
                availability: .continuousWhileAwake,
                localOnly: true
            ),
            stateDirectory
        )
    }

    private func printPacketFixture(_ name: String) throws -> PiqaePrintPacket {
        let repositoryRoot = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let url = repositoryRoot
            .appendingPathComponent("standards/printpacket/conformance", isDirectory: true)
            .appendingPathComponent("\(name).json")
        let object = try JSONSerialization.jsonObject(with: Data(contentsOf: url))
        guard let fixture = object as? [String: Any],
            let template = fixture["template"],
            let data = fixture["data"]
        else {
            throw PiqaeNodeError.invalidConfiguration("The PrintPacket fixture is invalid.")
        }
        return try PiqaePrintPacket(
            templateJSON: JSONSerialization.data(withJSONObject: template),
            dataJSON: JSONSerialization.data(withJSONObject: data)
        )
    }

    private func automaticDrainFixture(
        _ label: String,
        availability: PiqaeNodeAvailabilityClass = .continuousWhileAwake
    ) throws -> (
        node: PiqaeNode,
        runtime: PiqaeFakeEmbeddedRuntime,
        adapter: PiqaeFakePrinterAdapter,
        operation: PiqaeRuntimeAdapterOperation,
        contentURL: URL
    ) {
        let contentURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("piqae-nodekit-\(UUID().uuidString).pdf")
        try Data("%PDF-piqae-fake".utf8).write(to: contentURL, options: .atomic)
        let runtime = PiqaeFakeEmbeddedRuntime()
        let adapter = PiqaeFakePrinterAdapter(
            printers: [PiqaeFakePrinterAdapter.printer()]
        )
        let node = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                availability: availability,
                identityStore: PiqaeMemoryInstallationIdentityStore(
                    id: .init(rawValue: "ins_auto_\(label)")
                ),
                embeddedRuntime: runtime,
                printerAdapters: [adapter]
            )
        )
        return (
            node,
            runtime,
            adapter,
            try automaticOperation(label, contentURL: contentURL),
            contentURL
        )
    }

    private func automaticOperation(
        _ label: String,
        contentURL: URL,
        printerID: String = "prn_fake",
        printerNativeID: String = "virtual://prn_fake"
    ) throws -> PiqaeRuntimeAdapterOperation {
        let content = try Data(contentsOf: contentURL)
        let digest = SHA256.hash(data: content).map { String(format: "%02x", $0) }.joined()
        return PiqaeRuntimeAdapterOperation(
            operationID: "op_\(label)",
            adapterID: "fake.printer",
            jobID: "job_\(label)",
            idempotencyKey: "idem-\(label)",
            fence: "fence_\(label)",
            deadlineUnixMilliseconds: Int64(Date().addingTimeInterval(60).timeIntervalSince1970 * 1_000),
            printerID: printerID,
            printerNativeID: printerNativeID,
            title: "Automatic drain \(label)",
            contentPath: contentURL.path,
            contentKind: "pdf",
            contentSHA256: digest,
            optionsJSON: #"{"intent":{"copies":1}}"#,
            phase: .claimed
        )
    }

    private func eventually(
        attempts: Int = 100,
        condition: @escaping @Sendable () async -> Bool
    ) async -> Bool {
        for _ in 0..<attempts {
            if await condition() { return true }
            try? await Task.sleep(nanoseconds: 10_000_000)
        }
        return false
    }

    private func requireLinkedRuntime() throws {
        guard !PiqaeNativeRuntime.linkedLibraryAvailable else { return }
        let message = "Build the PiqaeNode XCFramework before running linked-runtime tests."
        if ProcessInfo.processInfo.environment["PIQAE_REQUIRE_LINKED_RUNTIME_TESTS"] == "1" {
            XCTFail(message)
            throw LinkedRuntimeRequired.unavailable
        }
        throw XCTSkip(message)
    }

    func testFailedEmbeddedRuntimeStartReleasesProcessOwnership() async throws {
        let identity = PiqaeMemoryInstallationIdentityStore(
            id: .init(rawValue: "ins_failed_runtime_ownership")
        )
        let failedRuntime = PiqaeFakeEmbeddedRuntime(failsToStart: true)
        let failed = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                identityStore: identity,
                embeddedRuntime: failedRuntime
            )
        )

        await XCTAssertThrowsErrorAsync(try await failed.start())

        let replacementRuntime = PiqaeFakeEmbeddedRuntime()
        let replacement = PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                identityStore: identity,
                embeddedRuntime: replacementRuntime
            )
        )
        try await replacement.start()
        await replacement.stop()

        let startCount = await replacementRuntime.startCount
        let stopCount = await replacementRuntime.stopCount
        XCTAssertEqual(startCount, 1)
        XCTAssertEqual(stopCount, 1)
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

    private func wakeTestNode(
        runtime: any PiqaeEmbeddedNodeRuntime,
        id: String
    ) -> PiqaeNode {
        PiqaeNode(
            .localOnly(
                startupMode: .embedded,
                availability: .backgroundOpportunistic,
                identityStore: PiqaeMemoryInstallationIdentityStore(id: .init(rawValue: id)),
                embeddedRuntime: runtime,
                wakeRetryPolicy: .init(
                    maximumAttempts: 4,
                    initialDelaySeconds: 0.001,
                    maximumDelaySeconds: 0.001,
                    executionSafetyMarginSeconds: 0.25,
                    cloudCycleTimeoutSeconds: 1
                )
            )
        )
    }

    private func cloudOutcome(
        failed: Int,
        succeeded: Int,
        retryable: Bool,
        failure: PiqaeCloudReconcileFailureClass
    ) -> PiqaeCloudReconcileOutcome {
        PiqaeCloudReconcileOutcome(
            generation: 1,
            cloudConfigured: true,
            loopCompleted: true,
            connectorCount: failed + succeeded,
            succeededCount: succeeded,
            failedCount: failed,
            allSucceeded: failed == 0,
            partialSuccess: failed > 0 && succeeded > 0,
            retryable: retryable,
            failureClass: failure
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

    private func runtimeConnector(id: String, workspace: String) -> PiqaeRuntimeConnectorSnapshot {
        PiqaeRuntimeConnectorSnapshot(
            connectorID: id,
            controlPlaneURL: URL(string: "https://api.piqae.com")!,
            displayName: "Piqae Cloud",
            workspaceName: workspace,
            enabled: true
        )
    }
}

private actor GatedWakeReconcileRuntime: PiqaeEmbeddedNodeRuntime {
    private var callCount = 0
    private var gates: [CheckedContinuation<Void, Never>] = []
    private var released = false

    func start() async throws {}

    func stop() async throws {
        release()
    }

    func report(_ event: PiqaeHostLifecycleEvent) async throws {}

    func reconcileCloudOutcome(
        timeoutMilliseconds: UInt64
    ) async throws -> PiqaeCloudReconcileOutcome {
        callCount += 1
        if !released {
            await withTaskCancellationHandler {
                await withCheckedContinuation { continuation in
                    gates.append(continuation)
                }
            } onCancel: {
                Task { await self.release() }
            }
        }
        try Task.checkCancellation()
        return .noCloud
    }

    func reconcileCallCount() -> Int { callCount }

    func release() {
        released = true
        let pending = gates
        gates.removeAll(keepingCapacity: true)
        for gate in pending { gate.resume() }
    }
}

private actor WakeResultRecorder {
    private(set) var value: PiqaeWakeHintResult?

    func record(_ result: PiqaeWakeHintResult) {
        value = result
    }
}

private struct ApplicationIDFixture: Decodable {
    let valid: [String]
    let invalid: [String]
}

private actor LegacyBoolReconcileRuntime: PiqaeEmbeddedNodeRuntime {
    func report(_ event: PiqaeHostLifecycleEvent) async throws {}

    func reconcileCloud(timeoutMilliseconds: UInt64) async throws -> Bool {
        timeoutMilliseconds > 0
    }

    func start() async throws {}
    func stop() async throws {}
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

private final class LockedCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var storedValue = 0

    var value: Int { lock.withLock { storedValue } }

    func increment() {
        lock.withLock { storedValue += 1 }
    }
}
