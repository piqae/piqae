#!/usr/bin/env bash
set -euo pipefail

: "${SPOOL_GCP_PROJECT:?required}"
: "${SPOOL_PRIMARY_GCP_REGION:?required}"
: "${SPOOL_SECONDARY_GCP_REGION:?required}"
: "${SPOOL_PRIMARY_API_SERVICE:?required}"
: "${SPOOL_PRIMARY_SYNC_SERVICE:?required}"
: "${SPOOL_PRIMARY_WORKER_SERVICE:?required}"
: "${SPOOL_SECONDARY_API_SERVICE:?required}"
: "${SPOOL_SECONDARY_SYNC_SERVICE:?required}"
: "${SPOOL_SECONDARY_WORKER_SERVICE:?required}"
: "${SPOOL_PRIMARY_API_PREVIOUS_REVISION:?required}"
: "${SPOOL_PRIMARY_SYNC_PREVIOUS_REVISION:?required}"
: "${SPOOL_PRIMARY_WORKER_PREVIOUS_REVISION:?required}"
: "${SPOOL_SECONDARY_API_PREVIOUS_REVISION:?required}"
: "${SPOOL_SECONDARY_SYNC_PREVIOUS_REVISION:?required}"
: "${SPOOL_SECONDARY_WORKER_PREVIOUS_REVISION:?required}"
: "${SPOOL_MIGRATION_JOB:?required}"
: "${SPOOL_SERVER_IMAGE:?required}"
: "${SPOOL_API_ORIGIN:?required}"

if [[ ! "${SPOOL_SERVER_IMAGE}" =~ @sha256:[0-9a-f]{64}$ ]]; then
  echo "SPOOL_SERVER_IMAGE must be digest-pinned" >&2
  exit 2
fi
if [[ ! "${SPOOL_API_ORIGIN}" =~ ^https:// ]]; then
  echo "SPOOL_API_ORIGIN must use HTTPS" >&2
  exit 2
fi

stage_5_seconds="${SPOOL_STAGE_5_PERCENT_SECONDS:-21600}"
stage_25_seconds="${SPOOL_STAGE_25_PERCENT_SECONDS:-43200}"
worker_observation_seconds="${SPOOL_WORKER_OBSERVATION_SECONDS:-300}"
post_cutover_seconds="${SPOOL_POST_CUTOVER_SECONDS:-30}"

keys=(primary_api primary_sync primary_worker secondary_api secondary_sync secondary_worker)
traffic_indexes=(0 1 3 4)
worker_indexes=(2 5)
services=(
  "${SPOOL_PRIMARY_API_SERVICE}"
  "${SPOOL_PRIMARY_SYNC_SERVICE}"
  "${SPOOL_PRIMARY_WORKER_SERVICE}"
  "${SPOOL_SECONDARY_API_SERVICE}"
  "${SPOOL_SECONDARY_SYNC_SERVICE}"
  "${SPOOL_SECONDARY_WORKER_SERVICE}"
)
regions=(
  "${SPOOL_PRIMARY_GCP_REGION}"
  "${SPOOL_PRIMARY_GCP_REGION}"
  "${SPOOL_PRIMARY_GCP_REGION}"
  "${SPOOL_SECONDARY_GCP_REGION}"
  "${SPOOL_SECONDARY_GCP_REGION}"
  "${SPOOL_SECONDARY_GCP_REGION}"
)
previous=(
  "${SPOOL_PRIMARY_API_PREVIOUS_REVISION}"
  "${SPOOL_PRIMARY_SYNC_PREVIOUS_REVISION}"
  "${SPOOL_PRIMARY_WORKER_PREVIOUS_REVISION}"
  "${SPOOL_SECONDARY_API_PREVIOUS_REVISION}"
  "${SPOOL_SECONDARY_SYNC_PREVIOUS_REVISION}"
  "${SPOOL_SECONDARY_WORKER_PREVIOUS_REVISION}"
)
candidate=("" "" "" "" "" "")
candidate_url=("" "" "" "")
promotion_complete=false

set_traffic() {
  local index="$1"
  local allocation="$2"
  gcloud run services update-traffic "${services[$index]}" \
    --project "${SPOOL_GCP_PROJECT}" \
    --region "${regions[$index]}" \
    --to-revisions "${allocation}" \
    --quiet
}

rollback() {
  status=$?
  if [[ "${promotion_complete}" != true ]]; then
    echo "Promotion failed; restoring all changed Cloud Run services." >&2
    set +e
    for index in "${!keys[@]}"; do
      if [[ -n "${candidate[$index]}" ]]; then
        set_traffic "${index}" "${previous[$index]}=100"
      fi
    done
    set -e
  fi
  exit "${status}"
}
trap rollback EXIT

gcloud run jobs execute "${SPOOL_MIGRATION_JOB}" \
  --project "${SPOOL_GCP_PROJECT}" \
  --region "${SPOOL_PRIMARY_GCP_REGION}" \
  --wait \
  --quiet

for index in "${!keys[@]}"; do
  deploy_arguments=(
    run deploy "${services[$index]}"
    --project "${SPOOL_GCP_PROJECT}"
    --region "${regions[$index]}"
    --image "${SPOOL_SERVER_IMAGE}"
    --no-traffic
    --quiet
    --format=value\(status.latestCreatedRevisionName\)
  )
  if [[ "${keys[$index]}" == *_api || "${keys[$index]}" == *_sync ]]; then
    deploy_arguments+=(--tag candidate)
  fi
  candidate[$index]="$(gcloud "${deploy_arguments[@]}")"
  if [[ -z "${candidate[$index]}" || "${candidate[$index]}" == "${previous[$index]}" ]]; then
    echo "Cloud Run did not create a distinct ${keys[$index]} candidate revision" >&2
    exit 1
  fi
done

for offset in "${!traffic_indexes[@]}"; do
  index="${traffic_indexes[$offset]}"
  candidate_url[$offset]="$(gcloud run services describe "${services[$index]}" \
    --project "${SPOOL_GCP_PROJECT}" \
    --region "${regions[$index]}" \
    --format=json | jq -er '.status.traffic[] | select(.tag == "candidate") | .url')"
  curl --fail --silent --show-error --max-time 10 \
    "${candidate_url[$offset]}/v1/ready" >/dev/null
done

for index in "${worker_indexes[@]}"; do
  ready="$(gcloud run revisions describe "${candidate[$index]}" \
    --project "${SPOOL_GCP_PROJECT}" \
    --region "${regions[$index]}" \
    --format='value(status.conditions[?type=Ready].status)')"
  if [[ "${ready}" != "True" ]]; then
    echo "${keys[$index]} candidate revision is not ready" >&2
    exit 1
  fi
done

observe_traffic() {
  local seconds="$1"
  local deadline=$((SECONDS + seconds))
  while (( SECONDS < deadline )); do
    for offset in "${!traffic_indexes[@]}"; do
      curl --fail --silent --show-error --max-time 10 \
        "${candidate_url[$offset]}/v1/ready" >/dev/null
    done
    curl --fail --silent --show-error --max-time 10 \
      "${SPOOL_API_ORIGIN}/v1/ready" >/dev/null
    sleep 30
  done
}

for index in "${traffic_indexes[@]}"; do
  set_traffic "${index}" "${candidate[$index]}=5,${previous[$index]}=95"
done
observe_traffic "${stage_5_seconds}"

for index in "${traffic_indexes[@]}"; do
  set_traffic "${index}" "${candidate[$index]}=25,${previous[$index]}=75"
done
observe_traffic "${stage_25_seconds}"

for index in "${traffic_indexes[@]}"; do
  set_traffic "${index}" "${candidate[$index]}=100"
done
observe_traffic "${post_cutover_seconds}"

# Workers use durable PostgreSQL leases. They are never percentage-split:
# hand over one region at a time so each service has one traffic-owning revision,
# while the other region remains available. Concurrent regional workers are an
# intentional HA topology and claim disjoint outbox rows transactionally.
for index in "${worker_indexes[@]}"; do
  set_traffic "${index}" "${candidate[$index]}=100"
  sleep "${worker_observation_seconds}"
  ready="$(gcloud run revisions describe "${candidate[$index]}" \
    --project "${SPOOL_GCP_PROJECT}" \
    --region "${regions[$index]}" \
    --format='value(status.conditions[?type=Ready].status)')"
  if [[ "${ready}" != "True" ]]; then
    echo "${keys[$index]} failed after worker handover" >&2
    exit 1
  fi
done

curl --fail --silent --show-error --max-time 10 \
  "${SPOOL_API_ORIGIN}/v1/ready" >/dev/null
promotion_complete=true
trap - EXIT
echo "Promoted API, sync, and worker pools in both regions; update the reviewed Terraform digest before the next plan."
