import Foundation

public struct NodeConnectPreview: Codable, Equatable, Sendable {
    public let workspaceID: String
    public let workspaceName: String
    public let requestingServiceAccountID: String?
    public let requestingServiceName: String?
    public let authorizationType: String
    public let environmentID: String
    public let requestedScopes: [String]
    public let printerGrant: String
    public let expiresAt: Date
    public let returnURL: URL?

    enum CodingKeys: String, CodingKey {
        case workspaceID = "workspace_id"
        case workspaceName = "workspace_name"
        case requestingServiceAccountID = "requesting_service_account_id"
        case requestingServiceName = "requesting_service_name"
        case authorizationType = "authorization_type"
        case environmentID = "environment_id"
        case requestedScopes = "requested_scopes"
        case printerGrant = "printer_grant"
        case expiresAt = "expires_at"
        case returnURL = "return_url"
    }
}

public enum NodeConnectAgentBridgeError: Error, LocalizedError {
    case unavailable
    case failed
    case oversizedResponse
    case expired

    public var errorDescription: String? {
        switch self {
        case .unavailable: "The installed Piqae node could not be found."
        case .failed: "Piqae could not verify or accept this connection invitation."
        case .oversizedResponse: "The Piqae node returned an oversized response."
        case .expired: "This connection invitation has expired."
        }
    }
}

public struct NodeConnectAgentBridge: Sendable {
    private static let maximumOutputBytes = 1024 * 1024
    public let executableURL: URL
    public let dataDirectory: URL

    public init(
        environment: [String: String] = ProcessInfo.processInfo.environment,
        homeDirectory: URL = FileManager.default.homeDirectoryForCurrentUser
    ) throws {
        dataDirectory = environment["PIQAE_DATA_DIR"].map(URL.init(fileURLWithPath:))
            ?? homeDirectory.appendingPathComponent("Library/Application Support/Spool")
        executableURL = environment["PIQAE_AGENT_PATH"].map(URL.init(fileURLWithPath:))
            ?? dataDirectory.appendingPathComponent("bin/piqae-agent")
        guard FileManager.default.isExecutableFile(atPath: executableURL.path) else {
            throw NodeConnectAgentBridgeError.unavailable
        }
    }

    public init(executableURL: URL, dataDirectory: URL) {
        self.executableURL = executableURL
        self.dataDirectory = dataDirectory
    }

    public func preview(capability: String, controlPlaneURL: URL, now: Date = Date()) throws -> NodeConnectPreview {
        let output = try run(flag: "--preview-connect-token-stdin", controlPlaneURL: controlPlaneURL, input: Data(capability.utf8))
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let preview = try decoder.decode(NodeConnectPreview.self, from: output)
        guard preview.expiresAt > now else { throw NodeConnectAgentBridgeError.expired }
        guard (preview.requestingServiceAccountID == nil)
            == (preview.requestingServiceName == nil)
        else { throw NodeConnectAgentBridgeError.failed }
        if let returnURL = preview.returnURL {
            let components = URLComponents(url: returnURL, resolvingAgainstBaseURL: false)
            let localHTTP = components?.scheme?.lowercased() == "http"
                && ["localhost", "127.0.0.1", "::1"].contains(components?.host?.lowercased())
            guard (components?.scheme?.lowercased() == "https" || localHTTP),
                components?.user == nil, components?.password == nil,
                components?.fragment == nil, components?.host != nil
            else { throw NodeConnectAgentBridgeError.failed }
        }
        return preview
    }

    public func accept(capability: String, controlPlaneURL: URL, printerIDs: [String]) throws {
        let input = try JSONSerialization.data(withJSONObject: [
            "token": capability,
            "printer_ids": printerIDs,
        ])
        _ = try run(flag: "--add-connector-json-stdin", controlPlaneURL: controlPlaneURL, input: input)
    }

    private func run(flag: String, controlPlaneURL: URL, input: Data) throws -> Data {
        let process = Process()
        process.executableURL = executableURL
        process.arguments = [flag, "--data-dir", dataDirectory.path, "--control-plane-url", controlPlaneURL.absoluteString]
        let stdin = Pipe()
        let stdout = Pipe()
        process.standardInput = stdin
        process.standardOutput = stdout
        let stderr = Pipe()
        process.standardError = stderr // Drained but never surfaced.
        try process.run()
        stdin.fileHandleForWriting.write(input)
        try? stdin.fileHandleForWriting.close()
        let group = DispatchGroup()
        let lock = NSLock()
        var output = Data()
        var oversized = false
        var timedOut = false
        group.enter()
        DispatchQueue.global(qos: .utility).async {
            defer { group.leave() }
            while let chunk = try? stdout.fileHandleForReading.read(upToCount: 64 * 1024),
                  !chunk.isEmpty {
                lock.lock()
                if output.count + chunk.count > Self.maximumOutputBytes {
                    oversized = true
                    lock.unlock()
                    process.terminate()
                    return
                }
                output.append(chunk)
                lock.unlock()
            }
        }
        group.enter()
        DispatchQueue.global(qos: .utility).async {
            defer { group.leave() }
            while let chunk = try? stderr.fileHandleForReading.read(upToCount: 64 * 1024),
                  !chunk.isEmpty {}
        }
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 15) {
            if process.isRunning {
                lock.lock()
                timedOut = true
                lock.unlock()
                process.terminate()
            }
        }
        process.waitUntilExit()
        group.wait()
        lock.lock()
        let result = output
        let wasOversized = oversized
        let didTimeOut = timedOut
        lock.unlock()
        guard !wasOversized else {
            throw NodeConnectAgentBridgeError.oversizedResponse
        }
        guard !didTimeOut else { throw NodeConnectAgentBridgeError.failed }
        guard process.terminationStatus == 0 else { throw NodeConnectAgentBridgeError.failed }
        return result
    }
}
