#!/usr/bin/env bash
set -euo pipefail

shell_root=$(cd "$(dirname "$0")" && pwd)
configuration=${CONFIGURATION:-release}
bundle=${SPOOL_APP_BUNDLE:-"$shell_root/build/Piqae.app"}
version=${SPOOL_VERSION:-0.1.0}
build_number=${SPOOL_BUILD_NUMBER:-1}
feed_url=${SPOOL_SPARKLE_FEED_URL:-}
public_key=${SPOOL_SPARKLE_PUBLIC_ED_KEY:-}
signing_identity=${SPOOL_CODE_SIGN_IDENTITY:-}
swift_archs=${SPOOL_SWIFT_ARCHS:-}
swift_arch_args=()

if [[ -n "$swift_archs" ]]; then
  IFS=',' read -r -a requested_archs <<< "$swift_archs"
  for architecture in "${requested_archs[@]}"; do
    case "$architecture" in
      arm64|x86_64) swift_arch_args+=(--arch "$architecture") ;;
      *) echo "SPOOL_SWIFT_ARCHS supports only arm64,x86_64" >&2; exit 2 ;;
    esac
  done
fi

swift_build() {
  if [[ -n "$swift_archs" ]]; then
    swift build \
      --package-path "$shell_root" \
      --configuration "$configuration" \
      "${swift_arch_args[@]}" \
      "$@"
  else
    swift build \
      --package-path "$shell_root" \
      --configuration "$configuration" \
      "$@"
  fi
}

case "$bundle" in
  ""|"/"|"$HOME"|"$shell_root") echo "refusing unsafe app bundle path: $bundle" >&2; exit 2 ;;
  *.app) ;;
  *) echo "SPOOL_APP_BUNDLE must end in .app" >&2; exit 2 ;;
esac
if [[ ! "$version" =~ ^[0-9A-Za-z][0-9A-Za-z.-]*$ ]]; then
  echo "SPOOL_VERSION is not a safe bundle version" >&2
  exit 2
fi
if [[ ! "$build_number" =~ ^[1-9][0-9]*$ ]]; then
  echo "SPOOL_BUILD_NUMBER must be a positive integer" >&2
  exit 2
fi
if [[ -n "$feed_url" || -n "$public_key" || -n "$signing_identity" ]]; then
  if [[ -z "$feed_url" || -z "$public_key" || -z "$signing_identity" ]]; then
    echo "Sparkle feed URL, public key, and signing identity are an all-or-none release gate" >&2
    exit 2
  fi
  if [[ ! "$feed_url" =~ ^https://[A-Za-z0-9][A-Za-z0-9.-]*(:[0-9]+)?(/[A-Za-z0-9._~:/?%+\&=-]*)?$ ]]; then
    echo "SPOOL_SPARKLE_FEED_URL must be a safe HTTPS URL" >&2
    exit 2
  fi
  if [[ ! "$public_key" =~ ^[A-Za-z0-9+/=]+$ ]]; then
    echo "SPOOL_SPARKLE_PUBLIC_ED_KEY is not valid base64 text" >&2
    exit 2
  fi
  decoded_key_length=$(printf '%s' "$public_key" | base64 -D | wc -c | tr -d ' ')
  if [[ "$decoded_key_length" -ne 32 ]]; then
    echo "SPOOL_SPARKLE_PUBLIC_ED_KEY must decode to a 32-byte Ed25519 public key" >&2
    exit 2
  fi
fi

swift_build --product SpoolMenu
swift_build --product SpoolPrintCoreReplay
binary_directory=$(swift_build --show-bin-path)

if [[ -e "$bundle" ]]; then
  rm -rf -- "$bundle"
fi
mkdir -p \
  "$bundle/Contents/MacOS" \
  "$bundle/Contents/Resources" \
  "$bundle/Contents/Frameworks"
install -m 0755 "$binary_directory/SpoolMenu" "$bundle/Contents/MacOS/SpoolMenu"
install -m 0755 \
  "$binary_directory/SpoolPrintCoreReplay" \
  "$bundle/Contents/MacOS/SpoolPrintCoreReplay"
menu_rpaths=$(otool -l "$bundle/Contents/MacOS/SpoolMenu")
if ! grep -F "@executable_path/../Frameworks" <<<"$menu_rpaths" >/dev/null; then
  install_name_tool \
    -add_rpath "@executable_path/../Frameworks" \
    "$bundle/Contents/MacOS/SpoolMenu"
fi
if [[ ! -d "$binary_directory/Sparkle.framework" ]]; then
  echo "Sparkle.framework was not produced by SwiftPM" >&2
  exit 1
fi
ditto \
  "$binary_directory/Sparkle.framework" \
  "$bundle/Contents/Frameworks/Sparkle.framework"
install -m 0644 "$shell_root/Resources/Info.plist" "$bundle/Contents/Info.plist"

plist_buddy=/usr/libexec/PlistBuddy
"$plist_buddy" -c "Set :CFBundleShortVersionString $version" "$bundle/Contents/Info.plist"
"$plist_buddy" -c "Set :CFBundleVersion $build_number" "$bundle/Contents/Info.plist"

if [[ -n "$feed_url" ]]; then
  "$plist_buddy" -c "Set :SpoolBuildChannel signed-release" "$bundle/Contents/Info.plist"
  "$plist_buddy" -c "Set :SpoolUpdatesEnabled true" "$bundle/Contents/Info.plist"
  "$plist_buddy" -c "Add :SUFeedURL string $feed_url" "$bundle/Contents/Info.plist"
  "$plist_buddy" -c "Add :SUPublicEDKey string $public_key" "$bundle/Contents/Info.plist"
fi
plutil -lint "$bundle/Contents/Info.plist"

if [[ -n "$signing_identity" ]]; then
  SPOOL_CODE_SIGN_IDENTITY="$signing_identity" \
    "$shell_root/../../packaging/macos/sign-app.sh" "$bundle"
else
  echo "Built unsigned preview; Sparkle updates are disabled." >&2
fi

echo "$bundle"
