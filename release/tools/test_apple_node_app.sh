#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
app_root="$repository_root/apps/node-apple"
fixture_root="$repository_root/.piqae-test-fixtures"
derived_data="$fixture_root/apple-node-app-derived-data"
temporary_app=$(mktemp -d "$repository_root/apps/.piqae-node-apple-project.XXXXXX")

cleanup() {
  rm -rf -- "$temporary_app"
}
trap cleanup EXIT HUP INT TERM

for command in xcodegen xcodebuild xcrun python3 diff; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "required Apple app validation command is unavailable: $command" >&2
    exit 2
  fi
done
if [[ ! -d "$repository_root/sdk/apple/.artifacts/PiqaeNode.xcframework" ]]; then
  echo "linked Apple app validation requires the already-built PiqaeNode XCFramework" >&2
  exit 2
fi

cp "$app_root/project.yml" "$temporary_app/project.yml"
xcodegen generate --spec "$temporary_app/project.yml" --project "$temporary_app"
diff -ru \
  -x xcuserdata \
  "$app_root/PiqaeNodeApple.xcodeproj" \
  "$temporary_app/PiqaeNodeApple.xcodeproj"

destination_id=$(xcrun simctl list devices available -j | python3 -c '
import json, sys
devices = json.load(sys.stdin).get("devices", {})
available = [
    device for runtime in sorted(devices, reverse=True) for device in devices[runtime]
    if device.get("isAvailable") and device.get("udid")
]
preferred = next((device for device in available if "iPad" in device.get("name", "")), None)
selected = preferred or (available[0] if available else None)
if selected is None:
    raise SystemExit("no available iOS Simulator device")
print(selected["udid"])
')

mkdir -p "$fixture_root"
PIQAE_REQUIRE_LINKED_RUNTIME_TESTS=1 xcodebuild \
  -quiet \
  -project "$app_root/PiqaeNodeApple.xcodeproj" \
  -scheme PiqaeNode \
  -destination "platform=iOS Simulator,id=$destination_id" \
  -derivedDataPath "$derived_data" \
  CODE_SIGNING_ALLOWED=NO \
  SWIFT_TREAT_WARNINGS_AS_ERRORS=YES \
  SWIFT_SUPPRESS_WARNINGS=NO \
  SWIFT_STRICT_CONCURRENCY=complete \
  test
