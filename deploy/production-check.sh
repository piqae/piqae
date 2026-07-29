#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mode="${1:-release}"

if [[ "${mode}" != "structural" && "${mode}" != "release" ]]; then
  echo "usage: $0 [structural|release]" >&2
  exit 2
fi

"${root}/deploy/validate.sh"
python3 "${root}/release/tools/check_workflow_pins.py" "${root}"/.github/workflows/*.yml
python3 -m unittest discover -s "${root}/release/tools" -p 'test_*.py' -v
ruby "${root}/release/tools/check_release_policy.rb"

if [[ "${mode}" == "release" ]]; then
  : "${SPOOL_PRODUCTION_VERCEL_ENV_FILE:?set to a protected exported Vercel environment file}"
  : "${SPOOL_PRODUCTION_TFVARS_FILE:?set to a protected production tfvars file}"
  : "${SPOOL_PRODUCTION_EVIDENCE_DIR:?set to the external release evidence directory}"
  (
    cd "${root}"
    cargo xtask release check
  )
  python3 "${root}/release/tools/check_production_readiness.py" \
    --mode release \
    --vercel-env "${SPOOL_PRODUCTION_VERCEL_ENV_FILE}" \
    --tfvars "${SPOOL_PRODUCTION_TFVARS_FILE}" \
    --evidence-dir "${SPOOL_PRODUCTION_EVIDENCE_DIR}"
else
  python3 "${root}/release/tools/check_production_readiness.py" --mode structural
fi
