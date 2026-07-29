import Foundation

public struct LocalAPIConfiguration: Equatable, Sendable {
    public static let defaultAPIURL = URL(string: "http://127.0.0.1:39100")!

    public let baseURL: URL
    public let tokenFile: URL

    public init(baseURL: URL, tokenFile: URL) throws {
        guard Self.isSafeLoopbackURL(baseURL) else {
            throw LocalAPIError.invalidConfiguration(
                "SPOOL_LOCAL_API_URL must be an HTTP loopback URL"
            )
        }
        self.baseURL = baseURL
        self.tokenFile = tokenFile
    }

    public init(
        environment: [String: String] = ProcessInfo.processInfo.environment,
        homeDirectory: URL = FileManager.default.homeDirectoryForCurrentUser
    ) throws {
        let baseURL: URL
        if let rawURL = environment["SPOOL_LOCAL_API_URL"], !rawURL.isEmpty {
            guard let parsed = URL(string: rawURL) else {
                throw LocalAPIError.invalidConfiguration(
                    "SPOOL_LOCAL_API_URL is not a valid URL"
                )
            }
            baseURL = parsed
        } else {
            baseURL = Self.defaultAPIURL
        }

        let tokenFile: URL
        if let path = environment["SPOOL_LOCAL_TOKEN_FILE"], !path.isEmpty {
            tokenFile = URL(fileURLWithPath: path)
        } else if let dataDirectory = environment["SPOOL_DATA_DIR"], !dataDirectory.isEmpty {
            tokenFile = URL(fileURLWithPath: dataDirectory)
                .appendingPathComponent("local.token", isDirectory: false)
        } else {
            tokenFile = homeDirectory
                .appendingPathComponent("Library/Application Support/Spool", isDirectory: true)
                .appendingPathComponent("local.token", isDirectory: false)
        }

        try self.init(baseURL: baseURL, tokenFile: tokenFile)
    }

    private static func isSafeLoopbackURL(_ url: URL) -> Bool {
        guard
            url.scheme?.lowercased() == "http",
            let host = url.host?.lowercased(),
            ["127.0.0.1", "localhost", "::1"].contains(host),
            url.user == nil,
            url.password == nil,
            url.query == nil,
            url.fragment == nil
        else {
            return false
        }
        return true
    }
}

public enum LocalAPIError: Error, LocalizedError, Equatable {
    case invalidConfiguration(String)
    case tokenUnavailable(String)
    case invalidResponse
    case responseTooLarge
    case rejected(status: Int, message: String)

    public var errorDescription: String? {
        switch self {
        case let .invalidConfiguration(message), let .tokenUnavailable(message):
            message
        case .invalidResponse:
            "The local agent returned an invalid response."
        case .responseTooLarge:
            "The local agent response exceeded the shell safety limit."
        case let .rejected(_, message):
            message
        }
    }
}

public struct LocalStatus: Codable, Equatable, Sendable {
    public let agentID: String?
    public let workspaceName: String?
    public let version: String
    public let connection: String
    public let queuedJobs: UInt32
    public let activeJobs: UInt32
    public let printerWarnings: UInt32
    public let paused: Bool

    enum CodingKeys: String, CodingKey {
        case agentID = "agent_id"
        case workspaceName = "workspace_name"
        case version
        case connection
        case queuedJobs = "queued_jobs"
        case activeJobs = "active_jobs"
        case printerWarnings = "printer_warnings"
        case paused
    }
}

public struct LocalPrinter: Codable, Equatable, Identifiable, Sendable {
    public let printerID: String
    public let nativeID: String?
    public let name: String
    public let state: String
    public let isDefault: Bool
    public let exposed: Bool?
    public let profiles: [LocalPrintProfile]?
    public let queueCounts: LocalPrinterQueueCounts?

    public var id: String { printerID }

    enum CodingKeys: String, CodingKey {
        case printerID = "printer_id"
        case nativeID = "native_id"
        case name
        case state
        case isDefault = "is_default"
        case exposed
        case profiles
        case queueCounts = "queue_counts"
    }
}

public struct LocalPrinterQueueCounts: Codable, Equatable, Sendable {
    public let queued: UInt32
    public let active: UInt32
}

public struct LocalPrintProfile: Codable, Equatable, Identifiable, Sendable {
    public let profileID: String
    public let revision: UInt64?
    public let name: String
    public let isDefault: Bool?
    public let status: String?
    public let stockID: String?
    public let lastValidatedUnixMS: Int64?

    public var id: String { profileID }

    enum CodingKeys: String, CodingKey {
        case profileID = "profile_id"
        case revision
        case name
        case isDefault = "is_default"
        case status
        case stockID = "stock_id"
        case lastValidatedUnixMS = "last_validated_unix_ms"
    }
}

public struct LocalQueueJob: Codable, Equatable, Identifiable, Sendable {
    public let jobID: String
    public let sequence: Int64?
    public let title: String
    public let state: String
    public let createdUnixMS: Int64?

    public var id: String { jobID }

    enum CodingKeys: String, CodingKey {
        case jobID = "job_id"
        case sequence
        case title
        case state
        case createdUnixMS = "created_unix_ms"
    }
}

public struct LocalJobAccepted: Codable, Equatable, Sendable {
    public let jobID: String
    public let state: String

    enum CodingKeys: String, CodingKey {
        case jobID = "job_id"
        case state
    }
}

private struct APIMessage: Codable {
    let message: String?
}

private struct ProfileCollection: Codable {
    let profiles: [LocalPrintProfile]
}

private struct QueueCollection: Codable {
    let printerID: String
    let localJobs: [LocalQueueJob]

    enum CodingKeys: String, CodingKey {
        case printerID = "printer_id"
        case localJobs = "local_jobs"
    }
}

private struct ExposureUpdate: Encodable {
    let exposed: Bool
}

private struct TestPageRequest: Encodable {
    let profileID: String

    enum CodingKeys: String, CodingKey {
        case profileID = "profile_id"
    }
}

private struct ProfileCaptureSessionRequest: Encodable {
    let operation: LocalProfileCaptureOperation
    let profileID: String?
    let expectedRevision: UInt64?

    enum CodingKeys: String, CodingKey {
        case operation
        case profileID = "profile_id"
        case expectedRevision = "expected_revision"
    }
}

public final class LocalAPIClient: @unchecked Sendable {
    private static let maximumResponseBytes = 1024 * 1024
    private static let maximumCaptureResponseBytes = 8 * 1024 * 1024

    public let configuration: LocalAPIConfiguration
    private let session: URLSession

    public init(configuration: LocalAPIConfiguration, session: URLSession? = nil) {
        self.configuration = configuration
        if let session {
            self.session = session
        } else {
            let sessionConfiguration = URLSessionConfiguration.ephemeral
            sessionConfiguration.timeoutIntervalForRequest = 3
            sessionConfiguration.timeoutIntervalForResource = 5
            sessionConfiguration.requestCachePolicy = .reloadIgnoringLocalCacheData
            sessionConfiguration.urlCache = nil
            self.session = URLSession(configuration: sessionConfiguration)
        }
    }

    public func status() async throws -> LocalStatus {
        try await request(path: "/v1/local/status")
    }

    public func printers() async throws -> [LocalPrinter] {
        try await request(path: "/v1/local/printers")
    }

    public func profiles(printerID: String) async throws -> [LocalPrintProfile] {
        let data = try await requestData(
            path: "/v1/local/printers/\(try pathComponent(printerID))/profiles"
        )
        if let collection = try? decoder.decode(ProfileCollection.self, from: data) {
            return collection.profiles
        }
        return try decoder.decode([LocalPrintProfile].self, from: data)
    }

    public func queue(printerID: String) async throws -> [LocalQueueJob] {
        let data = try await requestData(
            path: "/v1/local/printers/\(try pathComponent(printerID))/queue"
        )
        if let collection = try? decoder.decode(QueueCollection.self, from: data) {
            return collection.localJobs
        }
        return try decoder.decode([LocalQueueJob].self, from: data)
    }

    public func setExposure(printerID: String, exposed: Bool) async throws {
        try await sendWithoutResponse(
            method: "PUT",
            path: "/v1/local/printers/\(try pathComponent(printerID))/exposure",
            body: try encoder.encode(ExposureUpdate(exposed: exposed))
        )
    }

    public func pause() async throws {
        try await sendWithoutResponse(method: "POST", path: "/v1/local/pause")
    }

    public func resume() async throws {
        try await sendWithoutResponse(method: "POST", path: "/v1/local/resume")
    }

    public func submitDriverTest(
        printerID: String,
        profileID: String
    ) async throws -> LocalJobAccepted {
        try await request(
            method: "POST",
            path: "/v1/local/printers/\(try pathComponent(printerID))/test-page",
            body: try encoder.encode(TestPageRequest(profileID: profileID))
        )
    }

    public func createProfileCaptureSession(
        printerID: String,
        operation: LocalProfileCaptureOperation,
        profileID: String? = nil,
        expectedRevision: UInt64? = nil
    ) async throws -> LocalProfileCaptureSession {
        if operation != .create, profileID == nil {
            throw LocalAPIError.invalidConfiguration(
                "Editing or cloning requires a profile identifier."
            )
        }
        return try await request(
            method: "POST",
            path: "/v1/local/printers/\(try pathComponent(printerID))/profile-capture-sessions",
            body: try encoder.encode(
                ProfileCaptureSessionRequest(
                    operation: operation,
                    profileID: profileID,
                    expectedRevision: expectedRevision
                )
            ),
            maximumResponseBytes: Self.maximumCaptureResponseBytes
        )
    }

    public func completeProfileCapture(
        session: LocalProfileCaptureSession,
        completion: LocalProfileCaptureCompletion
    ) async throws -> LocalPrintProfile {
        try await request(
            method: "POST",
            path: "/v1/local/profile-capture-sessions/"
                + "\(try pathComponent(session.sessionID))/complete",
            body: try encoder.encode(completion),
            additionalHeaders: ["X-Spool-Capture-Token": session.captureToken]
        )
    }

    public func cancelProfileCapture(session: LocalProfileCaptureSession) async throws {
        try await sendWithoutResponse(
            method: "DELETE",
            path: "/v1/local/profile-capture-sessions/\(try pathComponent(session.sessionID))",
            additionalHeaders: ["X-Spool-Capture-Token": session.captureToken]
        )
    }

    private var decoder: JSONDecoder { JSONDecoder() }
    private var encoder: JSONEncoder { JSONEncoder() }

    private func request<Response: Decodable>(
        method: String = "GET",
        path: String,
        body: Data? = nil,
        additionalHeaders: [String: String] = [:],
        maximumResponseBytes: Int = LocalAPIClient.maximumResponseBytes
    ) async throws -> Response {
        let data = try await requestData(
            method: method,
            path: path,
            body: body,
            additionalHeaders: additionalHeaders,
            maximumResponseBytes: maximumResponseBytes
        )
        do {
            return try decoder.decode(Response.self, from: data)
        } catch {
            throw LocalAPIError.invalidResponse
        }
    }

    private func sendWithoutResponse(
        method: String,
        path: String,
        body: Data? = nil,
        additionalHeaders: [String: String] = [:]
    ) async throws {
        _ = try await requestData(
            method: method,
            path: path,
            body: body,
            additionalHeaders: additionalHeaders
        )
    }

    private func requestData(
        method: String = "GET",
        path: String,
        body: Data? = nil,
        additionalHeaders: [String: String] = [:],
        maximumResponseBytes: Int = LocalAPIClient.maximumResponseBytes
    ) async throws -> Data {
        var request = URLRequest(url: endpoint(path))
        request.httpMethod = method
        request.timeoutInterval = 3
        request.cachePolicy = .reloadIgnoringLocalCacheData
        request.setValue("Bearer \(try readToken())", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        for (name, value) in additionalHeaders {
            request.setValue(value, forHTTPHeaderField: name)
        }
        if let body {
            request.httpBody = body
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }

        let (bytes, response) = try await session.bytes(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw LocalAPIError.invalidResponse
        }
        if http.expectedContentLength > maximumResponseBytes {
            throw LocalAPIError.responseTooLarge
        }
        var data = Data()
        if http.expectedContentLength > 0 {
            data.reserveCapacity(Int(http.expectedContentLength))
        }
        for try await byte in bytes {
            guard data.count < maximumResponseBytes else {
                throw LocalAPIError.responseTooLarge
            }
            data.append(byte)
        }
        guard (200 ... 299).contains(http.statusCode) else {
            let message = (try? decoder.decode(APIMessage.self, from: data).message)
                ?? HTTPURLResponse.localizedString(forStatusCode: http.statusCode)
            throw LocalAPIError.rejected(status: http.statusCode, message: message)
        }
        return data
    }

    private func endpoint(_ path: String) -> URL {
        path.split(separator: "/", omittingEmptySubsequences: true)
            .reduce(configuration.baseURL) { url, component in
                url.appendingPathComponent(String(component), isDirectory: false)
            }
    }

    private func pathComponent(_ value: String) throws -> String {
        // Resource IDs are opaque path segments. Refuse separators and dot
        // segments instead of silently changing which resource is addressed.
        guard
            !value.isEmpty,
            value != ".",
            value != "..",
            !value.contains("/"),
            !value.contains("\\")
        else {
            throw LocalAPIError.invalidConfiguration(
                "The local agent returned an invalid resource identifier."
            )
        }
        return value
    }

    private func readToken() throws -> String {
        do {
            let values = try configuration.tokenFile.resourceValues(forKeys: [.fileSizeKey])
            if let size = values.fileSize, size > 1024 {
                throw LocalAPIError.tokenUnavailable("The local agent token is oversized.")
            }
            let value = try String(contentsOf: configuration.tokenFile, encoding: .utf8)
                .trimmingCharacters(in: .whitespacesAndNewlines)
            guard !value.isEmpty, value.utf8.count <= 1024 else {
                throw LocalAPIError.tokenUnavailable("The local agent token is empty or oversized.")
            }
            return value
        } catch let error as LocalAPIError {
            throw error
        } catch {
            throw LocalAPIError.tokenUnavailable(
                "Cannot read the local agent token. Check SPOOL_LOCAL_TOKEN_FILE."
            )
        }
    }
}
