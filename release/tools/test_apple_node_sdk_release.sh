#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
version=${1:?usage: test_apple_node_sdk_release.sh VERSION [STAGE]}
stage=${2:-"$repository_root/artifacts/apple-node-sdk"}
temporary_directory=""

cleanup() {
  if [[ -n "$temporary_directory" ]]; then
    rm -rf -- "$temporary_directory"
  fi
}
trap cleanup EXIT

python3 "$repository_root/release/tools/stage_apple_node_sdk.py" validate \
  --repository-root "$repository_root" \
  --version "$version" \
  --output "$stage"

temporary_directory=$(mktemp -d "${TMPDIR:-/tmp}/piqae-nodekit-release-consumer.XXXXXX")
package_archive="$stage/PiqaeNodeKit-$version.zip"
native_archive="$stage/PiqaeNode.xcframework-$version.zip"
ditto -x -k "$package_archive" "$temporary_directory/package"
package_root="$temporary_directory/package/PiqaeNodeKit-$version"
mkdir -p "$package_root/.artifacts"
ditto -x -k "$native_archive" "$package_root/.artifacts"

consumer="$temporary_directory/consumer"
mkdir -p "$consumer/Sources/PiqaeReleaseConsumer"
cat > "$consumer/Package.swift" <<EOF
// swift-tools-version: 5.10
import PackageDescription

let package = Package(
    name: "PiqaeReleaseConsumer",
    platforms: [.macOS(.v13)],
    dependencies: [.package(name: "PiqaeNodeKit", path: "$package_root")],
    targets: [
        .executableTarget(
            name: "PiqaeReleaseConsumer",
            dependencies: [.product(name: "PiqaeNodeKit", package: "PiqaeNodeKit")]
        )
    ]
)
EOF
# Xcode treats main.swift as an implicit top-level entry point, which cannot
# also contain an @main declaration. A non-special filename keeps this exact
# consumer canonical under both `swift run` and xcodebuild.
cat > "$consumer/Sources/PiqaeReleaseConsumer/ReleaseConsumer.swift" <<'EOF'
import Darwin
import Foundation
import PiqaeNodeKit

private final class FixedHostKeyStore: @unchecked Sendable, PiqaeHostKeyStore {
    func loadOrCreateKey() throws -> Data { Data(repeating: 0x5a, count: 32) }
}

@main
struct ReleaseConsumer {
    static func main() async throws {
        guard PiqaeNativeRuntime.linkedLibraryAvailable,
              PiqaeNativeRuntime.nativeABIVersion == 1,
              PiqaeNativeRuntime.nativeContractVersion == 2
        else {
            fputs("packaged native ABI 1 / contract 2 is unavailable\n", stderr)
            exit(1)
        }
        let applicationID = "com.piqae.release-smoke.\(UUID().uuidString.lowercased())"
        let dataDirectory = "runtime"
        let stateURL = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        )[0]
            .appendingPathComponent("Piqae/embedded", isDirectory: true)
            .appendingPathComponent(applicationID, isDirectory: true)
        let runtime = PiqaeNativeRuntime(
            configuration: .init(
                applicationID: applicationID,
                dataDirectory: dataDirectory,
                availability: .foregroundOnly,
                localOnly: true
            ),
            keyStore: FixedHostKeyStore()
        )
        do {
            try await runtime.start()
            let capabilities = try await runtime.printPacketCapabilities()
            guard capabilities.contract == "printpacket/v1",
                  capabilities.directOfflineRendering
            else { throw PiqaeNativeRuntimeError.invalidResponse }
            try await runtime.stop()
        } catch {
            try? await runtime.stop()
            throw error
        }
        try? FileManager.default.removeItem(at: stateURL)
        print("PiqaeNodeKit release consumer verified ABI 1, contract 2, and PrintPacket capabilities")
    }
}
EOF

swift package --package-path "$package_root" dump-package >/dev/null
swift run \
  --package-path "$consumer" \
  --scratch-path "$temporary_directory/consumer-build" \
  -Xswiftc -strict-concurrency=complete \
  -Xswiftc -warnings-as-errors \
  PiqaeReleaseConsumer

(
  cd "$consumer"
  xcodebuild \
    -quiet \
    -scheme PiqaeReleaseConsumer \
    -destination 'generic/platform=macOS' \
    -derivedDataPath "$temporary_directory/xcode-consumer" \
    CODE_SIGNING_ALLOWED=NO \
    SWIFT_TREAT_WARNINGS_AS_ERRORS=YES \
    SWIFT_SUPPRESS_WARNINGS=NO \
    SWIFT_STRICT_CONCURRENCY=complete \
    build
)
