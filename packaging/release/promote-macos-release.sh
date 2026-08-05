#!/usr/bin/env bash
set -euo pipefail

artifact_dir=${1:-}
version=${2:-}
build=${3:-}
source_sha=${4:-}
immutable_suffix=${5:-}

if [[ ! "$version" =~ ^[0-9]+([.][0-9]+){2}$ || ! "$build" =~ ^[1-9][0-9]*$ \
  || ! "$source_sha" =~ ^[a-f0-9]{40}$ \
  || ! "$immutable_suffix" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,80}$ ]]; then
  echo "invalid macOS promotion identity" >&2
  exit 2
fi
[[ -d "$artifact_dir" ]] || { echo "candidate artifact directory is missing" >&2; exit 2; }

user_source="$artifact_dir/Piqae-${version}-${build}-macos-installer.pkg"
update_source="$artifact_dir/Piqae-${version}-${build}-macos-update.zip"
user_name="piqae-macos-${version}-${build}-universal.pkg"
update_name="piqae-macos-${version}-${build}-update.zip"
sbom_name="piqae-macos-${version}-${build}-SBOM.spdx.json"
promotion=${PIQAE_PROMOTION_DIR:-promotion}
prefix="native/releases/$version/macos-${build}-${immutable_suffix}"

required=(
  "$user_source"
  "$update_source"
  "$artifact_dir/SBOM.spdx.json"
  "$artifact_dir/appcast.xml"
  "$artifact_dir/RELEASE-EVIDENCE.txt"
  "$artifact_dir/SHA256SUMS"
)
for path in "${required[@]}"; do
  [[ -s "$path" ]] || { echo "required candidate file is missing: $path" >&2; exit 1; }
done
(cd "$artifact_dir" && shasum -a 256 -c SHA256SUMS)

mkdir -p "$promotion"
install -m 0644 "$user_source" "$promotion/$user_name"
install -m 0644 "$update_source" "$promotion/$update_name"
install -m 0644 "$artifact_dir/SBOM.spdx.json" "$promotion/$sbom_name"
install -m 0644 "$artifact_dir/appcast.xml" "$promotion/appcast-macos.xml"
user_sha=$(shasum -a 256 "$promotion/$user_name" | awk '{print $1}')
printf '%s  %s\n' "$user_sha" "$user_name" > "$promotion/$user_name.sha256"
published_at=$(git show -s --format=%cI "$source_sha")
python3 release/tools/macos_release_metadata.py render \
  --version "$version" --build "$build" \
  --installer "$promotion/$user_name" --published-at "$published_at" \
  --output "$promotion/macos.json"
python3 release/tools/macos_release_metadata.py validate-appcast \
  --version "$version" --build "$build" \
  --appcast "$promotion/appcast-macos.xml" >/dev/null

for spec in \
  "$user_name:application/vnd.apple.installer+xml" \
  "$user_name.sha256:text/plain" \
  "$update_name:application/zip" \
  "$sbom_name:application/spdx+json" \
  "appcast-macos.xml:application/xml" \
  "macos.json:application/json"
do
  file=${spec%%:*}
  type=${spec#*:}
  packaging/release/publish-s3-object.sh immutable \
    "$promotion/$file" "$prefix/$file" "$type"
done

# Versioned and convenience assets are visible before the signed appcast.
packaging/release/publish-s3-object.sh promote "$prefix/$user_name" "native/stable/$user_name" application/vnd.apple.installer+xml
packaging/release/publish-s3-object.sh promote "$prefix/$user_name.sha256" "native/stable/$user_name.sha256" text/plain
packaging/release/publish-s3-object.sh promote "$prefix/$user_name" native/stable/piqae-macos-universal.pkg application/vnd.apple.installer+xml
packaging/release/publish-s3-object.sh promote "$prefix/$update_name" "native/stable/$update_name" application/zip
packaging/release/publish-s3-object.sh promote "$prefix/$update_name" native/stable/piqae-macos-update.zip application/zip
packaging/release/publish-s3-object.sh promote "$prefix/$sbom_name" "native/stable/$sbom_name" application/spdx+json

# Confirm the archive exists before committing the updater feed.
curl --fail --location --silent --show-error --max-time 30 \
  --range 0-0 "https://downloads.piqae.com/releases/stable/$update_name" --output /dev/null
packaging/release/publish-s3-object.sh promote \
  "$prefix/appcast-macos.xml" native/stable/appcast-macos.xml application/xml

if packaging/release/publish-s3-object.sh fetch native/stable/manifest.json "$promotion/current-manifest.json"; then
  jq -e '.schemaVersion == 1 and (.artifacts | type == "array")' "$promotion/current-manifest.json" >/dev/null
else
  code=$?
  [[ "$code" -eq 3 ]] || exit "$code"
  jq -n --arg repository_url "https://github.com/$GITHUB_REPOSITORY" \
    '{schemaVersion:1,channel:"stable",currentVersion:"0.0.0",updatedAt:null,artifacts:[],olderReleases:[],releasesUrl:"https://piqae.com/downloads",repositoryUrl:$repository_url}' \
    > "$promotion/current-manifest.json"
fi
jq --slurpfile platform "$promotion/macos.json" \
  --arg repository_url "https://github.com/$GITHUB_REPOSITORY" \
  --arg version "$version" --arg now "$published_at" \
  '.channel="stable" | .currentVersion=$version | .updatedAt=$now
   | .repositoryUrl=$repository_url
   | .artifacts=([.artifacts[] | select(.id != $platform[0].artifact.id)] + [$platform[0].artifact])' \
  "$promotion/current-manifest.json" > "$promotion/manifest.json"
jq -e --arg version "$version" \
  '.schemaVersion == 1 and .channel == "stable" and .currentVersion == $version
   and all(.artifacts[]; .status != "supported" and .signing.status == "verified"
     and (.sha256 | test("^[a-f0-9]{64}$"))
     and (.downloadUrl | startswith("https://downloads.piqae.com/releases/stable/")))' \
  "$promotion/manifest.json" >/dev/null
manifest_sha=$(shasum -a 256 "$promotion/manifest.json" | awk '{print $1}')
printf '%s  manifest.json\n' "$manifest_sha" > "$promotion/manifest.json.sha256"
packaging/release/publish-s3-object.sh immutable "$promotion/manifest.json" "$prefix/manifest.json" application/json
packaging/release/publish-s3-object.sh immutable "$promotion/manifest.json.sha256" "$prefix/manifest.json.sha256" text/plain
packaging/release/publish-s3-object.sh promote "$prefix/manifest.json.sha256" native/stable/manifest.json.sha256 text/plain
packaging/release/publish-s3-object.sh promote "$prefix/manifest.json" native/stable/manifest.json application/json

# The public channel is now committed; verify both metadata and bytes.
public_appcast=$(mktemp)
public_manifest=$(mktemp)
public_installer=$(mktemp)
trap 'rm -f -- "$public_appcast" "$public_manifest" "$public_installer"' EXIT
curl --fail --location --silent --show-error --max-time 30 \
  https://downloads.piqae.com/releases/stable/appcast-macos.xml --output "$public_appcast"
python3 release/tools/macos_release_metadata.py validate-appcast \
  --version "$version" --build "$build" --appcast "$public_appcast" >/dev/null
curl --fail --location --silent --show-error --max-time 30 \
  https://downloads.piqae.com/releases/stable/manifest.json --output "$public_manifest"
jq -e --arg version "$version" --arg download "https://downloads.piqae.com/releases/stable/$user_name" \
  '.currentVersion == $version and any(.artifacts[]; .id == "macos-universal" and .version == $version and .downloadUrl == $download)' \
  "$public_manifest" >/dev/null
curl --fail --location --silent --show-error --max-time 180 \
  "https://downloads.piqae.com/releases/stable/$user_name" --output "$public_installer"
[[ "$(shasum -a 256 "$public_installer" | awk '{print $1}')" == "$user_sha" ]]

tag="v$version"
if gh release view "$tag" >/dev/null 2>&1; then
  gh release view "$tag" --json isDraft --jq '.isDraft' | grep -Fx true
else
  gh release create "$tag" --draft --verify-tag --title "Piqae $tag" \
    --notes "Preview candidate. Review checksums, provenance, signatures, notarisation evidence, and the support matrix before publishing."
fi
gh release upload "$tag" "$promotion"/piqae-macos-* "$promotion/appcast-macos.xml" --clobber

{
  echo "### macOS $version ($build) promoted"
  echo "- Source: \`$source_sha\`"
  echo "- Immutable prefix: \`$prefix\`"
  echo "- Installer SHA-256: \`$user_sha\`"
  echo "- Public appcast and manifest verified"
} >> "${GITHUB_STEP_SUMMARY:-/dev/null}"
