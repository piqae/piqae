from pathlib import Path
import subprocess
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from release_completion import AggregateResults, certification_errors


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "release/tools/release_completion.py"


def results(**overrides: str) -> AggregateResults:
    values = {
        "core": "success",
        "macos": "success",
        "windows": "skipped",
        "apple_sdk": "success",
        "windows_sdk": "success",
        "linux": "success",
        "containers": "success",
        "macos_promotion": "success",
        "macos_prerelease": "success",
        "container_promotion": "success",
    }
    values.update(overrides)
    return AggregateResults(**values)


class ReleaseCompletionTest(unittest.TestCase):
    def test_published_all_scope_accepts_support_disabled_windows(self) -> None:
        self.assertEqual(
            certification_errors(
                results(), windows_enabled=False, publish=True
            ),
            [],
        )

    def test_published_all_scope_requires_enabled_windows(self) -> None:
        errors = certification_errors(
            results(), windows_enabled=True, publish=True
        )
        self.assertEqual(
            errors,
            ["windows must be success for aggregate certification; got skipped"],
        )
        self.assertEqual(
            certification_errors(
                results(windows="success"), windows_enabled=True, publish=True
            ),
            [],
        )

    def test_sibling_failure_fails_only_aggregate_policy(self) -> None:
        errors = certification_errors(
            results(windows_sdk="failure"), windows_enabled=False, publish=True
        )
        self.assertEqual(
            errors,
            ["windows-sdk must be success for aggregate certification; got failure"],
        )

    def test_private_all_candidate_requires_promotions_to_be_skipped(self) -> None:
        private = results(
            macos_promotion="skipped",
            macos_prerelease="skipped",
            container_promotion="skipped",
        )
        self.assertEqual(
            certification_errors(private, windows_enabled=False, publish=False),
            [],
        )

    def test_cli_rejects_unknown_or_missing_job_results(self) -> None:
        completed = subprocess.run(
            [sys.executable, str(SCRIPT), "--windows-enabled", "false"],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertNotEqual(completed.returncode, 0)
        self.assertNotIn("passed aggregate certification", completed.stdout)


if __name__ == "__main__":
    unittest.main()
