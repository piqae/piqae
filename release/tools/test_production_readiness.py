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
        errors = readiness.release_errors(None, None)
        self.assertIn("release preflight requires --railway-env", errors)
        self.assertNotIn("managed-ha preflight requires --tfvars", errors)
        self.assertIn(
            "release preflight requires --evidence-dir with external records",
            errors,
        )

    def test_managed_ha_adds_tfvars_and_dr_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory)
            railway_errors = readiness.release_errors(None, evidence)
            managed_errors = readiness.release_errors(
                None,
                evidence,
                target="managed-ha",
            )
        self.assertNotIn("managed-ha preflight requires --tfvars", railway_errors)
        self.assertIn("managed-ha preflight requires --tfvars", managed_errors)
        self.assertNotIn(
            "missing external evidence: regional_dr_rehearsal",
            railway_errors,
        )
        self.assertIn(
            "missing external evidence: regional_dr_rehearsal",
            managed_errors,
        )

    def test_public_soak_is_not_a_private_beta_gate(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            errors = readiness.release_errors(None, Path(directory))
        self.assertNotIn(
            "missing external evidence: production_soak_30_days",
            errors,
        )

    def test_railway_runtime_evidence_requires_isolated_successful_services(self) -> None:
        record = {
            "railway": {
                "project_id": "project",
                "environment_id": "production",
                "services": {
                    "web": {
                        "deployment_id": "web-deployment",
                        "status": "SUCCESS",
                        "public_domain": True,
                    },
                    "api": {
                        "deployment_id": "api-deployment",
                        "status": "SUCCESS",
                        "public_domain": True,
                    },
                    "worker": {
                        "deployment_id": "worker-deployment",
                        "status": "SUCCESS",
                        "public_domain": False,
                    },
                },
                "document_bucket": "spool-documents",
                "release_bucket": "piqae-releases",
            }
        }
        errors: list[str] = []
        readiness.check_railway_runtime_record(record, errors)
        self.assertEqual(errors, [])

        record["railway"]["release_bucket"] = "spool-documents"
        readiness.check_railway_runtime_record(record, errors)
        self.assertIn("Railway release and document buckets must be distinct", errors)


if __name__ == "__main__":
    unittest.main()
