import Foundation

public enum NativeComponentUpdaterError: Error, LocalizedError, Sendable {
    case invalidBundle
    case failed

    public var errorDescription: String? {
        switch self {
        case .invalidBundle: "The Piqae update does not contain a valid native component set."
        case .failed: "Piqae kept the previous node components because the native update could not be activated safely."
        }
    }
}

public struct NativeComponentUpdater: Sendable {
    public let scriptURL: URL
    public let componentDirectoryURL: URL
    public let version: String
    public let channel: String

    public init?(bundle: Bundle = .main) {
        guard bundle.object(forInfoDictionaryKey: "PiqaeNativeComponentsBundled") as? Bool == true,
            let resourceURL = bundle.resourceURL,
            let version = bundle.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String,
            let channel = bundle.object(forInfoDictionaryKey: "PiqaeBuildChannel") as? String,
            channel == "signed-release"
        else { return nil }
        let directory = resourceURL.appendingPathComponent("Node", isDirectory: true)
        scriptURL = directory.appendingPathComponent("update-native-components.sh")
        componentDirectoryURL = directory
        self.version = version
        self.channel = channel
    }

    public init(scriptURL: URL, componentDirectoryURL: URL, version: String, channel: String) {
        self.scriptURL = scriptURL
        self.componentDirectoryURL = componentDirectoryURL
        self.version = version
        self.channel = channel
    }

    public func run() throws {
        guard scriptURL.isFileURL, componentDirectoryURL.isFileURL,
            FileManager.default.isExecutableFile(atPath: scriptURL.path),
            !version.isEmpty, !channel.isEmpty
        else { throw NativeComponentUpdaterError.invalidBundle }
        let process = Process()
        process.executableURL = scriptURL
        process.arguments = [componentDirectoryURL.path, version, channel]
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { throw NativeComponentUpdaterError.failed }
    }
}
