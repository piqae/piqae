#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
mkdir -p "${root}/.piqae-test-fixtures"
fixture="$(mktemp -d "${root}/.piqae-test-fixtures/promotion.XXXXXX")"
trap 'rm -rf -- "${fixture}"' EXIT
mkdir -p "${fixture}/bin"

cat >"${fixture}/bin/gcloud" <<'SCRIPT'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"${PROMOTION_COMMAND_LOG}"
if [[ "$*" == *" run deploy "* || "$*" == run\ deploy\ * ]]; then
  service="$3"
  printf '%s-r2\n' "${service}"
elif [[ "$*" == *"run services describe"* ]]; then
  printf '{"status":{"traffic":[{"tag":"candidate","url":"https://candidate.invalid"}]}}\n'
elif [[ "$*" == *"run revisions describe"* ]]; then
  printf 'True\n'
fi
SCRIPT
chmod +x "${fixture}/bin/gcloud"

cat >"${fixture}/bin/curl" <<'SCRIPT'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${PROMOTION_FAIL_CURL:-false}" == true ]]; then
  exit 22
fi
SCRIPT
chmod +x "${fixture}/bin/curl"

cat >"${fixture}/bin/sleep" <<'SCRIPT'
#!/usr/bin/env bash
exit 0
SCRIPT
chmod +x "${fixture}/bin/sleep"

export PATH="${fixture}/bin:${PATH}"
export PROMOTION_COMMAND_LOG="${fixture}/commands.log"
export PIQAE_GCP_PROJECT=test-project
export PIQAE_PRIMARY_GCP_REGION=australia-southeast1
export PIQAE_SECONDARY_GCP_REGION=australia-southeast2
export PIQAE_PRIMARY_API_SERVICE=piqae-production-api
export PIQAE_PRIMARY_SYNC_SERVICE=piqae-production-sync
export PIQAE_PRIMARY_WORKER_SERVICE=piqae-production-worker
export PIQAE_SECONDARY_API_SERVICE=piqae-production-secondary-api
export PIQAE_SECONDARY_SYNC_SERVICE=piqae-production-secondary-sync
export PIQAE_SECONDARY_WORKER_SERVICE=piqae-production-secondary-worker
export PIQAE_PRIMARY_API_PREVIOUS_REVISION=primary-api-r1
export PIQAE_PRIMARY_SYNC_PREVIOUS_REVISION=primary-sync-r1
export PIQAE_PRIMARY_WORKER_PREVIOUS_REVISION=primary-worker-r1
export PIQAE_SECONDARY_API_PREVIOUS_REVISION=secondary-api-r1
export PIQAE_SECONDARY_SYNC_PREVIOUS_REVISION=secondary-sync-r1
export PIQAE_SECONDARY_WORKER_PREVIOUS_REVISION=secondary-worker-r1
export PIQAE_MIGRATION_JOB=piqae-production-migrate
export PIQAE_SERVER_IMAGE=registry.invalid/piqae/server@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
export PIQAE_API_ORIGIN=https://api.piqae.invalid
export PIQAE_STAGE_5_PERCENT_SECONDS=0
export PIQAE_STAGE_25_PERCENT_SECONDS=0
export PIQAE_WORKER_OBSERVATION_SECONDS=0
export PIQAE_POST_CUTOVER_SECONDS=0

"${root}/deploy/cloud/promote.sh" >/dev/null
[[ "$(grep -c 'run jobs execute' "${PROMOTION_COMMAND_LOG}")" -eq 1 ]]
[[ "$(grep -c 'run deploy' "${PROMOTION_COMMAND_LOG}")" -eq 6 ]]
[[ "$(grep -c 'run services update-traffic' "${PROMOTION_COMMAND_LOG}")" -eq 14 ]]
grep -q 'piqae-production-worker-r2=100' "${PROMOTION_COMMAND_LOG}"
grep -q 'piqae-production-secondary-worker-r2=100' "${PROMOTION_COMMAND_LOG}"

: >"${PROMOTION_COMMAND_LOG}"
export PROMOTION_FAIL_CURL=true
if "${root}/deploy/cloud/promote.sh" >/dev/null 2>&1; then
  echo "failure fixture unexpectedly promoted" >&2
  exit 1
fi
[[ "$(grep -c 'run deploy' "${PROMOTION_COMMAND_LOG}")" -eq 6 ]]
[[ "$(grep -c 'run services update-traffic' "${PROMOTION_COMMAND_LOG}")" -eq 6 ]]
for revision in \
  primary-api-r1 primary-sync-r1 primary-worker-r1 \
  secondary-api-r1 secondary-sync-r1 secondary-worker-r1; do
  grep -q "${revision}=100" "${PROMOTION_COMMAND_LOG}"
done

echo "production promotion orchestration tests passed"
