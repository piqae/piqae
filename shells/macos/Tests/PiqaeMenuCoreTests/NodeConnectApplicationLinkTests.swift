import Foundation
import XCTest
@testable import PiqaeMenuCore

final class NodeConnectApplicationLinkTests: XCTestCase {
    func testAcceptsVerifiedUniversalLinkAsPrimaryTransport() throws {
        let token = "piq_enr_0123456789abcdef0123456789abcdef"
        let link = try NodeConnectApplicationLink(
            url: XCTUnwrap(URL(string: "https://app.piqae.com/connect#enrolment_token=\(token)"))
        )
        XCTAssertEqual(link.enrolmentCapability, token)
        XCTAssertEqual(link.transport, .universalLink)
        XCTAssertNil(link.returnURL)
        XCTAssertEqual(link.capabilityFingerprint.count, 64)
    }

    func testLegacyCustomSchemeIsExplicitlyClassifiedAsDeprecated() throws {
        let token = "piq_enr_0123456789abcdef0123456789abcdef"
        let link = try NodeConnectApplicationLink(
            url: XCTUnwrap(URL(string: "piqae://connect#enrolment_token=\(token)"))
        )
        XCTAssertEqual(link.transport, .deprecatedCustomScheme)
    }

    func testAcceptsOnlySafeOptionalReturnURLs() throws {
        let token = "piq_enr_0123456789abcdef0123456789abcdef"
        let encoded = "https%3A%2F%2Fdesigner.example%2Fprinting%2Fcomplete%3Fsession%3D42"
        let link = try NodeConnectApplicationLink(url: XCTUnwrap(URL(
            string: "piqae://connect#enrolment_token=\(token)&return_url=\(encoded)"
        )))
        XCTAssertEqual(link.returnURL?.absoluteString, "https://designer.example/printing/complete?session=42")

        for value in [
            "http%3A%2F%2Fdesigner.example%2Fcomplete",
            "https%3A%2F%2Fuser%3Asecret%40designer.example%2Fcomplete",
            "https%3A%2F%2Fdesigner.example%2Fcomplete%23secret",
            "%2Frelative",
        ] {
            XCTAssertThrowsError(try NodeConnectApplicationLink(url: XCTUnwrap(URL(
                string: "piqae://connect#enrolment_token=\(token)&return_url=\(value)"
            ))), value)
        }
    }

    func testRejectsRoutesThatCanLeakOrConfuseTheCapability() throws {
        let token = "piq_enr_0123456789abcdef0123456789abcdef"
        for raw in [
            "https://connect#enrolment_token=\(token)",
            "https://app.piqae.com.evil.example/connect#enrolment_token=\(token)",
            "http://app.piqae.com/connect#enrolment_token=\(token)",
            "https://app.piqae.com/other#enrolment_token=\(token)",
            "piqae://other#enrolment_token=\(token)",
            "piqae://connect/path#enrolment_token=\(token)",
            "piqae://connect?enrolment_token=\(token)",
            "piqae://connect#enrolment_token=short",
            "piqae://connect#enrolment_token=\(token)&extra=value",
            "piqae://connect#enrolment_token=\(token)&enrolment_token=\(token)",
        ] {
            XCTAssertThrowsError(
                try NodeConnectApplicationLink(url: XCTUnwrap(URL(string: raw))),
                raw
            )
        }
    }

    func testConsentRequiresExplicitPrintersAndPermissions() throws {
        XCTAssertThrowsError(try NodeConnectConsent(printerIDs: [], permissions: [.print]))
        XCTAssertThrowsError(try NodeConnectConsent(printerIDs: ["printer-1"], permissions: []))
        XCTAssertThrowsError(try NodeConnectConsent(
            printerIDs: ["printer-1", "printer-1"], permissions: [.print]
        ))
        let consent = try NodeConnectConsent(
            printerIDs: ["printer-1"], permissions: [.discoverPrinters, .print, .monitorJobs]
        )
        XCTAssertEqual(consent.printerIDs, ["printer-1"])
    }

    func testReplayGuardRejectsConcurrentAndConsumedLinks() async throws {
        let token = "piq_enr_0123456789abcdef0123456789abcdef"
        let link = try NodeConnectApplicationLink(
            url: XCTUnwrap(URL(string: "piqae://connect#enrolment_token=\(token)"))
        )
        let guardrail = NodeConnectReplayGuard()
        let first = await guardrail.begin(link)
        let duplicate = await guardrail.begin(link)
        XCTAssertTrue(first)
        XCTAssertFalse(duplicate)
        await guardrail.finish(link, consumed: false)
        let retry = await guardrail.begin(link)
        XCTAssertTrue(retry)
        await guardrail.finish(link, consumed: true)
        let replay = await guardrail.begin(link)
        XCTAssertFalse(replay)
    }
}
