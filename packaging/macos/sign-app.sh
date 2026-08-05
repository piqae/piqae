#!/usr/bin/env bash
set -euo pipefail

app=${1:-}
identity=${PIQAE_CODE_SIGN_IDENTITY:-}
script_root=$(cd "$(dirname "$0")" && pwd)
entitlements="$script_root/../../shells/macos/Resources/Piqae.entitlements"

if [[ -z "$app" || ! -d "$app" || "$app" != *.app ]]; then
  echo "usage: PIQAE_CODE_SIGN_IDENTITY='Developer ID Application: …' $0 /path/Piqae.app" >&2
  exit 2
fi
if [[ "$identity" != "Developer ID Application:"* ]]; then
  echo "PIQAE_CODE_SIGN_IDENTITY must name a Developer ID Application certificate" >&2
  exit 2
fi
if ! security find-identity -v -p codesigning | grep -F -- "$identity" >/dev/null; then
  echo "Developer ID Application identity is not available in the active keychain" >&2
  exit 1
fi

framework="$app/Contents/Frameworks/Sparkle.framework"
for component in \
  "$app/Contents/Resources/Node/piqae-agent" \
  "$app/Contents/Resources/Node/piqae-executor-cups"
do
  if [[ -e "$component" ]]; then
    codesign --force --timestamp --options runtime --sign "$identity" "$component"
  fi
done
for nested in \
  "$framework/Versions/B/Autoupdate" \
  "$framework/Versions/B/XPCServices/Downloader.xpc" \
  "$framework/Versions/B/XPCServices/Installer.xpc" \
  "$framework/Versions/B/Updater.app"
do
  if [[ ! -e "$nested" ]]; then
    echo "missing required Sparkle nested code: $nested" >&2
    exit 1
  fi
  codesign \
    --force \
    --timestamp \
    --options runtime \
    --preserve-metadata=entitlements,requirements \
    --sign "$identity" \
    "$nested"
done

codesign \
  --force \
  --timestamp \
  --options runtime \
  --preserve-metadata=entitlements,requirements \
  --sign "$identity" \
  "$framework"
codesign \
  --force \
  --timestamp \
  --options runtime \
  --sign "$identity" \
  "$app/Contents/MacOS/PiqaePrintCoreReplay"
codesign \
  --force \
  --timestamp \
  --options runtime \
  --entitlements "$entitlements" \
  --sign "$identity" \
  "$app"

codesign --verify --deep --strict --verbose=2 "$app"
codesign -dv --verbose=4 "$app" 2>&1 | grep -F "Authority=Developer ID Application:"
