import XCTest
@testable import PiqaeMenuCore

final class UpdateReadinessTests: XCTestCase {
    func testUpdateMenuTruthfullyDescribesAppOnlyUpdateChannel() {
        XCTAssertEqual(
            UpdateMenuPresentation.unavailable.title,
            "App updates unavailable in this build"
        )
        XCTAssertFalse(UpdateMenuPresentation.unavailable.canOpenUpdater)
        XCTAssertEqual(
            UpdateMenuPresentation.readyToCheck.title,
            "Check for Piqae App Update…"
        )
        XCTAssertTrue(UpdateMenuPresentation.readyToCheck.canOpenUpdater)
        XCTAssertEqual(
            UpdateMenuPresentation.available(version: "1.2.3").title,
            "Piqae App 1.2.3 Available…"
        )
        XCTAssertTrue(
            UpdateMenuPresentation.available(version: "1.2.3").canOpenUpdater
        )
        XCTAssertEqual(
            UpdateMenuPresentation.waitingForIdle(version: "1.2.3").title,
            "Piqae App 1.2.3 Waiting for Idle"
        )
        XCTAssertFalse(
            UpdateMenuPresentation.waitingForIdle(version: "1.2.3").canOpenUpdater
        )
        XCTAssertEqual(
            UpdateMenuPresentation.available(
                version: " \n1234567890123456789012345678901234567890\r"
            ).title,
            "Piqae App 12345678901234567890123456789012 Available…"
        )
    }

    func testRequiresAgentStatusBeforeReplacingNativeComponents() {
        XCTAssertEqual(
            UpdateHandoffReadiness(status: nil, foregroundOperation: false),
            .agentUnavailable
        )
    }

    func testReadyOnlyWhenQueueAndForegroundWorkAreIdle() throws {
        let idle = try status(queued: 0, active: 0)
        XCTAssertTrue(
            UpdateHandoffReadiness(
                status: idle,
                foregroundOperation: false
            ).canReplaceNativeComponents
        )
        XCTAssertEqual(
            UpdateHandoffReadiness(status: idle, foregroundOperation: true),
            .busy(queuedJobs: 0, activeJobs: 0, foregroundOperation: true)
        )
    }

    func testBusyPreservesQueueCountsForDiagnostics() throws {
        XCTAssertEqual(
            UpdateHandoffReadiness(
                status: try status(queued: 3, active: 1),
                foregroundOperation: false
            ),
            .busy(queuedJobs: 3, activeJobs: 1, foregroundOperation: false)
        )
    }

    private func status(queued: UInt32, active: UInt32) throws -> LocalStatus {
        try JSONDecoder().decode(
            LocalStatus.self,
            from: Data(
                """
                {
                  "version":"0.1.0",
                  "connection":"local_only",
                  "queued_jobs":\(queued),
                  "active_jobs":\(active),
                  "printer_warnings":0,
                  "paused":false
                }
                """.utf8
            )
        )
    }
}
