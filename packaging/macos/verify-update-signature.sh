#!/usr/bin/env bash
set -euo pipefail

archive=${1:-}
appcast=${2:-}
public_key=${SPOOL_SPARKLE_PUBLIC_ED_KEY:-}
script_root=$(cd "$(dirname "$0")" && pwd)

if [[ ! -f "$archive" || ! -f "$appcast" || -z "$public_key" ]]; then
  echo "usage: SPOOL_SPARKLE_PUBLIC_ED_KEY=… $0 /archive.zip /appcast.xml" >&2
  exit 2
fi

filename=$(basename "$archive")
if [[ ! "$filename" =~ ^[A-Za-z0-9._-]+$ ]]; then
  echo "archive filename contains unsafe appcast characters" >&2
  exit 2
fi
xpath="string(//*[local-name()='enclosure' and substring(@url, string-length(@url) - string-length('$filename') + 1) = '$filename']/@*[local-name()='edSignature'])"
signature=$(xmllint --xpath "$xpath" "$appcast")
length_xpath="string(//*[local-name()='enclosure' and substring(@url, string-length(@url) - string-length('$filename') + 1) = '$filename']/@length)"
declared_length=$(xmllint --xpath "$length_xpath" "$appcast")
actual_length=$(stat -f '%z' "$archive")

if [[ -z "$signature" || ! "$declared_length" =~ ^[0-9]+$ ]]; then
  echo "appcast has no signed enclosure for $filename" >&2
  exit 1
fi
if [[ "$declared_length" != "$actual_length" ]]; then
  echo "appcast length does not match $filename" >&2
  exit 1
fi

swift "$script_root/verify-update-signature.swift" "$archive" "$public_key" "$signature"
echo "Verified Sparkle Ed25519 signature for $filename"
