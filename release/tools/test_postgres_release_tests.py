#!/usr/bin/env python3
"""Tests for the fail-closed PostgreSQL release evidence wrapper."""

from __future__ import annotations

import unittest

import check_postgres_release_tests as evidence


class PostgresReleaseEvidenceTest(unittest.TestCase):
    def test_all_release_database_boundaries_are_required(self) -> None:
        self.assertEqual(
            [gate.identifier for gate in evidence.GATES],
            [
                "automatic_wake_outbox",
                "automatic_wake_outbox_n_minus_one_upgrade",
                "destination_topology_fencing_fifo",
                "destination_topology_n_minus_one_upgrade",
                "routing_recovery",
                "platform_service_accounts",
                "platform_service_account_http",
                "platform_accounts",
                "migrations",
                "workos_identity",
                "billing",
            ],
        )

    def test_destination_topology_runs_both_named_release_boundaries(self) -> None:
        topology = evidence.GATES[2:4]
        self.assertEqual(
            [gate.expected_test for gate in topology],
            [
                "postgres_topology_is_tenant_isolated_and_fences_delivery",
                "migration_40_upgrades_39_and_backfills_without_inferring_route_merges",
            ],
        )
        for gate in topology:
            self.assertEqual(
                gate.command[:6],
                (
                    "cargo",
                    "test",
                    "-p",
                    "piqae-storage-postgres",
                    "--test",
                    "destination_topology",
                ),
            )
            self.assertIn(gate.expected_test, gate.command)
            self.assertEqual(gate.command[-3:], ("--locked", "--", "--nocapture"))

    def test_missing_database_url_fails_closed(self) -> None:
        with self.assertRaisesRegex(
            evidence.PostgresEvidenceError, "requires PIQAE_TEST_DATABASE_URL"
        ):
            evidence.require_database_url({})

    def test_non_postgres_database_url_is_rejected(self) -> None:
        with self.assertRaisesRegex(
            evidence.PostgresEvidenceError, "PostgreSQL connection URL"
        ):
            evidence.require_database_url(
                {"PIQAE_TEST_DATABASE_URL": "https://example.invalid/database"}
            )

    def test_skipped_test_is_not_release_evidence(self) -> None:
        gate = evidence.GATES[0]
        with self.assertRaisesRegex(evidence.PostgresEvidenceError, "skipped"):
            evidence.validate_output(
                gate,
                0,
                (
                    "running 1 test\n"
                    "skipped: set PIQAE_TEST_DATABASE_URL\n"
                    f"test {gate.expected_test} ... ok\n"
                    "test result: ok. 1 passed; 0 failed\n"
                ),
            )

    def test_zero_matching_tests_is_not_release_evidence(self) -> None:
        gate = evidence.GATES[1]
        with self.assertRaisesRegex(
            evidence.PostgresEvidenceError, "skipped or ran zero tests"
        ):
            evidence.validate_output(
                gate,
                0,
                "running 0 tests\ntest result: ok. 0 passed; 0 failed\n",
            )

    def test_wrong_test_name_is_rejected(self) -> None:
        gate = evidence.GATES[1]
        with self.assertRaisesRegex(evidence.PostgresEvidenceError, "did not pass"):
            evidence.validate_output(
                gate,
                0,
                "test a_unit_test_with_no_database ... ok\n"
                "test result: ok. 1 passed; 0 failed\n",
            )

    def test_exact_non_skipped_test_passes(self) -> None:
        gate = evidence.GATES[0]
        evidence.validate_output(
            gate,
            0,
            f"test {gate.expected_test} ... ok\n"
            "test result: ok. 1 passed; 0 failed\n",
        )

    def test_ten_passing_tests_are_not_mistaken_for_zero(self) -> None:
        gate = evidence.GATES[0]
        evidence.validate_output(
            gate,
            0,
            f"test {gate.expected_test} ... ok\n"
            "test result: ok. 10 passed; 0 failed\n",
        )

    def test_nonzero_command_fails(self) -> None:
        with self.assertRaisesRegex(evidence.PostgresEvidenceError, "exit code 101"):
            evidence.validate_output(evidence.GATES[0], 101, "compile failed")


if __name__ == "__main__":
    unittest.main()
