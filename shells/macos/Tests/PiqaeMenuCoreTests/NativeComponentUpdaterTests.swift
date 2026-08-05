import Foundation
import XCTest
@testable import PiqaeMenuCore

final class NativeComponentUpdaterTests: XCTestCase {
    func testRunnerPassesOnlyBoundedNonSecretComponentMetadata() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let script = directory.appendingPathComponent("update.sh")
        let output = directory.appendingPathComponent("arguments")
        try """
        #!/bin/sh
        printf '%s\n' "$@" > "$1/arguments"
        """.write(to: script, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)

        try NativeComponentUpdater(
            scriptURL: script,
            componentDirectoryURL: directory,
            version: "1.2.3",
            channel: "signed-release"
        ).run()

        XCTAssertEqual(
            try String(contentsOf: output, encoding: .utf8).split(separator: "\n").map(String.init),
            [directory.path, "1.2.3", "signed-release"]
        )
    }

    func testRunnerFailsClosedWhenActivationFails() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let script = directory.appendingPathComponent("update.sh")
        try "#!/bin/sh\nexit 1\n".write(to: script, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: script.path)
        XCTAssertThrowsError(try NativeComponentUpdater(
            scriptURL: script,
            componentDirectoryURL: directory,
            version: "1.2.3",
            channel: "signed-release"
        ).run())
    }
}
