#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 4 ]]; then
  echo "usage: $0 /path/Spool.app /path/spool-agent /path/spool-executor-cups /output/directory" >&2
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
  echo "first input must be a Spool.app bundle" >&2
  exit 2
fi

version=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$app/Contents/Info.plist")
build=$(/usr/libexec/PlistBuddy -c "Print :CFBundleVersion" "$app/Contents/Info.plist")
if [[ ! "$version" =~ ^[0-9A-Za-z][0-9A-Za-z.-]*$ || ! "$build" =~ ^[1-9][0-9]*$ ]]; then
  echo "app bundle contains unsafe version metadata" >&2
  exit 1
fi

mkdir -p "$output_root"
channel=$(/usr/libexec/PlistBuddy -c "Print :SpoolBuildChannel" "$app/Contents/Info.plist")
suffix=""
if [[ "$channel" == "unsigned-preview" ]]; then
  suffix="-unsigned-preview"
fi
package_name="Spool-${version}-${build}-macos-user${suffix}"
package_dir="$output_root/$package_name"
package_zip="$output_root/$package_name.zip"
update_zip="$output_root/Spool-${version}-${build}-macos-update${suffix}.zip"

if [[ -e "$package_dir" || -e "$package_zip" || -e "$update_zip" ]]; then
  echo "refusing to overwrite an existing release artifact" >&2
  exit 1
fi

mkdir -p "$package_dir/payload"
ditto "$app" "$package_dir/payload/Spool.app"
install -m 0755 "$agent" "$package_dir/payload/spool-agent"
install -m 0755 "$executor" "$package_dir/payload/spool-executor-cups"
install -m 0755 "$script_root/install-user.sh" "$package_dir/install-user.sh"
install -m 0755 "$script_root/uninstall-user.sh" "$package_dir/uninstall-user.sh"
install -m 0644 \
  "$script_root/com.c4coffee.spool.agent.plist.in" \
  "$package_dir/payload/com.c4coffee.spool.agent.plist.in"
install -m 0644 \
  "$script_root/com.c4coffee.spool.menu.plist.in" \
  "$package_dir/payload/com.c4coffee.spool.menu.plist.in"
install -m 0644 "$repository_root/LICENSE" "$package_dir/LICENSE"
ditto "$repository_root/LICENSES" "$package_dir/LICENSES"

(
  cd "$package_dir"
  shasum -a 256 \
    "payload/Spool.app/Contents/MacOS/SpoolMenu" \
    "payload/Spool.app/Contents/MacOS/SpoolPrintCoreReplay" \
    "payload/spool-agent" \
    "payload/spool-executor-cups" \
    > SHA256SUMS
)
ditto -c -k --sequesterRsrc --keepParent "$package_dir" "$package_zip"
ditto -c -k --sequesterRsrc --keepParent "$app" "$update_zip"
rm -rf -- "$package_dir"

shasum -a 256 "$package_zip" "$update_zip" > "$output_root/SHA256SUMS"
printf '%s\n%s\n' "$package_zip" "$update_zip"
