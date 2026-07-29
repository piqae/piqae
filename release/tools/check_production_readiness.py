#!/usr/bin/env python3
"""Fail-closed repository and deployment preflight for Spool Cloud."""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
REQUIRED_COMPONENTS = {
    "cloud_run",
    "cloud_sql",
    "gcs",
    "vercel",
    "workos",
    "stripe",
    "sentry",
}
REQUIRED_VERCEL_KEYS = {
    "SPOOL_AUTH_MODE",
    "PUBLIC_SPOOL_DASHBOARD_MODE",
    "PUBLIC_SPOOL_API_URL",
    "PUBLIC_SITE_URL",
    "WORKOS_CLIENT_ID",
    "WORKOS_API_KEY",
    "WORKOS_REDIRECT_URI",
    "WORKOS_COOKIE_PASSWORD",
    "STRIPE_CHECKOUT_ENABLED",
    "STRIPE_SECRET_KEY",
    "STRIPE_PRICE_PRO_MONTHLY",
    "STRIPE_PRICE_PRO_ANNUAL",
    "STRIPE_PRICE_PRO_OVERAGE_MONTHLY",
    "STRIPE_PRICE_PRO_OVERAGE_ANNUAL",
    "PRICING_DRIFT_SHARED_SECRET",
    "SENTRY_DSN",
    "PUBLIC_SENTRY_DSN",
    "SENTRY_ENVIRONMENT",
    "PUBLIC_SENTRY_ENVIRONMENT",
    "SENTRY_TRACES_SAMPLE_RATE",
    "PUBLIC_SENTRY_TRACES_SAMPLE_RATE",
    "SENTRY_AUTH_TOKEN",
    "SENTRY_ORG",
    "SENTRY_PROJECT",
    "SENTRY_RELEASE",
    "SPOOL_RELEASE_MANIFEST_JSON",
    "PUBLIC_MARKETING_INDEXABLE",
}
SECRET_KEYS = {
    "WORKOS_API_KEY",
    "WORKOS_COOKIE_PASSWORD",
    "STRIPE_SECRET_KEY",
    "PRICING_DRIFT_SHARED_SECRET",
    "SENTRY_AUTH_TOKEN",
}


class PreflightError(RuntimeError):
    pass


def parse_env(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for line_number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            raise PreflightError(f"{path}:{line_number}: expected KEY=VALUE")
        key, value = line.split("=", 1)
        if not re.fullmatch(r"[A-Z][A-Z0-9_]*", key):
            raise PreflightError(f"{path}:{line_number}: invalid environment key")
        if key in values:
            raise PreflightError(f"{path}:{line_number}: duplicate key {key}")
        values[key] = value
    return values


def require_text(path: Path, snippets: list[str], errors: list[str]) -> str:
    if not path.is_file():
        errors.append(f"missing {path.relative_to(ROOT)}")
        return ""
    text = path.read_text(encoding="utf-8")
    for snippet in snippets:
        if snippet not in text:
            errors.append(f"{path.relative_to(ROOT)} is missing required contract: {snippet}")
    return text


def check_migrations(errors: list[str]) -> None:
    directory = ROOT / "migrations/postgres"
    versions: list[int] = []
    names: set[str] = set()
    for path in sorted(directory.glob("*.sql")):
        match = re.fullmatch(r"(\d{4})_[a-z0-9_]+\.sql", path.name)
        if match is None:
            errors.append(f"invalid migration filename: {path.name}")
            continue
        version = int(match.group(1))
        if path.name in names or version in versions:
            errors.append(f"duplicate migration version: {version:04d}")
        names.add(path.name)
        versions.append(version)
    if not versions:
        errors.append("no PostgreSQL migrations found")
        return
    expected = list(range(1, max(versions) + 1))
    if sorted(versions) != expected:
        errors.append(f"migration sequence is not contiguous: found {sorted(versions)}")


def structural_errors() -> list[str]:
    errors: list[str] = []
    contract_path = ROOT / "release/production-readiness.json"
    try:
        contract = json.loads(contract_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        return [f"cannot load production contract: {error}"]

    if contract.get("commercial_plans") != ["free", "pro"]:
        errors.append("commercial plan contract must be exactly Free and Pro")
    if set(contract.get("hosted_components", {})) != REQUIRED_COMPONENTS:
        errors.append("production contract must require every hosted component")
    evidence = contract.get("external_evidence")
    if not isinstance(evidence, list) or not evidence:
        errors.append("external evidence gates are missing")
    elif any(item.get("status") != "open" for item in evidence):
        errors.append("checked-in external evidence declarations must remain open")

    template_path = ROOT / "deploy/hosted/vercel.env.example"
    try:
        template = parse_env(template_path)
    except (OSError, PreflightError) as error:
        errors.append(str(error))
        template = {}
    missing_keys = sorted(REQUIRED_VERCEL_KEYS - set(template))
    if missing_keys:
        errors.append(f"Vercel template is missing: {', '.join(missing_keys)}")
    if template.get("STRIPE_CHECKOUT_ENABLED") != "false":
        errors.append("example Stripe checkout must fail closed")
    if template.get("PUBLIC_MARKETING_INDEXABLE") != "false":
        errors.append("example marketing indexing must fail closed")
    for key in SECRET_KEYS:
        if template.get(key):
            errors.append(f"example must not contain a value for {key}")

    terraform_main = require_text(
        ROOT / "deploy/terraform/main.tf",
        [
            'for_each = var.allow_public_cloud_run_invocation ? toset(["api", "sync"]) : toset([])',
            'path = "/v1/ready"',
            'path = "/v1/health"',
            "deletion_protection = var.environment == \"production\"",
            'name  = "SPOOL_DEPLOYMENT"',
            'value = "cloud"',
            'name  = "SPOOL_IDENTITY_PROVIDER"',
            'value = "workos"',
            'resource "google_cloud_run_v2_job" "migration"',
            'name = "STRIPE_WEBHOOK_SECRET"',
            'for_each            = toset(["api", "sync", "worker"])',
        ],
        errors,
    )
    if 'resource "google_cloud_run_v2_service_iam_member" "primary_invoker"' in terraform_main:
        errors.append("duplicate primary Cloud Run public IAM resource remains")
    require_text(
        ROOT / "deploy/terraform/variables.tf",
        [
            '@sha256:[0-9a-f]{64}$',
            'object_store_endpoint must use HTTPS',
            'webhook_master_key_secret must be base64 that decodes to exactly 32 bytes',
        ],
        errors,
    )
    require_text(
        ROOT / "deploy/terraform/ha.tf",
        [
            'path = "/v1/ready"',
            'availability_type = "REGIONAL"',
            "point_in_time_recovery_enabled = true",
            'master_instance_name = google_sql_database_instance.primary[0].name',
            "custom_placement_config",
            "versioning { enabled = true }",
            'global_sync',
            '"/v1/agent/jobs/*"',
        ],
        errors,
    )
    require_text(
        ROOT / "deploy/self-host/docker-compose.yml",
        ['http://127.0.0.1:8080/v1/ready'],
        errors,
    )
    require_text(
        ROOT / "deploy/helm/spool/templates/deployments.yaml",
        [
            "maxUnavailable: 0",
            "readinessProbe:",
            "path: /v1/ready",
        ],
        errors,
    )
    require_text(
        ROOT / "deploy/helm/spool/templates/migration-job.yaml",
        [
            "helm.sh/hook: post-install,pre-upgrade",
            "activeDeadlineSeconds:",
        ],
        errors,
    )
    require_text(
        ROOT / ".github/workflows/release-evidence.yml",
        [
            "actions/attest-build-provenance@",
            "--github-repository",
        ],
        errors,
    )
    generic_release = require_text(ROOT / ".github/workflows/release.yml", [], errors)
    if "gh release create" in generic_release:
        errors.append("generic unsigned preview workflow may publish a GitHub release")
    require_text(
        ROOT / ".github/workflows/production-promotion.yml",
        [
            "environment: production",
            "./deploy/production-check.sh release",
            "./deploy/cloud/promote.sh",
        ],
        errors,
    )
    require_text(
        ROOT / "deploy/cloud/promote.sh",
        [
            "gcloud run jobs execute",
            "=5,",
            "=25,",
            "rollback",
            "SPOOL_PRIMARY_API_SERVICE",
            "SPOOL_PRIMARY_SYNC_SERVICE",
            "SPOOL_PRIMARY_WORKER_SERVICE",
            "SPOOL_SECONDARY_API_SERVICE",
            "SPOOL_SECONDARY_SYNC_SERVICE",
            "SPOOL_SECONDARY_WORKER_SERVICE",
            'worker_indexes=(2 5)',
        ],
        errors,
    )

    check_migrations(errors)
    return errors


def require_https(name: str, value: str, errors: list[str]) -> None:
    if not value.startswith("https://"):
        errors.append(f"{name} must use HTTPS")


def release_errors(vercel_env: Path | None, tfvars: Path | None, evidence_dir: Path | None) -> list[str]:
    errors = structural_errors()
    if vercel_env is None or not vercel_env.is_file():
        errors.append("release preflight requires --vercel-env")
        values: dict[str, str] = {}
    else:
        try:
            values = parse_env(vercel_env)
        except PreflightError as error:
            errors.append(str(error))
            values = {}

    for key in REQUIRED_VERCEL_KEYS:
        if not values.get(key):
            errors.append(f"production Vercel environment is missing {key}")
    if values.get("SPOOL_AUTH_MODE") != "workos":
        errors.append("production SPOOL_AUTH_MODE must be workos")
    if values.get("PUBLIC_SPOOL_DASHBOARD_MODE") != "live":
        errors.append("production dashboard mode must be live")
    if values.get("STRIPE_CHECKOUT_ENABLED") != "true":
        errors.append("production Stripe checkout must be explicitly enabled")
    if values.get("PUBLIC_MARKETING_INDEXABLE") != "true":
        errors.append("production indexing must be an explicit release decision")
    for key in ("SENTRY_ENVIRONMENT", "PUBLIC_SENTRY_ENVIRONMENT"):
        if values.get(key) != "production":
            errors.append(f"production {key} must be production")
    for key in ("SENTRY_TRACES_SAMPLE_RATE", "PUBLIC_SENTRY_TRACES_SAMPLE_RATE"):
        try:
            sample_rate = float(values.get(key, ""))
        except ValueError:
            sample_rate = -1
        if not 0 <= sample_rate <= 1:
            errors.append(f"{key} must be between 0 and 1")
    if re.search(r"replace|example|latest", values.get("SENTRY_RELEASE", ""), re.IGNORECASE):
        errors.append("SENTRY_RELEASE must identify the immutable promoted release")
    for key in ("PUBLIC_SPOOL_API_URL", "PUBLIC_SITE_URL", "WORKOS_REDIRECT_URI"):
        if values.get(key):
            require_https(key, values[key], errors)

    if tfvars is None or not tfvars.is_file():
        errors.append("release preflight requires --tfvars")
    else:
        text = tfvars.read_text(encoding="utf-8")
        for setting in (
            "environment",
            "image",
            "enable_multi_region",
            "enable_global_load_balancer",
            "allow_public_cloud_run_invocation",
            "enable_managed_data_plane",
            "managed_object_bucket_name",
            "load_balancer_domains",
            "stripe_meter_event_name",
        ):
            if not re.search(rf"(?m)^\s*{re.escape(setting)}\s*=", text):
                errors.append(f"production tfvars is missing {setting}")
        for setting in (
            "enable_multi_region",
            "enable_global_load_balancer",
            "allow_public_cloud_run_invocation",
            "enable_managed_data_plane",
        ):
            if not re.search(rf"(?m)^\s*{setting}\s*=\s*true\s*$", text):
                errors.append(f"production tfvars must set {setting}=true")
        if not re.search(r'(?m)^\s*image\s*=\s*"[^"]+@sha256:[0-9a-f]{64}"\s*$', text):
            errors.append("production tfvars must pin image by digest")
        if re.search(r"replace|example\.com|000000000000", text, re.IGNORECASE):
            errors.append("production tfvars still contains an example placeholder")

    app_sources = "\n".join(
        path.read_text(encoding="utf-8")
        for path in (ROOT / "apps/web/src").rglob("*")
        if path.suffix in {".ts", ".js", ".svelte"}
    )
    for key in (
        "STRIPE_PRICE_PRO_MONTHLY",
        "STRIPE_PRICE_PRO_ANNUAL",
        "STRIPE_PRICE_PRO_OVERAGE_MONTHLY",
        "STRIPE_PRICE_PRO_OVERAGE_ANNUAL",
    ):
        if key not in app_sources:
            errors.append(f"web runtime does not consume canonical {key}")
    package = (ROOT / "apps/web/package.json").read_text(encoding="utf-8")
    if (
        "@sentry/sveltekit" not in package
        or "SENTRY_DSN" not in app_sources
        or "PUBLIC_SENTRY_DSN" not in app_sources
    ):
        errors.append("Sentry web SDK/runtime integration is not implemented")
    control_plane = (ROOT / "crates/control-plane/src/main.rs").read_text(encoding="utf-8")
    if not re.search(r'"gcs"\s*=>', control_plane):
        errors.append("native GCS object-store runtime is not implemented")
    if "SPOOL_SERVICE_ROLE" not in control_plane:
        errors.append("api/sync/worker service-role isolation is not implemented")
    if "SPOOL_RUN_MIGRATIONS_ON_STARTUP" not in control_plane or "migrate_only" not in control_plane:
        errors.append("server still runs database migrations during replica startup")
    if "STRIPE_WEBHOOK_SECRET" not in control_plane:
        errors.append("control plane does not consume STRIPE_WEBHOOK_SECRET")
    for key in ("STRIPE_SECRET_KEY", "STRIPE_METER_EVENT_NAME"):
        if key not in control_plane:
            errors.append(f"billing meter worker does not consume {key}")

    contract = json.loads((ROOT / "release/production-readiness.json").read_text(encoding="utf-8"))
    if evidence_dir is None or not evidence_dir.is_dir():
        errors.append("release preflight requires --evidence-dir with external records")
    else:
        for gate in contract["external_evidence"]:
            path = evidence_dir / gate["required_file"]
            if not path.is_file():
                errors.append(f"missing external evidence: {gate['id']}")
                continue
            try:
                record = json.loads(path.read_text(encoding="utf-8"))
            except json.JSONDecodeError:
                errors.append(f"invalid external evidence JSON: {gate['id']}")
                continue
            if record.get("gate") != gate["id"] or record.get("status") != "passed":
                errors.append(f"external evidence did not pass: {gate['id']}")
            if not re.fullmatch(r"[0-9a-f]{40}", str(record.get("commit", ""))):
                errors.append(f"external evidence lacks a full commit: {gate['id']}")
            if not record.get("recorded_at") or not record.get("evidence_url"):
                errors.append(f"external evidence lacks timestamp/URL: {gate['id']}")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mode", choices=("structural", "release"), default="structural")
    parser.add_argument("--vercel-env", type=Path)
    parser.add_argument("--tfvars", type=Path)
    parser.add_argument("--evidence-dir", type=Path)
    args = parser.parse_args()
    errors = (
        structural_errors()
        if args.mode == "structural"
        else release_errors(args.vercel_env, args.tfvars, args.evidence_dir)
    )
    if errors:
        for error in errors:
            print(f"FAIL: {error}", file=sys.stderr)
        return 1
    print(f"production {args.mode} preflight passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
