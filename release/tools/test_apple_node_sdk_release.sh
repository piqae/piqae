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
cat > "$consumer/Sources/PiqaeReleaseConsumer/main.swift" <<'EOF'
import Darwin
import PiqaeNodeKit

guard PiqaeNativeRuntime.linkedLibraryAvailable else {
    fputs("packaged native ABI is unavailable\n", stderr)
    exit(1)
}
print("PiqaeNodeKit release consumer linked the packaged native ABI")
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
