import Foundation
import XCTest
@testable import PiqaeMenuCore

final class NodeConnectAgentBridgeTests: XCTestCase {
    func testCapabilityTravelsOnStdinAndNeverArgv() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let script = directory.appendingPathComponent("fake-agent")
        try """
        #!/bin/sh
        case "$*" in *piq_enr_*) exit 91;; esac
        read token
        printf '{"workspace_id":"ws_1","workspace_name":"Customer","requesting_service_account_id":"psa_1","requesting_service_name":"Designer","return_url":"https://designer.example/done","environment_id":"env_1","requested_scopes":["print"],"printer_grant":"select","expires_at":"2099-01-01T00:00:00Z"}'
        """.write(to: script, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
        let bridge = NodeConnectAgentBridge(executableURL: script, dataDirectory: directory)
        let preview = try bridge.preview(capability: "piq_enr_0123456789abcdef0123456789abcdef")
        XCTAssertEqual(preview.requestingServiceName, "Designer")
        XCTAssertEqual(preview.returnURL?.host, "designer.example")
        let tamperedLink = try NodeConnectApplicationLink(url: XCTUnwrap(URL(
            string: "piqae://connect#enrolment_token=piq_enr_0123456789abcdef0123456789abcdef&return_url=https%3A%2F%2Fevil.example%2Fsteal"
        )))
        XCTAssertEqual(tamperedLink.returnURL?.host, "evil.example")
        XCTAssertNotEqual(tamperedLink.returnURL, preview.returnURL)
    }

    func testExpiredPreviewFailsClosed() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let script = directory.appendingPathComponent("fake-agent")
        try """
        #!/bin/sh
        cat >/dev/null
        printf '{"workspace_id":"ws_1","workspace_name":"Customer","requesting_service_account_id":null,"requesting_service_name":null,"return_url":null,"environment_id":"env_1","requested_scopes":["print"],"printer_grant":"select","expires_at":"2020-01-01T00:00:00Z"}'
        """.write(to: script, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
        XCTAssertThrowsError(try NodeConnectAgentBridge(
            executableURL: script, dataDirectory: directory
        ).preview(capability: "piq_enr_0123456789abcdef0123456789abcdef"))
    }
}
