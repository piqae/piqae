#!/usr/bin/env bash
set -euo pipefail

identity=${SPOOL_CODE_SIGN_IDENTITY:-}

if [[ "$#" -eq 0 ]]; then
  echo "usage: SPOOL_CODE_SIGN_IDENTITY='Developer ID Application: …' $0 /path/to/binary [...]" >&2
  exit 2
fi
if [[ "$identity" != "Developer ID Application:"* ]]; then
  echo "SPOOL_CODE_SIGN_IDENTITY must name a Developer ID Application certificate" >&2
  exit 2
fi
if ! security find-identity -v -p codesigning | grep -F -- "$identity" >/dev/null; then
  echo "Developer ID Application identity is not available in the active keychain" >&2
  exit 1
fi

for component in "$@"; do
  if [[ ! -f "$component" || ! -x "$component" ]]; then
    echo "release component is missing or not executable: $component" >&2
    exit 1
  fi
  if ! file "$component" | grep -F "Mach-O" >/dev/null; then
    echo "release component is not a Mach-O executable: $component" >&2
    exit 1
  fi

  codesign \
    --force \
    --timestamp \
    --options runtime \
    --sign "$identity" \
    "$component"
  codesign --verify --strict --verbose=2 "$component"
  codesign -dv --verbose=4 "$component" 2>&1 |
    grep -F "Authority=Developer ID Application:"
done
