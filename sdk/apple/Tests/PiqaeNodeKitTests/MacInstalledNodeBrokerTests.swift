#if os(macOS)
import CryptoKit
import Foundation
@testable import PiqaeNodeKit
import PiqaeNodeKitTesting
import XCTest

private actor MemoryBrokerCredentialStore: PiqaeBrokerCredentialStore {
    var values: [String: Data] = [:]
    func load(account: String) -> Data? { values[account] }
    func save(_ credential: Data, account: String) { values[account] = credential }
    func remove(account: String) { values.removeValue(forKey: account) }
}

private final class MemoryConnectorKeyStore: @unchecked Sendable, PiqaeConnectorKeyStore {
    private let lock = NSLock()
    private var keys: [Data: Curve25519.Signing.PrivateKey] = [:]

    func generate(applicationScope: Data) throws -> PiqaeGeneratedConnectorKey {
        lock.withLock {
            let handle = Data(SHA256.hash(data: applicationScope))
            let key = keys[handle] ?? Curve25519.Signing.PrivateKey()
            keys[handle] = key
            return .init(handle: handle, publicKey: key.publicKey.rawRepresentation)
        }
    }

    func sign(handle: Data, message: Data) throws -> Data {
        try lock.withLock {
            guard let key = keys[handle] else { throw PiqaeNativeRuntimeError.keyUnavailable }
            return try key.signature(for: message)
        }
    }

    func delete(handle: Data) throws {
        _ = lock.withLock { keys.removeValue(forKey: handle) }
    }
}

private actor ScriptedBroker: PiqaeBrokerWireTransport {
    enum Decision { case approved, denied, expired, partial }
    let decision: Decision
    var authorizationRequests = 0
    var executeRequests = 0
    var lastConnectRequest: [String: String]?
    var requestedCapabilities: [String] = []

    init(_ decision: Decision) { self.decision = decision }

    func send(endpoint: String, request: Data) throws -> Data {
        XCTAssertTrue(endpoint.hasSuffix("node.sock"))
        let input = try XCTUnwrap(JSONSerialization.jsonObject(with: request) as? [String: Any])
        let requestID = try XCTUnwrap(input["request_id"] as? String)
        let operation = try XCTUnwrap(input["operation"] as? [String: Any])
        let type = try XCTUnwrap(operation["type"] as? String)
        let value: [String: Any]
        switch type {
        case "presence":
            value = ["type": "presence", "protocol_min": 2, "protocol_max": 4]
        case "request_authorization":
            authorizationRequests += 1
            requestedCapabilities = try XCTUnwrap(
                operation["requested_capabilities"] as? [String]
            )
            value = [
                "type": "authorization_requested",
                "authorization_id": "00000000-0000-0000-0000-000000000003",
                "nonce": "synthetic-nonce",
                "expires_unix_ms": decision == .expired ? 1 : 4_102_444_800_000,
            ]
        case "authorization_status":
            value = [
                "type": "authorization_status",
                "state": decision == .denied ? "denied" : decision == .expired ? "expired" : "approved",
            ]
        case "exchange_authorization":
            let grants = decision == .partial
                ? ["observe_status"]
                : requestedCapabilities
            value = [
                "type": "authorization_exchanged",
                "application_id": "com.example.pos",
                "token": "synthetic-token",
                "granted_capabilities": grants,
            ]
        default: throw PiqaeNodeError.invalidBrokerResponse
        }
        return try JSONSerialization.data(withJSONObject: [
            "protocol": 4, "request_id": requestID, "result": ["Ok": value],
        ])
    }

    func execute(
        endpoint: String,
        credential: Data,
        capability: Data,
        operation: Data
    ) throws -> Data {
        XCTAssertTrue(endpoint.hasSuffix("node.sock"))
        let decodedCredential = try XCTUnwrap(
            JSONSerialization.jsonObject(with: credential) as? [String: Any]
        )
        XCTAssertEqual(decodedCredential["application_id"] as? String, "com.example.pos")
        XCTAssertEqual(decodedCredential["token"] as? String, "synthetic-token")
        _ = try JSONDecoder().decode(PiqaeBrokerCapability.self, from: capability)
        let local = try XCTUnwrap(JSONSerialization.jsonObject(with: operation) as? [String: Any])
        executeRequests += 1
        return try JSONSerialization.data(withJSONObject: executeLocal(local))
    }

    private func executeLocal(_ operation: [String: Any]) throws -> [String: Any] {
        switch operation["type"] as? String {
        case "status":
            return [
                "type": "status", "agent_id": "agt_fixture", "workspace_name": "Fixture",
                "version": "0.1.22", "connection": "connected", "queued_jobs": 1,
                "active_jobs": 0, "printer_warnings": 0, "paused": false,
            ]
        case "printers":
            return [
                "type": "printers",
                "printers": [[
                    "printer_id": "prn_fixture", "native_id": "fake-printer",
                    "name": "Virtual fixture", "state": "idle", "is_default": true,
                    "exposed": true, "capability_revision": 1, "capabilities": [:],
                    "native_options": [:], "profiles": [],
                    "queue_counts": ["queued": 1, "active": 0],
                ]],
            ]
        case "sdk":
            let sdk = try XCTUnwrap(operation["operation"] as? [String: Any])
            switch sdk["type"] as? String {
            case "connect_invitation":
                lastConnectRequest = [
                    "control_plane_url": try XCTUnwrap(sdk["control_plane_url"] as? String),
                    "invitation_token": try XCTUnwrap(sdk["invitation_token"] as? String),
                    "printer_grant": try XCTUnwrap(sdk["printer_grant"] as? String),
                    "node_name": try XCTUnwrap(sdk["node_name"] as? String),
                    "hostname": try XCTUnwrap(sdk["hostname"] as? String),
                ]
                return ["type": "sdk", "data": [
                    "connector_id": "ncon_fixture", "agent_id": "agt_fixture_connector",
                    "display_name": "Example platform", "workspace_name": "Coffee shop",
                    "manage_url": "https://app.example.test/connections/ncon_fixture",
                ]]
            case "profiles":
                return ["type": "sdk", "data": [[
                    "profile_id": "prf_fixture", "revision": 2, "name": "Receipt",
                    "is_default": true,
                ]]]
            case "job_history":
                return ["type": "sdk", "data": [
                    "jobs": [[
                        "job_id": "job_fixture", "printer_id": "prn_fixture",
                        "title": "Virtual job", "state": "queued_local",
                        "native_job_id": NSNull(), "can_reprint": false,
                        "created_unix_ms": 1_700_000_000_000,
                    ]], "next_offset": NSNull(),
                ]]
            case "submit_local_job":
                return ["type": "sdk", "data": ["job_id": "job_submitted", "state": "queued_local"]]
            default: throw PiqaeNodeError.invalidBrokerResponse
            }
        default: throw PiqaeNodeError.invalidBrokerResponse
        }
    }

    func counts() -> (Int, Int) { (authorizationRequests, executeRequests) }
    func connectedRequest() -> [String: String]? { lastConnectRequest }
}

final class MacInstalledNodeBrokerTests: XCTestCase {
    private let endpoint = URL(fileURLWithPath: "/tmp/piqae-nodekit-tests/node.sock")

    func testConsentUnlocksPrinterProfileHistoryAndDurableSubmission() async throws {
        let server = ScriptedBroker(.approved)
        let store = MemoryBrokerCredentialStore()
        let broker = broker(server: server, store: store)
        let probe = await broker.probe()
        XCTAssertEqual(probe.state, .available(protocolVersion: 4))
        try await broker.prepareForAttachment()
        let snapshot = try await broker.snapshot()
        XCTAssertEqual(snapshot.printers.first?.nativeID, "fake-printer")
        let profiles = try await broker.profiles(for: .init(rawValue: "prn_fixture"))
        XCTAssertEqual(profiles.first?.name, "Receipt")
        let history = try await broker.jobHistory(offset: 0, limit: 50)
        XCTAssertEqual(history.jobs.first?.jobID.rawValue, "job_fixture")
        let receipt = try await broker.submit(
            PiqaePrintRequest(
                printerID: .init(rawValue: "prn_fixture"), title: "Virtual",
                content: .pdf(Data("%PDF-fake".utf8)), idempotencyKey: "virtual-1"
            )
        )
        XCTAssertEqual(receipt.jobID.rawValue, "job_submitted")
    }

    func testCredentialSurvivesClientRestartWithoutAnotherConsent() async throws {
        let server = ScriptedBroker(.approved)
        let store = MemoryBrokerCredentialStore()
        try await broker(server: server, store: store).prepareForAttachment()
        try await broker(server: server, store: store).prepareForAttachment()
        let counts = await server.counts()
        XCTAssertEqual(counts.0, 1)
        XCTAssertGreaterThanOrEqual(counts.1, 1)
    }

    func testExplicitResetRequestsFreshConsentWithoutChangingNodeState() async throws {
        let server = ScriptedBroker(.approved)
        let store = MemoryBrokerCredentialStore()
        let attached = broker(server: server, store: store)
        try await attached.prepareForAttachment()
        try await attached.resetAuthorization()
        try await attached.prepareForAttachment()
        let counts = await server.counts()
        XCTAssertEqual(counts.0, 2)
    }

    func testDeniedExpiredAndPartialConsentFailClosed() async throws {
        for (decision, expected) in [
            (ScriptedBroker.Decision.denied, PiqaeNodeError.brokerAuthorizationDenied),
            (.expired, .brokerAuthorizationExpired),
            (.partial, .brokerCapabilityDenied(
                "manage_connectors,observe_job_history,observe_printers,submit_local_jobs"
            )),
        ] {
            do {
                try await broker(
                    server: ScriptedBroker(decision), store: MemoryBrokerCredentialStore()
                ).prepareForAttachment()
                XCTFail("Expected broker authorization failure")
            } catch let error as PiqaeNodeError {
                XCTAssertEqual(error, expected)
            }
        }
    }

    func testAttachedInvitationUsesAuthenticatedConnectorCommand() async throws {
        let server = ScriptedBroker(.approved)
        let attached = broker(server: server, store: MemoryBrokerCredentialStore())
        try await attached.prepareForAttachment()
        let connection = try await attached.connect(
            PiqaeEnrollmentRequest(
                authorityURL: try XCTUnwrap(URL(string: "https://api.example.test")),
                invitation: try PiqaeSensitiveString("short-lived-invitation"),
                installationID: .init(rawValue: "ins_installed"),
                hostMode: .userAgent,
                availability: .continuousWhileAwake
            )
        )
        XCTAssertEqual(connection.id.rawValue, "ncon_fixture")
        XCTAssertEqual(connection.workspaceName, "Coffee shop")
        XCTAssertEqual(connection.state, .connected)
        let captured = await server.connectedRequest()
        let sent = try XCTUnwrap(captured)
        XCTAssertEqual(sent["control_plane_url"], "https://api.example.test")
        XCTAssertEqual(sent["invitation_token"], "short-lived-invitation")
        XCTAssertEqual(sent["printer_grant"], "all_local_printers")
        XCTAssertEqual(sent["node_name"], "Example POS")
        XCTAssertFalse(try XCTUnwrap(sent["hostname"]).isEmpty)
    }

    func testAttachedInvitationRequiresExplicitConnectorCapability() async throws {
        let server = ScriptedBroker(.approved)
        let attached = broker(
            server: server,
            store: MemoryBrokerCredentialStore(),
            requiredCapabilities: [
                .observeStatus, .observePrinters, .observeJobHistory, .submitLocalJobs,
            ]
        )
        try await attached.prepareForAttachment()
        do {
            _ = try await attached.connect(
                PiqaeEnrollmentRequest(
                    authorityURL: try XCTUnwrap(URL(string: "https://api.example.test")),
                    invitation: try PiqaeSensitiveString("short-lived-invitation"),
                    installationID: .init(rawValue: "ins_installed"),
                    hostMode: .userAgent,
                    availability: .continuousWhileAwake
                )
            )
            XCTFail("Expected connector capability denial")
        } catch let error as PiqaeNodeError {
            XCTAssertEqual(error, .brokerCapabilityDenied("manage_connectors"))
        }
    }

    func testAutomaticDesktopFallbackMustBeExplicit() async throws {
        let unavailable = PiqaeFakeInstalledNodeIPC(protocolVersion: nil, snapshot: .init(
            installationID: nil, hostMode: .userAgent, availability: .continuousWhileAwake,
            phase: .stopped, connections: [], printers: [], lastUpdatedAt: Date()
        ))
        let denied = PiqaeNode(.localOnly(startupMode: .automatic, installedNodeIPC: unavailable))
        do { try await denied.start(); XCTFail("Expected fail-closed attach") }
        catch let error as PiqaeNodeError { XCTAssertEqual(error, .installedNodeUnavailable) }

        let allowed = PiqaeNode(.localOnly(
            startupMode: .automatic,
            identityStore: PiqaeMemoryInstallationIdentityStore(id: .init(rawValue: "ins_fallback")),
            installedNodeIPC: unavailable, allowsEmbeddedFallback: true
        ))
        try await allowed.start()
        let snapshot = await allowed.snapshot()
        XCTAssertEqual(snapshot.hostMode, .embeddedApplication)
        await allowed.stop()
    }

    func testConnectorSignerCallbacksExposeOnlyHandlePublicKeyAndSignature() throws {
        let store = MemoryConnectorKeyStore()
        let context = PiqaeConnectorKeyCallbackContext(store: store)
        let opaque = Unmanaged.passUnretained(context).toOpaque()
        let scope = Array("com.example.pos".utf8)
        let handleCapacity = 512
        let publicKeyLength = 32
        let signatureLength = 64
        var handle = [UInt8](repeating: 0, count: handleCapacity)
        var handleLength = 0
        var publicKey = [UInt8](repeating: 0, count: publicKeyLength)
        let generated = scope.withUnsafeBufferPointer { scopeBytes in
            handle.withUnsafeMutableBufferPointer { handleBytes in
                publicKey.withUnsafeMutableBufferPointer { publicBytes in
                    piqaeAppleGenerateConnectorKey(
                        opaque, scopeBytes.baseAddress, scope.count,
                        handleBytes.baseAddress, handleCapacity, &handleLength,
                        publicBytes.baseAddress, publicKeyLength
                    )
                }
            }
        }
        XCTAssertEqual(generated, 0)
        XCTAssertEqual(handleLength, 32)
        XCTAssertNotEqual(publicKey, [UInt8](repeating: 0, count: 32))

        let message = Array("piqae-connector-bind-v1".utf8)
        var signature = [UInt8](repeating: 0, count: signatureLength)
        let signed = handle.withUnsafeBufferPointer { handleBytes in
            message.withUnsafeBufferPointer { messageBytes in
                signature.withUnsafeMutableBufferPointer { signatureBytes in
                    piqaeAppleSignConnector(
                        opaque, handleBytes.baseAddress, handleLength,
                        messageBytes.baseAddress, message.count,
                        signatureBytes.baseAddress, signatureLength
                    )
                }
            }
        }
        XCTAssertEqual(signed, 0)
        XCTAssertNotEqual(signature, [UInt8](repeating: 0, count: 64))
        let deleted = handle.withUnsafeBufferPointer {
            piqaeAppleDeleteConnectorKey(opaque, $0.baseAddress, handleLength)
        }
        XCTAssertEqual(deleted, 0)
        let rejected = handle.withUnsafeBufferPointer { handleBytes in
            message.withUnsafeBufferPointer { messageBytes in
                signature.withUnsafeMutableBufferPointer { signatureBytes in
                    piqaeAppleSignConnector(
                        opaque, handleBytes.baseAddress, handleLength,
                        messageBytes.baseAddress, message.count,
                        signatureBytes.baseAddress, signatureLength
                    )
                }
            }
        }
        XCTAssertNotEqual(rejected, 0)
    }

    private func broker(
        server: ScriptedBroker,
        store: MemoryBrokerCredentialStore,
        requiredCapabilities: Set<PiqaeBrokerCapability> = [
            .observeStatus, .observePrinters, .observeJobHistory, .submitLocalJobs,
            .manageConnectors,
        ]
    ) -> PiqaeMacInstalledNodeBroker {
        PiqaeMacInstalledNodeBroker(
            endpoint: endpoint,
            application: try! PiqaeBrokerApplication(
                applicationID: "com.example.pos", displayName: "Example POS"
            ),
            requiredCapabilities: requiredCapabilities,
            credentialStore: store, transport: server
        )
    }
}
#endif
