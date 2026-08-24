from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from check_workflow_runners import check

WORKFLOWS = Path(__file__).resolve().parents[2] / ".github" / "workflows"


def written(body: str) -> Path:
    directory = Path(tempfile.mkdtemp())
    path = directory / "workflow.yml"
    path.write_text(body, encoding="utf-8")
    return path


class WorkflowRunnerTests(unittest.TestCase):
    def test_every_checked_in_workflow_keeps_the_runner_indirection(self) -> None:
        workflows = sorted(WORKFLOWS.glob("*.yml"))
        self.assertTrue(workflows, "no workflows were discovered")
        for workflow in workflows:
            with self.subTest(workflow=workflow.name):
                self.assertEqual(check(workflow), [])

    def test_repository_variable_indirection_is_accepted(self) -> None:
        path = written(
            "jobs:\n"
            "  build:\n"
            "    runs-on: ${{ vars.PIQAE_CI_LINUX_RUNNER || 'ubuntu-latest' }}\n"
        )
        self.assertEqual(check(path), [])

    def test_matrix_conditional_indirection_is_accepted(self) -> None:
        path = written(
            "jobs:\n"
            "  build:\n"
            "    runs-on: ${{ matrix.kind == 'aarch64'"
            " && (vars.PIQAE_RELEASE_LINUX_ARM_RUNNER || 'ubuntu-24.04-arm')"
            " || (vars.PIQAE_RELEASE_LINUX_RUNNER || 'ubuntu-latest') }}\n"
        )
        self.assertEqual(check(path), [])

    def test_protected_self_hosted_pool_is_accepted(self) -> None:
        path = written(
            "jobs:\n"
            "  promote:\n"
            "    runs-on: [self-hosted, piqae-production]\n"
        )
        self.assertEqual(check(path), [])

    def test_hardcoded_runner_is_rejected(self) -> None:
        for label in ("ubuntu-latest", "windows-latest", "macos-15"):
            with self.subTest(label=label):
                path = written(f"jobs:\n  build:\n    runs-on: {label}\n")
                failures = check(path)
                self.assertEqual(len(failures), 1)
                self.assertIn("hardcoded", failures[0])

    def test_unrelated_repository_variable_is_rejected(self) -> None:
        path = written(
            "jobs:\n  build:\n    runs-on: ${{ vars.PIQAE_CI_LINUX_IMAGE }}\n"
        )
        self.assertEqual(len(check(path)), 1)

    def test_block_scalar_runner_is_rejected(self) -> None:
        path = written(
            "jobs:\n"
            "  build:\n"
            "    runs-on:\n"
            "      - self-hosted\n"
        )
        self.assertEqual(len(check(path)), 1)


if __name__ == "__main__":
    unittest.main()
