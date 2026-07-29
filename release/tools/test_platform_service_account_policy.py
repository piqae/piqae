from __future__ import annotations

import datetime as dt
import json
import tempfile
import unittest
from pathlib import Path

import check_platform_service_account_policy as policy


class PlatformServiceAccountPolicyTests(unittest.TestCase):
    def repository_policy(self) -> tuple[dict[str, object], Path]:
        release = Path(__file__).resolve().parent.parent
        return (
            policy.load_json(release / "platform-service-account-gates.json"),
            release / "support-matrix.yaml",
        )

    def test_repository_policy_is_complete_and_truthful(self) -> None:
        configured, matrix = self.repository_policy()
        policy.validate_policy(configured, matrix)
        self.assertEqual(configured["current_tier"], "disabled")

    def test_missing_workspace_selection_or_redaction_scenario_fails(self) -> None:
        configured, _matrix = self.repository_policy()
        for missing in {"ordinary_key_workspace_selection", "secret_redaction"}:
            broken = dict(configured)
            broken["required_scenarios"] = [
                scenario
                for scenario in configured["required_scenarios"]
                if scenario["id"] != missing
            ]
            with tempfile.TemporaryDirectory() as temporary:
                matrix = Path(temporary) / "support.yaml"
                matrix.write_text(
                    "version: 1\nfeatures:\n"
                    "  platform_service_accounts:\n"
                    "    tier: disabled\n"
                    "    reason: evidence pending\n",
                    encoding="utf-8",
                )
                with self.assertRaisesRegex(policy.PolicyError, "exactly"):
                    policy.validate_policy(broken, matrix)

    def test_preview_fails_closed_without_evidence(self) -> None:
        configured, _matrix = self.repository_policy()
        configured["current_tier"] = "preview"
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            configured_path = root / "policy.json"
            configured_path.write_text(json.dumps(configured), encoding="utf-8")
            matrix = root / "support.yaml"
            matrix.write_text(
                "version: 1\nfeatures:\n"
                "  platform_service_accounts:\n"
                "    tier: preview\n"
                "    reason: evidence required\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(policy.PolicyError, "requires evidence"):
                policy.check(configured_path, matrix, root / "missing.json")

    def test_evidence_rejects_failed_scenario_and_unsafe_reference(self) -> None:
        configured, _matrix = self.repository_policy()
        configured["current_tier"] = "preview"
        results = [
            {
                "id": identifier,
                "status": "passed",
                "command": "cargo test -p spool-control-plane platform_credentials",
                "reference": f"reports/{identifier}.json",
                "synthetic_secrets_only": True,
            }
            for identifier in sorted(policy.REQUIRED_SCENARIOS)
        ]
        evidence = {
            "schema_version": 1,
            "feature": "platform_service_accounts",
            "release": "v1.0.0-rc.1",
            "commit": "1" * 40,
            "recorded_at": "2026-01-01T00:00:00Z",
            "results": results,
        }
        results[0]["status"] = "failed"
        with self.assertRaisesRegex(policy.PolicyError, "has not passed"):
            policy.validate_evidence(
                evidence,
                configured,
                now=dt.datetime(2026, 2, 1, tzinfo=dt.timezone.utc),
            )
        results[0]["status"] = "passed"
        results[0]["reference"] = "../secret.log"
        with self.assertRaisesRegex(policy.PolicyError, "unsafe evidence reference"):
            policy.validate_evidence(
                evidence,
                configured,
                now=dt.datetime(2026, 2, 1, tzinfo=dt.timezone.utc),
            )


if __name__ == "__main__":
    unittest.main()
