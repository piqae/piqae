#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
artifact_directory="$repository_root/sdk/apple/.artifacts"
xcframework="$artifact_directory/PiqaeNode.xcframework"
archive="$artifact_directory/PiqaeNode.xcframework.zip"
mode=${1:-build-clean}
cleanup_artifact=false
temporary_directory=$(mktemp -d "${TMPDIR:-/tmp}/piqae-apple-sdk-linked.XXXXXX")
artifact_backup="$temporary_directory/original-artifacts"
verify_reproducible=false

cleanup() {
  if [[ "$cleanup_artifact" == true ]]; then
    rm -rf -- "$xcframework"
    rm -f -- "$archive" "$artifact_directory/PiqaeNode.artifact.json"
    if [[ -d "$artifact_backup" ]]; then
      mkdir -p "$artifact_directory"
      for original in "$artifact_backup"/*; do
        [[ -e "$original" ]] || continue
        mv -- "$original" "$artifact_directory/"
      done
    fi
    rmdir "$artifact_directory" 2>/dev/null || true
  fi
  rm -rf -- "$temporary_directory"
}
trap cleanup EXIT

case "$mode" in
  build-clean)
    cleanup_artifact=true
    mkdir -p "$artifact_backup"
    for generated in \
      "$xcframework" \
      "$archive" \
      "$artifact_directory/PiqaeNode.artifact.json"
    do
      if [[ -e "$generated" ]]; then
        mv -- "$generated" "$artifact_backup/"
      fi
    done
    "$repository_root/sdk/apple/scripts/build-xcframework.sh"
    verify_reproducible=true
    ;;
  use-existing)
    if [[ ! -d "$xcframework" ]]; then
      echo "Apple XCFramework is required before linked validation." >&2
      exit 2
    fi
    ;;
  verify-existing)
    if [[ ! -d "$xcframework" || ! -f "$archive" ]]; then
      echo "Apple XCFramework and archive are required before reproducibility validation." >&2
      exit 2
    fi
    verify_reproducible=true
    ;;
  *)
    echo "usage: $0 [build-clean|use-existing|verify-existing]" >&2
    exit 2
    ;;
esac

if [[ "$verify_reproducible" == true ]]; then
  first_hash=$(shasum -a 256 "$archive" | awk '{print $1}')
  "$repository_root/sdk/apple/scripts/build-xcframework.sh" --replace
  second_hash=$(shasum -a 256 "$archive" | awk '{print $1}')
  if [[ "$first_hash" != "$second_hash" ]]; then
    echo "Apple XCFramework archive is not reproducible." >&2
    exit 1
  fi
fi

export PIQAE_REQUIRE_LINKED_RUNTIME_TESTS=1

swift test \
  --package-path "$repository_root/sdk/apple" \
  --scratch-path "$temporary_directory/nodekit" \
  -Xswiftc -strict-concurrency=complete \
  -Xswiftc -warnings-as-errors

swift build \
  --package-path "$repository_root/sdk/apple/Examples/ConsumerFixture" \
  --scratch-path "$temporary_directory/consumer-macos" \
  -Xswiftc -strict-concurrency=complete \
  -Xswiftc -warnings-as-errors

(
  cd "$repository_root/sdk/apple/Examples/ConsumerFixture"
  xcodebuild \
    -quiet \
    -scheme PiqaeNodeKitConsumerFixture \
    -destination 'generic/platform=iOS Simulator' \
    -sdk iphonesimulator \
    -derivedDataPath "$temporary_directory/consumer-ios" \
    CODE_SIGNING_ALLOWED=NO \
    SWIFT_TREAT_WARNINGS_AS_ERRORS=YES \
    SWIFT_SUPPRESS_WARNINGS=NO \
    SWIFT_STRICT_CONCURRENCY=complete \
    build
)
