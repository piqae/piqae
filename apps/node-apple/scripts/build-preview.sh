#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
APP_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
REPO_ROOT=$(CDPATH= cd -- "$APP_DIR/../.." && pwd)
ARTIFACT_DIR="$APP_DIR/.artifacts"
NATIVE_FRAMEWORK="$REPO_ROOT/sdk/apple/.artifacts/PiqaeNode.xcframework"

if [ ! -d "$NATIVE_FRAMEWORK" ]; then
  "$REPO_ROOT/sdk/apple/scripts/build-xcframework.sh"
fi
"$SCRIPT_DIR/generate-project.sh"
mkdir -p "$ARTIFACT_DIR/device"

PIQAE_REQUIRE_LINKED_RUNTIME_TESTS=1 xcodebuild \
  -project "$APP_DIR/PiqaeNodeApple.xcodeproj" \
  -scheme PiqaeNode \
  -configuration Release \
  -destination 'generic/platform=iOS' \
  CODE_SIGNING_ALLOWED=NO \
  CONFIGURATION_BUILD_DIR="$ARTIFACT_DIR/device" \
  build

APP_PATH="$ARTIFACT_DIR/device/PiqaeNode.app"
if [ ! -d "$APP_PATH" ]; then
  echo "unsigned app was not produced at the expected path" >&2
  exit 1
fi
ditto -c -k --keepParent "$APP_PATH" "$ARTIFACT_DIR/PiqaeNode-iOS-unsigned-preview.zip"
shasum -a 256 "$ARTIFACT_DIR/PiqaeNode-iOS-unsigned-preview.zip" \
  > "$ARTIFACT_DIR/PiqaeNode-iOS-unsigned-preview.zip.sha256"
