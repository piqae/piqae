#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
derived_data="$repository_root/.piqae-test-fixtures/apple-sdk-derived-data"

swift test \
  --package-path "$repository_root/sdk/apple" \
  -Xswiftc -strict-concurrency=complete \
  -Xswiftc -warnings-as-errors

mkdir -p "$(dirname "$derived_data")"
(
  cd "$repository_root/sdk/apple"
  xcodebuild \
    -scheme PiqaeNodeKit-Package \
    -destination 'generic/platform=iOS Simulator' \
    -derivedDataPath "$derived_data" \
    CODE_SIGNING_ALLOWED=NO \
    SWIFT_TREAT_WARNINGS_AS_ERRORS=YES \
    SWIFT_STRICT_CONCURRENCY=complete \
    build
)
