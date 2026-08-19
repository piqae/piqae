#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mode="${1:-release}"

if [[ "${mode}" != "structural" && "${mode}" != "release" && "${mode}" != "managed-ha" ]]; then
  echo "usage: $0 [structural|release|managed-ha]" >&2
  exit 2
fi

"${root}/deploy/validate.sh"
python3 "${root}/release/tools/check_workflow_pins.py" "${root}"/.github/workflows/*.yml
python3 -m unittest discover -s "${root}/release/tools" -p 'test_*.py' -v
ruby "${root}/release/tools/check_release_policy.rb"
ruby "${root}/release/tools/check_product_release.rb"

if [[ "${mode}" == "release" || "${mode}" == "managed-ha" ]]; then
  : "${PIQAE_PRODUCTION_RAILWAY_ENV_FILE:?set to a protected exported Railway web environment file}"
  : "${PIQAE_PRODUCTION_EVIDENCE_DIR:?set to the external release evidence directory}"
  (
    cd "${root}"
    cargo xtask release check
  )
  arguments=(
    --mode release
    --target railway
    --railway-env "${PIQAE_PRODUCTION_RAILWAY_ENV_FILE}"
    --evidence-dir "${PIQAE_PRODUCTION_EVIDENCE_DIR}"
  )
  if [[ "${mode}" == "managed-ha" ]]; then
    : "${PIQAE_PRODUCTION_TFVARS_FILE:?set to a protected managed-HA production tfvars file}"
    arguments=(
      --mode release
      --target managed-ha
      --railway-env "${PIQAE_PRODUCTION_RAILWAY_ENV_FILE}"
      --tfvars "${PIQAE_PRODUCTION_TFVARS_FILE}"
      --evidence-dir "${PIQAE_PRODUCTION_EVIDENCE_DIR}"
    )
  fi
  python3 "${root}/release/tools/check_production_readiness.py" "${arguments[@]}"
else
  python3 "${root}/release/tools/check_production_readiness.py" --mode structural
fi
