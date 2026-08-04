#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 4 ]]; then
  echo "usage: $0 /path/Piqae.app /path/piqae-agent /path/piqae-executor-cups /output/directory" >&2
  exit 2
fi

app=$1
agent=$2
executor=$3
output_root=$4
script_root=$(cd "$(dirname "$0")" && pwd)
repository_root=$(cd "$script_root/../.." && pwd)

for required in "$app" "$agent" "$executor"; do
  if [[ ! -e "$required" ]]; then
    echo "missing release input: $required" >&2
    exit 1
  fi
done
if [[ "$app" != *.app ]]; then
  echo "first input must be a macOS app bundle" >&2
  exit 2
fi

version=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$app/Contents/Info.plist")
build=$(/usr/libexec/PlistBuddy -c "Print :CFBundleVersion" "$app/Contents/Info.plist")
if [[ ! "$version" =~ ^[0-9A-Za-z][0-9A-Za-z.-]*$ || ! "$build" =~ ^[1-9][0-9]*$ ]]; then
  echo "app bundle contains unsafe version metadata" >&2
  exit 1
fi

mkdir -p "$output_root"
channel=$(/usr/libexec/PlistBuddy -c "Print :PiqaeBuildChannel" "$app/Contents/Info.plist")
suffix=""
if [[ "$channel" == "unsigned-preview" ]]; then
  suffix="-unsigned-preview"
fi
package_name="Piqae-${version}-${build}-macos-user${suffix}"
package_dir="$output_root/$package_name"
package_zip="$output_root/$package_name.zip"
update_zip="$output_root/Piqae-${version}-${build}-macos-update${suffix}.zip"
installer_pkg="$output_root/Piqae-${version}-${build}-macos-installer${suffix}.pkg"
dmg="$output_root/Piqae-${version}-${build}-macos-installer${suffix}.dmg"
installer_stage="$output_root/.Piqae-${version}-${build}-installer"
pkg_scripts="$output_root/.Piqae-${version}-${build}-pkg-scripts"

if [[ -e "$package_dir" || -e "$package_zip" || -e "$update_zip" ||
  -e "$installer_pkg" || -e "$dmg" || -e "$installer_stage" ||
  -e "$pkg_scripts" ]]; then
  echo "refusing to overwrite an existing release artifact" >&2
  exit 1
fi
cleanup() {
  rm -rf -- "$package_dir" "$installer_stage" "$pkg_scripts"
}
trap cleanup EXIT

mkdir -p "$package_dir/payload"
ditto "$app" "$package_dir/payload/Piqae.app"
install -m 0755 "$agent" "$package_dir/payload/piqae-agent"
install -m 0755 "$executor" "$package_dir/payload/piqae-executor-cups"
install -m 0755 "$script_root/install-user.sh" "$package_dir/install-user.sh"
install -m 0755 "$script_root/uninstall-user.sh" "$package_dir/uninstall-user.sh"
install -m 0644 \
  "$script_root/com.piqae.agent.plist.in" \
  "$package_dir/payload/com.piqae.agent.plist.in"
install -m 0644 \
  "$script_root/com.piqae.menu.plist.in" \
  "$package_dir/payload/com.piqae.menu.plist.in"
install -m 0644 "$repository_root/LICENSE" "$package_dir/LICENSE"
ditto "$repository_root/LICENSES" "$package_dir/LICENSES"

(
  cd "$package_dir"
  shasum -a 256 \
    "payload/Piqae.app/Contents/MacOS/PiqaeMenu" \
    "payload/Piqae.app/Contents/MacOS/PiqaePrintCoreReplay" \
    "payload/piqae-agent" \
    "payload/piqae-executor-cups" \
    > SHA256SUMS
)
ditto -c -k --sequesterRsrc --keepParent "$package_dir" "$package_zip"
ditto -c -k --sequesterRsrc --keepParent "$app" "$update_zip"

mkdir -p "$pkg_scripts/PiqaePackage" "$installer_stage"
ditto "$package_dir" "$pkg_scripts/PiqaePackage"
install -m 0755 "$script_root/pkg-postinstall" "$pkg_scripts/postinstall"

installer_identity=${PIQAE_INSTALLER_SIGN_IDENTITY:-}
pkgbuild_args=(
  --nopayload
  --scripts "$pkg_scripts"
  --identifier com.piqae.node.installer
  --version "$version"
  --install-location /
)
if [[ -n "$installer_identity" ]]; then
  if [[ "$installer_identity" != "Developer ID Installer:"* ]]; then
    echo "PIQAE_INSTALLER_SIGN_IDENTITY must name a Developer ID Installer certificate" >&2
    exit 2
  fi
  pkgbuild_args+=(--sign "$installer_identity" --timestamp)
fi
pkgbuild "${pkgbuild_args[@]}" "$installer_pkg"
if [[ -n "$installer_identity" ]]; then
  signature_report=$(pkgutil --check-signature "$installer_pkg")
  printf '%s\n' "$signature_report"
  grep -F "$installer_identity" <<<"$signature_report" >/dev/null
else
  pkg_verify="$output_root/.Piqae-${version}-${build}-pkg-verify"
  pkgutil --expand-full "$installer_pkg" "$pkg_verify"
  test -x "$pkg_verify/Scripts/postinstall"
  rm -rf -- "$pkg_verify"
fi

cp "$installer_pkg" "$installer_stage/Install Piqae.pkg"
install -m 0644 "$script_root/INSTALL.txt" "$installer_stage/Read Me.txt"

hdiutil create \
  -quiet \
  -fs HFS+ \
  -format UDZO \
  -imagekey zlib-level=9 \
  -srcfolder "$installer_stage" \
  -volname "Install Piqae" \
  "$dmg"
identity=${PIQAE_CODE_SIGN_IDENTITY:-}
if [[ -n "$identity" ]]; then
  codesign \
    --force \
    --timestamp \
    --sign "$identity" \
    "$dmg"
  codesign --verify --strict --verbose=2 "$dmg"
fi

shasum -a 256 "$package_zip" "$update_zip" "$installer_pkg" "$dmg" > "$output_root/SHA256SUMS"
printf '%s\n%s\n%s\n%s\n' "$package_zip" "$update_zip" "$installer_pkg" "$dmg"
