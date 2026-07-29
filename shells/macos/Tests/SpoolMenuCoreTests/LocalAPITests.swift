import Foundation
import XCTest
@testable import SpoolMenuCore

final class LocalAPITests: XCTestCase {
    func testConfigurationUsesExplicitEnvironment() throws {
        let configuration = try LocalAPIConfiguration(
            environment: [
                "SPOOL_LOCAL_API_URL": "http://localhost:49100",
                "SPOOL_DATA_DIR": "/var/lib/spool",
            ],
            homeDirectory: URL(fileURLWithPath: "/Users/test")
        )

        XCTAssertEqual(configuration.baseURL.absoluteString, "http://localhost:49100")
        XCTAssertEqual(configuration.tokenFile.path, "/var/lib/spool/local.token")
    }

    func testConfigurationPrefersExplicitTokenFile() throws {
        let configuration = try LocalAPIConfiguration(
            environment: [
                "SPOOL_LOCAL_TOKEN_FILE": "/tmp/spool-shell-token",
                "SPOOL_DATA_DIR": "/var/lib/spool",
            ]
        )

        XCTAssertEqual(configuration.tokenFile.path, "/tmp/spool-shell-token")
    }

    func testConfigurationRejectsRemoteOrCredentialedURLs() {
        for rawURL in [
            "https://127.0.0.1:39100",
            "http://example.com:39100",
            "http://localhost.evil:39100",
            "http://user:password@localhost:39100",
        ] {
            XCTAssertThrowsError(
                try LocalAPIConfiguration(
                    baseURL: XCTUnwrap(URL(string: rawURL)),
                    tokenFile: URL(fileURLWithPath: "/tmp/token")
                ),
                rawURL
            )
        }
    }

    func testPrinterDecodingToleratesMissingEvolutionFields() throws {
        let data = Data(
            """
            {
              "printer_id": "prn_test",
              "name": "Packing",
              "state": "idle",
              "is_default": true
            }
            """.utf8
        )

        let printer = try JSONDecoder().decode(LocalPrinter.self, from: data)
        XCTAssertEqual(printer.printerID, "prn_test")
        XCTAssertNil(printer.exposed)
        XCTAssertNil(printer.queueCounts)
        XCTAssertNil(printer.profiles)
    }

    func testStatusDecodesAgentContract() throws {
        let data = Data(
            """
            {
              "agent_id": "agt_test",
              "workspace_name": "C4",
              "version": "0.1.0",
              "connection": "connected",
              "queued_jobs": 2,
              "active_jobs": 1,
              "printer_warnings": 0,
              "paused": false
            }
            """.utf8
        )

        let status = try JSONDecoder().decode(LocalStatus.self, from: data)
        XCTAssertEqual(status.connection, "connected")
        XCTAssertEqual(status.queuedJobs, 2)
        XCTAssertEqual(status.activeJobs, 1)
    }

    func testPerPrinterRoutesAndAuthentication() async throws {
        let tokenFile = FileManager.default.temporaryDirectory
            .appendingPathComponent("spool-menu-\(UUID().uuidString).token")
        try Data("test-secret\n".utf8).write(to: tokenFile, options: .atomic)
        defer { try? FileManager.default.removeItem(at: tokenFile) }

        let sessionConfiguration = URLSessionConfiguration.ephemeral
        sessionConfiguration.protocolClasses = [StubURLProtocol.self]
        let client = LocalAPIClient(
            configuration: try LocalAPIConfiguration(
                baseURL: XCTUnwrap(URL(string: "http://127.0.0.1:39100")),
                tokenFile: tokenFile
            ),
            session: URLSession(configuration: sessionConfiguration)
        )

        StubURLProtocol.handler = { request in
            XCTAssertEqual(request.url?.path, "/v1/local/printers/prn_test/profiles")
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer test-secret")
            return Self.response(
                for: request,
                body: """
                [{"profile_id":"profile_a4","revision":1,"name":"A4","is_default":true,"options":{}}]
                """
            )
        }
        let profiles = try await client.profiles(printerID: "prn_test")
        XCTAssertEqual(profiles.first?.profileID, "profile_a4")

        StubURLProtocol.handler = { request in
            XCTAssertEqual(request.url?.path, "/v1/local/printers/prn_test/queue")
            return Self.response(
                for: request,
                body: """
                {
                  "printer_id":"prn_test",
                  "local_jobs":[{
                    "job_id":"job_1",
                    "sequence":1,
                    "title":"Test",
                    "state":"queued_local",
                    "native_job_id":null
                  }],
                  "native_jobs":[]
                }
                """
            )
        }
        let jobs = try await client.queue(printerID: "prn_test")
        XCTAssertEqual(jobs.first?.jobID, "job_1")

        StubURLProtocol.handler = { request in
            XCTAssertEqual(request.httpMethod, "PUT")
            XCTAssertEqual(request.url?.path, "/v1/local/printers/prn_test/exposure")
            XCTAssertEqual(
                try JSONSerialization.jsonObject(with: try Self.body(of: request)) as? [String: Bool],
                ["exposed": true]
            )
            return Self.response(for: request, status: 200, body: "{}")
        }
        try await client.setExposure(printerID: "prn_test", exposed: true)

        StubURLProtocol.handler = { request in
            XCTAssertEqual(request.httpMethod, "POST")
            XCTAssertEqual(request.url?.path, "/v1/local/printers/prn_test/test-page")
            XCTAssertEqual(
                try JSONSerialization.jsonObject(with: try Self.body(of: request))
                    as? [String: String],
                ["profile_id": "profile_a4"]
            )
            return Self.response(
                for: request,
                status: 202,
                body: #"{"job_id":"job_test","state":"queued_local"}"#
            )
        }
        let accepted = try await client.submitDriverTest(
            printerID: "prn_test",
            profileID: "profile_a4"
        )
        XCTAssertEqual(accepted.jobID, "job_test")
    }

    private static func response(
        for request: URLRequest,
        status: Int = 200,
        body: String
    ) -> (HTTPURLResponse, Data) {
        (
            HTTPURLResponse(
                url: request.url!,
                statusCode: status,
                httpVersion: "HTTP/1.1",
                headerFields: ["Content-Type": "application/json"]
            )!,
            Data(body.utf8)
        )
    }

    private static func body(of request: URLRequest) throws -> Data {
        if let body = request.httpBody {
            return body
        }
        let stream = try XCTUnwrap(request.httpBodyStream)
        stream.open()
        defer { stream.close() }
        var result = Data()
        var buffer = [UInt8](repeating: 0, count: 4096)
        while true {
            let count = stream.read(&buffer, maxLength: buffer.count)
            if count < 0 {
                throw try XCTUnwrap(stream.streamError)
            }
            if count == 0 {
                return result
            }
            result.append(buffer, count: count)
        }
    }
}

private final class StubURLProtocol: URLProtocol {
    static var handler: ((URLRequest) throws -> (HTTPURLResponse, Data))?

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        do {
            let (response, data) = try XCTUnwrap(Self.handler)(request)
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}
}
