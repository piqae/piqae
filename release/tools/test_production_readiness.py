import importlib.util
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("check_production_readiness.py")
SPEC = importlib.util.spec_from_file_location("production_readiness", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
readiness = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(readiness)


class ProductionReadinessTests(unittest.TestCase):
    def test_repository_contract_is_structurally_valid(self) -> None:
        self.assertEqual(readiness.structural_errors(), [])

    def test_env_parser_rejects_duplicate_keys(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "production.env"
            path.write_text("SPOOL_AUTH_MODE=workos\nSPOOL_AUTH_MODE=demo\n", encoding="utf-8")
            with self.assertRaisesRegex(readiness.PreflightError, "duplicate key"):
                readiness.parse_env(path)

    def test_release_mode_fails_closed_without_operator_inputs(self) -> None:
        errors = readiness.release_errors(None, None, None)
        self.assertIn("release preflight requires --vercel-env", errors)
        self.assertIn("release preflight requires --tfvars", errors)
        self.assertIn(
            "release preflight requires --evidence-dir with external records",
            errors,
        )


if __name__ == "__main__":
    unittest.main()
