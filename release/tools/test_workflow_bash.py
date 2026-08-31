from pathlib import Path
import tempfile
import unittest

from check_workflow_bash import check, explicit_bash_blocks


WORKFLOWS = Path(__file__).resolve().parents[2] / ".github" / "workflows"


def workflow(directory: Path, script: str) -> Path:
    path = directory / "workflow.yml"
    body = "\n".join(f"          {line}" for line in script.splitlines())
    path.write_text(
        "jobs:\n"
        "  verify:\n"
        "    steps:\n"
        "      - name: Verify\n"
        "        shell: bash\n"
        "        run: |\n"
        f"{body}\n",
        encoding="utf-8",
    )
    return path


class WorkflowBashTests(unittest.TestCase):
    def test_every_explicit_bash_block_in_checked_in_workflows_parses(self) -> None:
        workflows = sorted(WORKFLOWS.glob("*.yml"))
        self.assertTrue(workflows, "no workflows were discovered")
        failures = [failure for path in workflows for failure in check(path)]
        self.assertEqual(failures, [])

    def test_multiline_boolean_regression_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = workflow(
                Path(directory),
                "if [[ \"$PUBLISH\" == true ]] && (\n"
                "  [[ \"$FIRST\" != success ]]\n"
                "  || [[ \"$SECOND\" != success ]]\n"
                "); then\n"
                "  exit 1\n"
                "fi",
            )
            self.assertEqual(len(explicit_bash_blocks(path)), 1)
            self.assertEqual(len(check(path)), 1)

    def test_github_expressions_are_replaced_before_parse_checking(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = workflow(
                Path(directory),
                'value="${{ needs.prepare.outputs.publish }}"\n'
                '[[ "$value" == true ]] || exit 1',
            )
            self.assertEqual(check(path), [])


if __name__ == "__main__":
    unittest.main()
