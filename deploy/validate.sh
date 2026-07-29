#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
chart="${root}/deploy/helm/spool"
values="${chart}/values-ci.yaml"
chart_arg="${chart}"
values_arg="${values}"
if ! command -v helm >/dev/null 2>&1; then
  chart_arg="deploy/helm/spool"
  values_arg="${chart_arg}/values-ci.yaml"
fi

run_helm() {
  if command -v helm >/dev/null 2>&1; then
    helm "$@"
  elif command -v docker >/dev/null 2>&1; then
    docker run --rm -v "${root}:/work" -w /work alpine/helm:3.16.4 "$@"
  else
    echo "helm or docker is required" >&2
    return 1
  fi
}

run_helm lint "${chart_arg}" --values "${values_arg}" --strict
rendered="$(mktemp)"
trap 'rm -f "${rendered}"' EXIT
run_helm template spool "${chart_arg}" \
  --namespace spool-system \
  --values "${values_arg}" >"${rendered}"

ruby -e '
  require "yaml"
  documents = YAML.load_stream(File.read(ARGV.fetch(0))).compact
  abort "no Kubernetes resources rendered" if documents.empty?
  required = %w[Deployment Service Job PodDisruptionBudget HorizontalPodAutoscaler NetworkPolicy ExternalSecret Ingress]
  kinds = documents.map { |doc| doc.fetch("kind") }.uniq
  missing = required - kinds
  abort "missing rendered kinds: #{missing.join(", ")}" unless missing.empty?
  names = documents.map { |doc| [doc.fetch("kind"), doc.fetch("metadata").fetch("name")] }
  abort "duplicate kind/name resources rendered" unless names.uniq.length == names.length
' "${rendered}"

if command -v kubeconform >/dev/null 2>&1; then
  kubeconform -strict -summary -ignore-missing-schemas "${rendered}"
elif command -v docker >/dev/null 2>&1; then
  docker run --rm -i ghcr.io/yannh/kubeconform:v0.6.7 \
    -strict -summary -ignore-missing-schemas <"${rendered}"
fi

if command -v terraform >/dev/null 2>&1; then
  terraform -chdir="${root}/deploy/terraform" fmt -check -recursive
  terraform -chdir="${root}/deploy/terraform" init -backend=false -input=false >/dev/null
  terraform -chdir="${root}/deploy/terraform" validate
elif command -v docker >/dev/null 2>&1; then
  docker run --rm -v "${root}:/work" -w /work/deploy/terraform \
    hashicorp/terraform:1.9.8 fmt -check -recursive
  docker run --rm --entrypoint sh -e TF_DATA_DIR=/tmp/tfdata \
    -v "${root}:/work" -w /work/deploy/terraform hashicorp/terraform:1.9.8 \
    -ec 'terraform init -backend=false -input=false >/dev/null && terraform validate'
else
  echo "terraform or docker is required" >&2
  exit 1
fi

echo "deploy validation passed"
