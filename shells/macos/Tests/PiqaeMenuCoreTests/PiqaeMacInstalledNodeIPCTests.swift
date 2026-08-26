import Foundation
import PiqaeNodeKit
import XCTest
@testable import PiqaeMenuCore

final class PiqaeMacInstalledNodeIPCTests: XCTestCase {
    func testProbeRevealsOnlyAvailabilityAndProtocol() async {
        let attachment = PiqaeMacInstalledNodeIPC(
            status: { Self.status() },
            printers: { [] }
        )

        let probe = await attachment.probe()
        XCTAssertEqual(probe.state, .available(protocolVersion: 1))
    }

    func testSnapshotMapsCurrentLocalAPIWithoutClaimingPhysicalDelivery() async throws {
        let attachment = PiqaeMacInstalledNodeIPC(
            status: { Self.status() },
            printers: {
                [
                    LocalPrinter(
                        printerID: "prn_fixture",
                        nativeID: "cups-fixture",
                        name: "Virtual fixture",
                        state: "printing",
                        isDefault: true,
                        exposed: true,
                        profiles: nil,
                        queueCounts: .init(queued: 2, active: 1)
                    ),
                ]
            }
        )

        let snapshot = try await attachment.snapshot()
        XCTAssertEqual(snapshot.installationID?.rawValue, "agt_fixture")
        XCTAssertEqual(snapshot.hostMode, .userAgent)
        XCTAssertEqual(snapshot.availability, .continuousWhileAwake)
        XCTAssertEqual(snapshot.phase, .ready)
        XCTAssertEqual(snapshot.connections.first?.workspaceName, "Fixture workspace")
        XCTAssertEqual(snapshot.printers.first?.state, .busy)
        XCTAssertEqual(snapshot.printers.first?.queue?.piqaeOwned, 3)
        XCTAssertEqual(snapshot.printers.first?.queue?.external, 0)
        XCTAssertEqual(snapshot.printers.first?.queue?.unknown, 0)
    }

    func testUnavailableProbeDoesNotThrowOrStartEmbeddedWork() async {
        struct Unavailable: Error {}
        let attachment = PiqaeMacInstalledNodeIPC(
            status: { throw Unavailable() },
            printers: { [] }
        )

        let probe = await attachment.probe()
        XCTAssertEqual(probe.state, .unavailable)
    }

    private static func status() -> LocalStatus {
        LocalStatus(
            agentID: "agt_fixture",
            workspaceName: "Fixture workspace",
            version: "0.1.21",
            connection: "connected",
            queuedJobs: 2,
            activeJobs: 1,
            printerWarnings: 0,
            paused: false
        )
    }
}
