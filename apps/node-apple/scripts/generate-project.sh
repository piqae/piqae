#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
APP_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

if ! command -v xcodegen >/dev/null 2>&1; then
  echo "xcodegen 2.46 or newer is required" >&2
  exit 1
fi

xcodegen generate --spec "$APP_DIR/project.yml" --project "$APP_DIR"
