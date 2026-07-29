#!/usr/bin/env bash
set -euo pipefail

shell_root=$(cd "$(dirname "$0")" && pwd)
configuration=${CONFIGURATION:-release}
bundle="$shell_root/build/Spool.app"

swift build \
  --package-path "$shell_root" \
  --configuration "$configuration" \
  --product SpoolMenu

binary_directory=$(swift build \
  --package-path "$shell_root" \
  --configuration "$configuration" \
  --show-bin-path)

rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
install -m 0755 "$binary_directory/SpoolMenu" "$bundle/Contents/MacOS/SpoolMenu"
install -m 0644 "$shell_root/Resources/Info.plist" "$bundle/Contents/Info.plist"
plutil -lint "$bundle/Contents/Info.plist"

echo "$bundle"
