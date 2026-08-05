import XCTest
@testable import PiqaeMenuCore

final class NodeConnectConsentPresentationTests: XCTestCase {
    func testWorkspaceConnectionUsesPlainLanguageAndDefaultsToAllPrinters() throws {
        let preview = try decodePreview(
            #"{"workspace_id":"wsp_internal","workspace_name":"C4 Coffee Co.","requesting_service_account_id":null,"requesting_service_name":null,"authorization_type":"workspace","environment_id":"env_internal","requested_scopes":["discover_printers","print","monitor_jobs"],"printer_grant":"select","expires_at":"2099-01-01T00:00:00Z","return_url":null}"#
        )
        let presentation = NodeConnectConsentPresentation(preview: preview)

        XCTAssertEqual(presentation.title, "Allow C4 Coffee Co. to print?")
        XCTAssertEqual(presentation.defaultGrant, .allLocalPrinters)
        XCTAssertTrue(presentation.permissionsText.contains("Send print jobs"))
        XCTAssertTrue(presentation.permissionsText.contains("View print status"))
        XCTAssertFalse(presentation.detailText.contains("wsp_internal"))
        XCTAssertFalse(presentation.detailText.contains("monitor_jobs"))
    }

    func testThirdPartyConnectionNamesServiceAndDefaultsToAllPrinters() throws {
        let preview = try decodePreview(
            #"{"workspace_id":"wsp_internal","workspace_name":"Customer","requesting_service_account_id":"psa_internal","requesting_service_name":"Design Cloud","authorization_type":"platform_customer","environment_id":"env_internal","requested_scopes":["print"],"printer_grant":"select","expires_at":"2099-01-01T00:00:00Z","return_url":"https://design.example/done"}"#
        )
        let presentation = NodeConnectConsentPresentation(preview: preview)

        XCTAssertEqual(presentation.title, "Allow Design Cloud to print?")
        XCTAssertEqual(presentation.defaultGrant, .allLocalPrinters)
        XCTAssertTrue(presentation.detailText.contains("Customer workspace"))
    }

    private func decodePreview(_ json: String) throws -> NodeConnectPreview {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try decoder.decode(NodeConnectPreview.self, from: Data(json.utf8))
    }
}
