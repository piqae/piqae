import Foundation
import Darwin

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

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        workspaceID = try values.decode(String.self, forKey: .workspaceID)
        workspaceName = try values.decode(String.self, forKey: .workspaceName)
        requestingServiceAccountID = try values.decodeIfPresent(String.self, forKey: .requestingServiceAccountID)
        requestingServiceName = try values.decodeIfPresent(String.self, forKey: .requestingServiceName)
        authorizationType = try values.decodeIfPresent(String.self, forKey: .authorizationType)
            ?? (requestingServiceAccountID == nil ? "workspace" : "platform_customer")
        environmentID = try values.decode(String.self, forKey: .environmentID)
        requestedScopes = try values.decode([String].self, forKey: .requestedScopes)
        printerGrant = try values.decode(String.self, forKey: .printerGrant)
        expiresAt = try values.decode(Date.self, forKey: .expiresAt)
        returnURL = try values.decodeIfPresent(URL.self, forKey: .returnURL)
    }
}

public enum NodeConnectAgentBridgeError: Error, LocalizedError {
    case unavailable
    case failed
    case oversizedResponse
    case expired
    case invitationRejected
    case identityRejected
    case nativeProcessFailure(NativeProcessEvidence)

    public var errorDescription: String? {
        switch self {
        case .unavailable: "The installed Piqae node could not be found."
        case .failed: "Piqae could not verify or accept this connection invitation."
        case .oversizedResponse: "The Piqae node returned an oversized response."
        case .expired: "This connection invitation has expired. Return to the service and create a new one."
        case .invitationRejected: "This invitation is no longer valid. Return to the service and create a new connection."
        case .identityRejected: "This installation could not prove its identity. Open Piqae Diagnostics and retry the connection."
        case .nativeProcessFailure: "The Piqae node helper failed. Open Piqae Diagnostics and retry."
        }
    }
}

public struct NativeProcessEvidence: Equatable, Sendable, CustomStringConvertible {
    public let classification: String
    public let stderrBytes: Int
    public let inspectedBytes: Int
    public let drainComplete: Bool

    public var description: String {
        "stderr_class=\(classification), stderr_bytes=\(stderrBytes), inspected_bytes=\(inspectedBytes), stderr_drain_complete=\(drainComplete)"
    }
}

public enum NodePrinterGrant: String, Codable, Equatable, Sendable {
    case allLocalPrinters = "all_local_printers"
    case selectedPrinters = "selected_printers"
}

public struct NodePrinterAuthorization: Equatable, Sendable {
    public let grant: NodePrinterGrant
    public let printerIDs: [String]

    public init(grant: NodePrinterGrant, printerIDs: [String] = []) {
        self.grant = grant
        self.printerIDs = printerIDs
    }
}

public struct NodeConnectAgentBridge: Sendable {
    private static let maximumOutputBytes = 1024 * 1024
    public let executableURL: URL
    public let dataDirectory: URL
    private let processTimeout: TimeInterval

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
        processTimeout = 15
    }

    public init(executableURL: URL, dataDirectory: URL, processTimeout: TimeInterval = 15) {
        self.executableURL = executableURL
        self.dataDirectory = dataDirectory
        self.processTimeout = processTimeout
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
        let expectedAuthorizationType = preview.requestingServiceAccountID == nil
            ? "workspace" : "platform_customer"
        guard preview.authorizationType == expectedAuthorizationType else {
            throw NodeConnectAgentBridgeError.failed
        }
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

    public func accept(
        capability: String,
        controlPlaneURL: URL,
        authorization: NodePrinterAuthorization
    ) throws {
        let input = try JSONSerialization.data(withJSONObject: [
            "token": capability,
            "printer_grant": authorization.grant.rawValue,
            "printer_ids": authorization.printerIDs,
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
        var diagnostic = Data()
        var diagnosticBytes = 0
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
                  !chunk.isEmpty {
                lock.lock()
                diagnosticBytes += chunk.count
                if diagnostic.count < 32 * 1024 {
                    diagnostic.append(chunk.prefix(32 * 1024 - diagnostic.count))
                }
                lock.unlock()
            }
        }
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + processTimeout) {
            if process.isRunning {
                lock.lock()
                timedOut = true
                lock.unlock()
                process.terminate()
                DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 2) {
                    if process.isRunning { _ = Darwin.kill(process.processIdentifier, SIGKILL) }
                }
            }
        }
        process.waitUntilExit()
        var drainComplete = group.wait(timeout: .now() + 2) == .success
        if !drainComplete {
            try? stdout.fileHandleForReading.close()
            try? stderr.fileHandleForReading.close()
            drainComplete = group.wait(timeout: .now() + 2) == .success
        }
        lock.lock()
        let result = output
        let failureDiagnostic = diagnostic
        let failureDiagnosticBytes = diagnosticBytes
        let wasOversized = oversized
        let didTimeOut = timedOut
        lock.unlock()
        let evidence = Self.nativeEvidence(
            failureDiagnostic,
            observedBytes: failureDiagnosticBytes,
            drainComplete: drainComplete
        )
        guard !wasOversized else {
            throw NodeConnectAgentBridgeError.oversizedResponse
        }
        guard !didTimeOut else { throw NodeConnectAgentBridgeError.nativeProcessFailure(evidence) }
        guard process.terminationStatus == 0 else {
            throw Self.classifiedFailure(failureDiagnostic, evidence: evidence)
        }
        return result
    }

    private static func nativeEvidence(
        _ diagnostic: Data,
        observedBytes: Int,
        drainComplete: Bool
    ) -> NativeProcessEvidence {
        let message = String(decoding: diagnostic, as: UTF8.self).lowercased()
        let classification: String
        if message.contains("permission denied") || message.contains("access is denied") {
            classification = "access_denied"
        } else if message.contains("not found") || message.contains("missing") {
            classification = "missing_dependency"
        } else if message.contains("panic") || message.contains("fatal") {
            classification = "native_crash"
        } else if diagnostic.isEmpty {
            classification = "none"
        } else {
            classification = "unclassified"
        }
        return NativeProcessEvidence(
            classification: classification,
            stderrBytes: observedBytes,
            inspectedBytes: diagnostic.count,
            drainComplete: drainComplete
        )
    }

    private static func classifiedFailure(
        _ diagnostic: Data,
        evidence: NativeProcessEvidence
    ) -> NodeConnectAgentBridgeError {
        let message = String(decoding: diagnostic, as: UTF8.self).lowercased()
        if message.contains("expired") { return .expired }
        if message.contains("401") || message.contains("unauthorized")
            || message.contains("invalid_agent_public_key")
        {
            return .identityRejected
        }
        if message.contains("404") || message.contains("409")
            || message.contains("already used") || message.contains("invalid invitation")
        {
            return .invitationRejected
        }
        return .nativeProcessFailure(evidence)
    }
}
