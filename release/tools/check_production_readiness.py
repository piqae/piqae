#!/usr/bin/env python3
"""Fail-closed repository and deployment preflight for Piqae Cloud."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
REQUIRED_COMPONENTS = {
    "railway_web",
    "railway_api",
    "railway_worker",
    "railway_postgres",
    "railway_document_bucket",
    "railway_release_bucket",
    "workos",
    "stripe",
    "sentry",
}
OPTIONAL_SCALE_UP_COMPONENTS = {
    "cloud_run",
    "cloud_sql",
    "gcs",
    "kubernetes",
}
REQUIRED_RAILWAY_WEB_KEYS = {
    "PIQAE_AUTH_MODE",
    "PUBLIC_PIQAE_DASHBOARD_MODE",
    "PUBLIC_PIQAE_API_URL",
    "PUBLIC_SITE_URL",
    "ORIGIN",
    "PIQAE_COOKIE_SECURE",
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
    "PIQAE_RELEASE_MANIFEST_JSON",
    "PIQAE_RELEASES_S3_ENDPOINT",
    "PIQAE_RELEASES_S3_ACCESS_KEY_ID",
    "PIQAE_RELEASES_S3_SECRET_ACCESS_KEY",
    "PIQAE_RELEASES_S3_BUCKET",
    "PIQAE_RELEASES_S3_REGION",
    "PIQAE_RELEASES_S3_VIRTUAL_HOSTED_STYLE",
    "PUBLIC_MARKETING_INDEXABLE",
}
SECRET_KEYS = {
    "WORKOS_API_KEY",
    "WORKOS_COOKIE_PASSWORD",
    "STRIPE_SECRET_KEY",
    "PRICING_DRIFT_SHARED_SECRET",
    "SENTRY_AUTH_TOKEN",
    "PIQAE_RELEASES_S3_ACCESS_KEY_ID",
    "PIQAE_RELEASES_S3_SECRET_ACCESS_KEY",
}


class PreflightError(RuntimeError):
    pass


def hcl_block(source: str, declaration: str) -> str:
    """Return one balanced HCL block while ignoring braces inside strings."""
    start = source.find(declaration)
    if start < 0:
        return ""
    opening = source.find("{", start + len(declaration))
    if opening < 0:
        return ""
    depth = 0
    quoted = False
    escaped = False
    for offset in range(opening, len(source)):
        character = source[offset]
        if escaped:
            escaped = False
            continue
        if quoted and character == "\\":
            escaped = True
            continue
        if character == '"':
            quoted = not quoted
            continue
        if quoted:
            continue
        if character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
            if depth == 0:
                return source[start : offset + 1]
    return ""


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
    if contract.get("current_target") != "railway_private_beta":
        errors.append("current hosted target must be railway_private_beta")
    if set(contract.get("hosted_components", {})) != REQUIRED_COMPONENTS:
        errors.append("production contract must require every hosted component")
    if any(value != "required" for value in contract.get("hosted_components", {}).values()):
        errors.append("current hosted components must remain required")
    if set(contract.get("optional_scale_up_components", {})) != OPTIONAL_SCALE_UP_COMPONENTS:
        errors.append("production contract must declare every optional scale-up component")
    if any(
        value != "optional"
        for value in contract.get("optional_scale_up_components", {}).values()
    ):
        errors.append("scale-up components must remain optional for Railway private beta")
    for key in (
        "external_evidence",
        "managed_ha_external_evidence",
        "public_release_external_evidence",
    ):
        evidence = contract.get(key)
        if not isinstance(evidence, list) or not evidence:
            errors.append(f"{key} gates are missing")
        elif any(item.get("status") != "open" for item in evidence):
            errors.append("checked-in external evidence declarations must remain open")

    template_path = ROOT / "deploy/hosted/railway.env.example"
    try:
        template = parse_env(template_path)
    except (OSError, PreflightError) as error:
        errors.append(str(error))
        template = {}
    missing_keys = sorted(REQUIRED_RAILWAY_WEB_KEYS - set(template))
    if missing_keys:
        errors.append(f"Railway web template is missing: {', '.join(missing_keys)}")
    if template.get("STRIPE_CHECKOUT_ENABLED") != "false":
        errors.append("example Stripe checkout must fail closed")
    if template.get("PUBLIC_MARKETING_INDEXABLE") != "false":
        errors.append("example marketing indexing must fail closed")
    for key in SECRET_KEYS:
        if template.get(key):
            errors.append(f"example must not contain a value for {key}")

    require_text(
        ROOT / "railway.toml",
        [
            'dockerfilePath = "deploy/docker/Dockerfile.server"',
            'healthcheckPath = "/v1/ready"',
        ],
        errors,
    )
    require_text(
        ROOT / "railway.web.toml",
        [
            'dockerfilePath = "deploy/docker/Dockerfile.web"',
            'healthcheckPath = "/healthz"',
        ],
        errors,
    )

    terraform_main = require_text(
        ROOT / "deploy/terraform/main.tf",
        [
            'for_each = var.allow_public_cloud_run_invocation ? toset(["api", "sync"]) : toset([])',
            'path = "/v1/ready"',
            'path = "/v1/health"',
            "deletion_protection = var.environment == \"production\"",
            'name  = "PIQAE_DEPLOYMENT"',
            'value = "cloud"',
            'name  = "PIQAE_IDENTITY_PROVIDER"',
            'value = "workos"',
            'resource "google_cloud_run_v2_job" "migration"',
            'name = "STRIPE_WEBHOOK_SECRET"',
            'name = "PIQAE_DESTINATION_IDENTITY_KEY"',
            'google_secret_manager_secret.destination_identity_key.id',
            'for_each            = toset(["api", "sync", "worker"])',
        ],
        errors,
    )
    if 'resource "google_cloud_run_v2_service_iam_member" "primary_invoker"' in terraform_main:
        errors.append("duplicate primary Cloud Run public IAM resource remains")
    terraform_variables = require_text(
        ROOT / "deploy/terraform/variables.tf",
        [
            '@sha256:[0-9a-f]{64}$',
            'object_store_endpoint must use HTTPS',
            'webhook_master_key_secret must be canonical standard Base64 for exactly 32 bytes',
            'destination_identity_key_secret must be canonical standard Base64 for exactly 32 bytes',
            'webhook_master_key_secret, destination_identity_key_secret, and document_master_key_secret must be pairwise distinct',
        ],
        errors,
    )
    destination_variable = hcl_block(
        terraform_variables, 'variable "destination_identity_key_secret"'
    )
    if not destination_variable:
        errors.append("Terraform destination identity variable block is missing")
    else:
        for relationship in (
            'regex("^[A-Za-z0-9+/]{42}[AEIMQUYcgkosw048]=$", var.destination_identity_key_secret)',
            "var.destination_identity_key_secret != var.webhook_master_key_secret",
            "var.destination_identity_key_secret != var.document_master_key_secret",
            "var.webhook_master_key_secret != var.document_master_key_secret",
        ):
            if relationship not in destination_variable:
                errors.append(
                    "Terraform destination identity validation is incomplete: "
                    f"missing {relationship}"
                )

    destination_version = hcl_block(
        terraform_main,
        'resource "google_secret_manager_secret_version" "destination_identity_key"',
    )
    if not destination_version or not all(
        relationship in destination_version
        for relationship in (
            "google_secret_manager_secret.destination_identity_key.id",
            "var.destination_identity_key_secret",
        )
    ):
        errors.append(
            "Terraform destination identity Secret Manager version is not wired to its input"
        )
    runtime_iam = hcl_block(
        terraform_main, 'resource "google_secret_manager_secret_iam_member" "runtime_secrets"'
    )
    if "google_secret_manager_secret.destination_identity_key.id" not in runtime_iam:
        errors.append("Terraform runtime service account cannot access the destination identity secret")
    primary_services = hcl_block(
        terraform_main, 'resource "google_cloud_run_v2_service" "server"'
    )
    if not re.search(
        r'name\s*=\s*"PIQAE_DESTINATION_IDENTITY_KEY".*?secret\s*=\s*google_secret_manager_secret\.destination_identity_key\.secret_id.*?version\s*=\s*google_secret_manager_secret_version\.destination_identity_key\.version',
        primary_services,
        flags=re.DOTALL,
    ):
        errors.append("Terraform primary Cloud Run services do not consume the destination identity secret")

    terraform_ha = require_text(
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
            'name = "PIQAE_DESTINATION_IDENTITY_KEY"',
            'version = google_secret_manager_secret_version.destination_identity_key.version',
        ],
        errors,
    )
    secondary_services = hcl_block(
        terraform_ha, 'resource "google_cloud_run_v2_service" "server_secondary"'
    )
    if not re.search(
        r'name\s*=\s*"PIQAE_DESTINATION_IDENTITY_KEY".*?secret\s*=\s*google_secret_manager_secret\.destination_identity_key\.secret_id.*?version\s*=\s*google_secret_manager_secret_version\.destination_identity_key\.version',
        secondary_services,
        flags=re.DOTALL,
    ):
        errors.append("Terraform secondary Cloud Run services do not consume the destination identity secret")
    require_text(
        ROOT / "deploy/self-host/docker-compose.yml",
        ['http://127.0.0.1:8080/v1/ready'],
        errors,
    )
    require_text(
        ROOT / "deploy/helm/piqae/templates/deployments.yaml",
        [
            "maxUnavailable: 0",
            "readinessProbe:",
            "path: /v1/ready",
        ],
        errors,
    )
    require_text(
        ROOT / "deploy/helm/piqae/templates/migration-job.yaml",
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
            "./deploy/production-check.sh managed-ha",
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
            "PIQAE_PRIMARY_API_SERVICE",
            "PIQAE_PRIMARY_SYNC_SERVICE",
            "PIQAE_PRIMARY_WORKER_SERVICE",
            "PIQAE_SECONDARY_API_SERVICE",
            "PIQAE_SECONDARY_SYNC_SERVICE",
            "PIQAE_SECONDARY_WORKER_SERVICE",
            'worker_indexes=(2 5)',
        ],
        errors,
    )

    check_migrations(errors)
    return errors


def require_https(name: str, value: str, errors: list[str]) -> None:
    if not value.startswith("https://"):
        errors.append(f"{name} must use HTTPS")


def release_errors(
    railway_env: Path | None,
    evidence_dir: Path | None,
    *,
    target: str = "railway",
    tfvars: Path | None = None,
) -> list[str]:
    errors = structural_errors()
    if railway_env is None or not railway_env.is_file():
        errors.append("release preflight requires --railway-env")
        values: dict[str, str] = {}
    else:
        try:
            values = parse_env(railway_env)
        except PreflightError as error:
            errors.append(str(error))
            values = {}

    for key in REQUIRED_RAILWAY_WEB_KEYS:
        if not values.get(key):
            errors.append(f"production Railway web environment is missing {key}")
    if values.get("PIQAE_AUTH_MODE") != "workos":
        errors.append("production PIQAE_AUTH_MODE must be workos")
    if values.get("PUBLIC_PIQAE_DASHBOARD_MODE") != "live":
        errors.append("production dashboard mode must be live")
    if values.get("PIQAE_COOKIE_SECURE") != "true":
        errors.append("production cookies must be explicitly secure")
    expected_redirect = values.get("ORIGIN", "").rstrip("/") + "/auth/callback"
    if values.get("WORKOS_REDIRECT_URI") != expected_redirect:
        errors.append("WORKOS_REDIRECT_URI must be the canonical ORIGIN callback")
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
    for key in (
        "PUBLIC_PIQAE_API_URL",
        "PUBLIC_SITE_URL",
        "ORIGIN",
        "WORKOS_REDIRECT_URI",
        "PIQAE_RELEASES_S3_ENDPOINT",
    ):
        if values.get(key):
            require_https(key, values[key], errors)
    if values.get("PIQAE_RELEASES_S3_VIRTUAL_HOSTED_STYLE") != "false":
        errors.append("Railway release bucket must use path-style S3 addressing")

    if target == "managed-ha":
        if tfvars is None or not tfvars.is_file():
            errors.append("managed-ha preflight requires --tfvars")
        else:
            check_managed_ha_tfvars(tfvars, errors)
    elif target != "railway":
        errors.append(f"unsupported release target: {target}")

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
    if "PIQAE_SERVICE_ROLE" not in control_plane:
        errors.append("api/sync/worker service-role isolation is not implemented")
    if "PIQAE_RUN_MIGRATIONS_ON_STARTUP" not in control_plane or "migrate_only" not in control_plane:
        errors.append("server still runs database migrations during replica startup")
    if "STRIPE_WEBHOOK_SECRET" not in control_plane:
        errors.append("control plane does not consume STRIPE_WEBHOOK_SECRET")
    for key in ("STRIPE_SECRET_KEY", "STRIPE_METER_EVENT_NAME"):
        if key not in control_plane:
            errors.append(f"billing meter worker does not consume {key}")

    contract = json.loads((ROOT / "release/production-readiness.json").read_text(encoding="utf-8"))
    evidence_gates = list(contract["external_evidence"])
    if target == "managed-ha":
        evidence_gates.extend(contract["managed_ha_external_evidence"])
    if evidence_dir is None or not evidence_dir.is_dir():
        errors.append("release preflight requires --evidence-dir with external records")
    else:
        for gate in evidence_gates:
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
            if gate["id"] == "railway_production_runtime":
                check_railway_runtime_record(record, errors)
    return errors


def check_railway_runtime_record(record: dict[str, object], errors: list[str]) -> None:
    railway = record.get("railway")
    if not isinstance(railway, dict):
        errors.append("Railway runtime evidence is missing railway details")
        return
    for key in ("project_id", "environment_id"):
        if not railway.get(key):
            errors.append(f"Railway runtime evidence is missing {key}")
    services = railway.get("services")
    if not isinstance(services, dict):
        errors.append("Railway runtime evidence is missing services")
    else:
        for name, should_be_public in (("web", True), ("api", True), ("worker", False)):
            service = services.get(name)
            if not isinstance(service, dict):
                errors.append(f"Railway runtime evidence is missing {name} service")
                continue
            if not service.get("deployment_id"):
                errors.append(f"Railway {name} evidence is missing deployment_id")
            if service.get("status") != "SUCCESS":
                errors.append(f"Railway {name} deployment is not SUCCESS")
            if service.get("public_domain") is not should_be_public:
                expected = "public" if should_be_public else "private"
                errors.append(f"Railway {name} service must be {expected}")
    document_bucket = railway.get("document_bucket")
    release_bucket = railway.get("release_bucket")
    if not document_bucket or not release_bucket:
        errors.append("Railway runtime evidence is missing bucket identities")
    elif document_bucket == release_bucket:
        errors.append("Railway release and document buckets must be distinct")


def check_managed_ha_tfvars(tfvars: Path, errors: list[str]) -> None:
    try:
        text = tfvars.read_text(encoding="utf-8")
    except OSError as error:
        errors.append(f"cannot read managed-ha tfvars: {error}")
        return
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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mode", choices=("structural", "release"), default="structural")
    parser.add_argument("--target", choices=("railway", "managed-ha"), default="railway")
    parser.add_argument("--railway-env", type=Path)
    parser.add_argument("--tfvars", type=Path)
    parser.add_argument("--evidence-dir", type=Path)
    args = parser.parse_args()
    errors = (
        structural_errors()
        if args.mode == "structural"
        else release_errors(
            args.railway_env,
            args.evidence_dir,
            target=args.target,
            tfvars=args.tfvars,
        )
    )
    if errors:
        for error in errors:
            print(f"FAIL: {error}", file=sys.stderr)
        return 1
    print(f"production {args.mode} preflight passed for {args.target}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
