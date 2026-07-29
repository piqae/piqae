from __future__ import annotations

import copy
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import audit_platform_service_account_coverage as audit
from check_platform_service_account_policy import PolicyError, load_json


class PlatformServiceAccountCoverageTests(unittest.TestCase):
    def repository_coverage(self) -> dict[str, object]:
        release = Path(__file__).resolve().parent.parent
        return load_json(release / "platform-service-account-coverage.json")

    @mock.patch.object(audit, "commit_is_ancestor", return_value=True)
    def test_repository_coverage_is_truthful(self, _ancestor: mock.Mock) -> None:
        audit.validate_coverage(
            self.repository_coverage(),
            root=audit.repository_root(),
            support_matrix=audit.repository_root() / "release" / "support-matrix.yaml",
        )

    @mock.patch.object(audit, "commit_is_ancestor", return_value=True)
    def test_partial_scenario_cannot_be_marked_passed_with_gaps(
        self, _ancestor: mock.Mock
    ) -> None:
        coverage = copy.deepcopy(self.repository_coverage())
        scenario = next(
            item
            for item in coverage["scenarios"]
            if item["id"] == "tenant_isolation"
        )
        scenario["status"] = "passed"
        with self.assertRaisesRegex(PolicyError, "requires tests and no missing"):
            audit.validate_coverage(
                coverage,
                root=audit.repository_root(),
                support_matrix=audit.repository_root()
                / "release"
                / "support-matrix.yaml",
            )

    @mock.patch.object(audit, "commit_is_ancestor", return_value=True)
    def test_preview_tier_rejects_any_partial_scenario(
        self, _ancestor: mock.Mock
    ) -> None:
        coverage = self.repository_coverage()
        with tempfile.TemporaryDirectory() as temporary:
            matrix = Path(temporary) / "support.yaml"
            matrix.write_text(
                "features:\n"
                "  platform_service_accounts:\n"
                "    tier: preview\n"
                "    reason: candidate\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(PolicyError, "incomplete code coverage"):
                audit.validate_coverage(
                    coverage, root=audit.repository_root(), support_matrix=matrix
                )

    @mock.patch.object(audit, "commit_is_ancestor", return_value=True)
    def test_stale_source_anchor_fails(self, _ancestor: mock.Mock) -> None:
        coverage = copy.deepcopy(self.repository_coverage())
        coverage["scenarios"][0]["code_references"][0]["anchors"] = [
            "not-a-real-authorization-anchor"
        ]
        with self.assertRaisesRegex(PolicyError, "source anchor is missing"):
            audit.validate_coverage(
                coverage,
                root=audit.repository_root(),
                support_matrix=audit.repository_root()
                / "release"
                / "support-matrix.yaml",
            )


if __name__ == "__main__":
    unittest.main()
