from __future__ import annotations

import unittest

from ci_changed_paths import GROUPS, classify


class CiChangedPathsTests(unittest.TestCase):
    def test_documentation_only_selects_no_expensive_jobs(self) -> None:
        self.assertFalse(any(classify(["docs/nodes/updates.md"]).values()))

    def test_web_and_sdk_are_independent(self) -> None:
        web = classify(["apps/web/src/routes/+page.svelte"])
        sdk = classify(["sdk/typescript/src/index.ts"])
        self.assertTrue(web["web"])
        self.assertFalse(web["sdk"] or web["macos_rust"] or web["windows_rust"])
        self.assertTrue(sdk["sdk"])
        self.assertFalse(sdk["web"] or sdk["macos_rust"] or sdk["windows_rust"])

    def test_shared_agent_change_checks_both_native_platforms(self) -> None:
        selected = classify(["crates/protocol/src/lib.rs"])
        self.assertTrue(selected["rust_shared"])
        self.assertTrue(selected["macos_rust"])
        self.assertTrue(selected["windows_rust"])

    def test_server_migration_does_not_build_native_apps(self) -> None:
        selected = classify(["migrations/0024_example.sql"])
        self.assertTrue(selected["rust_server"])
        self.assertFalse(selected["macos_rust"] or selected["windows_rust"])

    def test_platform_packaging_is_platform_specific(self) -> None:
        macos = classify(["packaging/macos/install-user.sh"])
        windows = classify(["packaging/windows/Piqae.iss"])
        self.assertTrue(macos["macos_packaging"])
        self.assertFalse(macos["windows_installer"])
        self.assertTrue(windows["windows_installer"])
        self.assertFalse(windows["macos_packaging"])

    def test_release_workflow_change_does_not_select_every_platform(self) -> None:
        selected = classify([".github/workflows/macos-promotion.yml"])
        self.assertTrue(selected["release_tooling"] and selected["macos_packaging"])
        self.assertFalse(selected["windows_installer"] or selected["sdk"] or selected["web"])

    def test_release_orchestration_selects_native_packaging_only(self) -> None:
        selected = classify([".github/workflows/release.yml"])
        self.assertTrue(selected["release_tooling"])
        self.assertTrue(selected["macos_packaging"] and selected["windows_installer"])
        self.assertFalse(selected["sdk"] or selected["web"] or selected["terraform"])

    def test_ci_classifier_change_exercises_all_scopes(self) -> None:
        self.assertEqual(
            classify(["release/tools/ci_changed_paths.py"]),
            {group: True for group in GROUPS},
        )

    def test_contract_change_checks_contract_and_javascript(self) -> None:
        selected = classify(["contracts/openapi/piqae-v1.yaml"])
        self.assertTrue(selected["openapi"] and selected["web"] and selected["sdk"])

    def test_root_lockfile_selects_every_rust_platform(self) -> None:
        selected = classify(["Cargo.lock"])
        self.assertTrue(selected["rust_server"] and selected["rust_shared"])
        self.assertTrue(selected["macos_rust"] and selected["windows_rust"])
        self.assertTrue(selected["dependency_policy"])

    def test_all_override_selects_every_group(self) -> None:
        self.assertEqual(classify([], run_all=True), {group: True for group in GROUPS})


if __name__ == "__main__":
    unittest.main()
