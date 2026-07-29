#!/usr/bin/env bash
set -euo pipefail

archives=${1:-}
output=${2:-}
sparkle_bin=${SPARKLE_BIN_DIR:-}
private_key_file=${SPARKLE_PRIVATE_KEY_FILE:-}
download_prefix=${SPOOL_SPARKLE_DOWNLOAD_URL_PREFIX:-}

if [[ -z "$archives" || ! -d "$archives" || -z "$output" ]]; then
  echo "usage: SPARKLE_BIN_DIR=… SPARKLE_PRIVATE_KEY_FILE=… SPOOL_SPARKLE_DOWNLOAD_URL_PREFIX=https://… $0 /archives /archives/appcast.xml" >&2
  exit 2
fi
if [[ "$output" != "$archives/"* || "$output" != *.xml ]]; then
  echo "the appcast output must be an XML file inside the archives directory" >&2
  exit 2
fi
if [[ ! -x "$sparkle_bin/generate_appcast" || ! -r "$private_key_file" ]]; then
  echo "Sparkle generate_appcast or its private EdDSA key file is unavailable" >&2
  exit 1
fi
if [[ ! "$download_prefix" =~ ^https://[A-Za-z0-9][A-Za-z0-9.-]*(:[0-9]+)?(/[A-Za-z0-9._~:/?%+\&=-]*)?$ ]]; then
  echo "download URL prefix must be a safe HTTPS URL" >&2
  exit 2
fi

"$sparkle_bin/generate_appcast" \
  --ed-key-file "$private_key_file" \
  --download-url-prefix "$download_prefix" \
  --maximum-versions 5 \
  --maximum-deltas 3 \
  -o "$output" \
  "$archives"

if [[ ! -s "$output" ]]; then
  echo "Sparkle did not produce an appcast" >&2
  exit 1
fi
xmllint --noout "$output"
echo "$output"
