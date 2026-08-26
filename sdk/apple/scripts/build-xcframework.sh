#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_directory/../../.." && pwd)
artifact_directory="$repository_root/sdk/apple/.artifacts"
output="$artifact_directory/PiqaeNode.xcframework"
archive="$artifact_directory/PiqaeNode.xcframework.zip"
manifest="$artifact_directory/PiqaeNode.artifact.json"

if [ -e "$output" ] || [ -e "$archive" ] || [ -e "$manifest" ]; then
  if [ "${1:-}" != "--replace" ]; then
    echo "Apple artifacts already exist; pass --replace to replace generated outputs." >&2
    exit 2
  fi
  rm -rf -- "$output"
  rm -f -- "$archive" "$manifest"
fi

for command in cargo rustup lipo xcodebuild ditto swift shasum; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "required command is unavailable: $command" >&2
    exit 2
  fi
done

for target in \
  aarch64-apple-darwin \
  x86_64-apple-darwin \
  aarch64-apple-ios \
  aarch64-apple-ios-sim \
  x86_64-apple-ios
do
  rustup target add "$target"
done

temporary_directory=$(mktemp -d "${TMPDIR:-/tmp}/piqae-node-xcframework.XXXXXX")
cleanup() {
  rm -rf -- "$temporary_directory"
}
trap cleanup EXIT HUP INT TERM

headers="$temporary_directory/Headers"
mkdir -p "$headers" "$artifact_directory"
cp "$repository_root/sdk/native/include/piqae_node.h" "$headers/piqae_node.h"
cp "$repository_root/sdk/apple/support/PiqaeNodeNative.modulemap" "$headers/module.modulemap"

build_target() {
  target=$1
  shift
  env "$@" cargo build \
    --manifest-path "$repository_root/Cargo.toml" \
    --locked \
    --release \
    --target "$target" \
    -p piqae-node-ffi
}

build_target aarch64-apple-darwin MACOSX_DEPLOYMENT_TARGET=13.0
build_target x86_64-apple-darwin MACOSX_DEPLOYMENT_TARGET=13.0
build_target aarch64-apple-ios IPHONEOS_DEPLOYMENT_TARGET=16.0
build_target aarch64-apple-ios-sim IPHONEOS_DEPLOYMENT_TARGET=16.0
build_target x86_64-apple-ios IPHONEOS_DEPLOYMENT_TARGET=16.0

macos_library="$temporary_directory/libPiqaeNode-macos.a"
simulator_library="$temporary_directory/libPiqaeNode-simulator.a"
device_library="$temporary_directory/libPiqaeNode-device.a"
lipo -create \
  "$repository_root/target/aarch64-apple-darwin/release/libpiqae_node_ffi.a" \
  "$repository_root/target/x86_64-apple-darwin/release/libpiqae_node_ffi.a" \
  -output "$macos_library"
lipo -create \
  "$repository_root/target/aarch64-apple-ios-sim/release/libpiqae_node_ffi.a" \
  "$repository_root/target/x86_64-apple-ios/release/libpiqae_node_ffi.a" \
  -output "$simulator_library"
cp "$repository_root/target/aarch64-apple-ios/release/libpiqae_node_ffi.a" "$device_library"

xcodebuild -create-xcframework \
  -library "$macos_library" -headers "$headers" \
  -library "$device_library" -headers "$headers" \
  -library "$simulator_library" -headers "$headers" \
  -output "$output"

ditto -c -k --sequesterRsrc --keepParent "$output" "$archive"
swiftpm_checksum=$(swift package compute-checksum "$archive")
sha256=$(shasum -a 256 "$archive" | awk '{print $1}')
revision=$(git -C "$repository_root" rev-parse HEAD)
cat > "$manifest" <<EOF
{
  "schema": 1,
  "git_revision": "$revision",
  "artifact": "PiqaeNode.xcframework.zip",
  "swiftpm_checksum": "$swiftpm_checksum",
  "sha256": "$sha256",
  "slices": [
    "macos-arm64_x86_64",
    "ios-arm64",
    "ios-arm64_x86_64-simulator"
  ]
}
EOF

echo "Built $output"
echo "Archive SHA-256: $sha256"
echo "SwiftPM checksum: $swiftpm_checksum"
echo "Manifest: $manifest"
