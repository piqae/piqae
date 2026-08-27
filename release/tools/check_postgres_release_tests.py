#!/usr/bin/env python3
"""Run release-only PostgreSQL evidence tests without permitting silent skips."""

from __future__ import annotations

import os
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence


class PostgresEvidenceError(RuntimeError):
    """Required PostgreSQL release evidence is absent or failed."""


@dataclass(frozen=True)
class Gate:
    identifier: str
    command: tuple[str, ...]
    expected_test: str


GATES = (
    Gate(
        identifier="automatic_wake_outbox",
        command=(
            "cargo",
            "test",
            "-p",
            "piqae-storage-postgres",
            "--test",
            "automatic_wake_outbox",
            "--locked",
            "--",
            "--nocapture",
        ),
        expected_test="postgres_wake_outbox_is_idempotent_content_free_and_at_least_once",
    ),
    Gate(
        identifier="automatic_wake_outbox_n_minus_one_upgrade",
        command=(
            "cargo",
            "test",
            "-p",
            "piqae-storage-postgres",
            "--test",
            "migrations",
            "automatic_wake_outbox_upgrades_41_and_is_tenant_isolated",
            "--locked",
            "--",
            "--nocapture",
        ),
        expected_test="automatic_wake_outbox_upgrades_41_and_is_tenant_isolated",
    ),
    Gate(
        identifier="destination_topology_fencing_fifo",
        command=(
            "cargo",
            "test",
            "-p",
            "piqae-storage-postgres",
            "--test",
            "destination_topology",
            "postgres_topology_is_tenant_isolated_and_fences_delivery",
            "--locked",
            "--",
            "--nocapture",
        ),
        expected_test="postgres_topology_is_tenant_isolated_and_fences_delivery",
    ),
    Gate(
        identifier="destination_topology_n_minus_one_upgrade",
        command=(
            "cargo",
            "test",
            "-p",
            "piqae-storage-postgres",
            "--test",
            "destination_topology",
            "migration_40_upgrades_39_and_backfills_without_inferring_route_merges",
            "--locked",
            "--",
            "--nocapture",
        ),
        expected_test="migration_40_upgrades_39_and_backfills_without_inferring_route_merges",
    ),
    Gate(
        identifier="routing_recovery",
        command=(
            "cargo",
            "test",
            "-p",
            "piqae-storage-postgres",
            "--test",
            "routing_recovery",
            "--locked",
            "--",
            "--nocapture",
        ),
        expected_test="postgres_reroute_is_atomic_and_fenced_by_lease_and_acceptance",
    ),
    Gate(
        identifier="platform_service_accounts",
        command=(
            "cargo",
            "test",
            "-p",
            "piqae-storage-postgres",
            "--test",
            "platform_service_accounts",
            "--locked",
            "--",
            "--nocapture",
        ),
        expected_test="postgres_platform_grants_are_exact_scoped_and_revocable",
    ),
    Gate(
        identifier="platform_service_account_http",
        command=(
            "cargo",
            "test",
            "-p",
            "piqae-control-plane",
            "--test",
            "platform_service_accounts_postgres",
            "--locked",
            "--",
            "--nocapture",
        ),
        expected_test="postgres_platform_http_auth_is_tenant_scoped_audited_and_revocable",
    ),
    Gate(
        identifier="platform_accounts",
        command=(
            "cargo",
            "test",
            "-p",
            "piqae-control-plane",
            "--test",
            "platform_accounts",
            "--locked",
            "--",
            "--nocapture",
        ),
        expected_test="postgres_http_platform_accounts_are_owned_idempotent_and_archive_safely",
    ),
    # Schema upgrades, the WorkOS identity projection, and cloud billing each
    # only exist against a real database. Without them here nothing ran them:
    # they answer "skipped" and report a pass to any ordinary test command.
    Gate(
        identifier="migrations",
        command=(
            "cargo",
            "test",
            "-p",
            "piqae-storage-postgres",
            "--test",
            "migrations",
            "--locked",
            "--",
            "--nocapture",
        ),
        expected_test="documents_migrate_and_enforce_tenant_scoped_references",
    ),
    Gate(
        identifier="workos_identity",
        command=(
            "cargo",
            "test",
            "-p",
            "piqae-storage-postgres",
            "--test",
            "workos_identity",
            "--locked",
            "--",
            "--nocapture",
        ),
        expected_test="workos_projection_is_idempotent_ordered_and_organization_scoped",
    ),
    Gate(
        identifier="billing",
        command=(
            "cargo",
            "test",
            "-p",
            "piqae-control-plane",
            "--test",
            "billing_postgres",
            "--locked",
            "--",
            "--nocapture",
        ),
        expected_test="cloud_billing_is_tenant_scoped_idempotent_and_stripe_projected",
    ),
)


def require_database_url(environment: dict[str, str]) -> str:
    value = environment.get("PIQAE_TEST_DATABASE_URL", "").strip()
    if not value:
        raise PostgresEvidenceError(
            "release PostgreSQL evidence requires PIQAE_TEST_DATABASE_URL; "
            "normal contributor tests may omit it"
        )
    if not re.match(r"^postgres(?:ql)?://", value):
        raise PostgresEvidenceError(
            "PIQAE_TEST_DATABASE_URL must use a PostgreSQL connection URL"
        )
    return value


def validate_output(gate: Gate, returncode: int, output: str) -> None:
    if returncode != 0:
        raise PostgresEvidenceError(
            f"{gate.identifier}: test command failed with exit code {returncode}"
        )
    lowered = output.lower()
    if "skipped:" in lowered or re.search(
        r"(?m)^test result: ok\. 0 passed;", lowered
    ):
        raise PostgresEvidenceError(
            f"{gate.identifier}: PostgreSQL evidence was skipped or ran zero tests"
        )
    expected = re.compile(
        rf"(?m)^test {re.escape(gate.expected_test)} \.\.\. ok$"
    )
    if not expected.search(output):
        raise PostgresEvidenceError(
            f"{gate.identifier}: required test {gate.expected_test!r} did not pass"
        )


def run_gate(root: Path, gate: Gate, database_url: str) -> None:
    completed = subprocess.run(
        gate.command,
        cwd=root,
        env=os.environ.copy(),
        capture_output=True,
        text=True,
        check=False,
    )
    output = completed.stdout + completed.stderr
    try:
        validate_output(gate, completed.returncode, output)
    except PostgresEvidenceError:
        redacted = output.replace(database_url, "<redacted database URL>")
        if redacted.strip():
            print(redacted[-4_000:], file=sys.stderr)
        raise
    print(f"postgres release evidence passed: {gate.identifier}")


def check(root: Path, environment: dict[str, str]) -> None:
    database_url = require_database_url(environment)
    for gate in GATES:
        run_gate(root, gate, database_url)


def main(argv: Sequence[str] | None = None) -> int:
    if argv is None:
        argv = sys.argv[1:]
    if argv:
        print("check_postgres_release_tests.py takes no arguments", file=sys.stderr)
        return 2
    root = Path(__file__).resolve().parents[2]
    try:
        check(root, dict(os.environ))
    except PostgresEvidenceError as error:
        print(f"PostgreSQL release evidence failed: {error}", file=sys.stderr)
        return 1
    print("all required PostgreSQL release evidence passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
