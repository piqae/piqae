from __future__ import annotations

import unittest

from ci_changed_paths import GROUPS, classify


class CiChangedPathsTests(unittest.TestCase):
    def test_documentation_only_change_selects_no_expensive_jobs(self) -> None:
        self.assertFalse(any(classify(["docs/nodes/updates.md"]).values()))

    def test_web_change_is_scoped_to_web(self) -> None:
        selected = classify(["apps/web/src/routes/+page.svelte"])
        self.assertTrue(selected["web"])
        self.assertFalse(selected["rust"])
        self.assertFalse(selected["macos"])
        self.assertFalse(selected["windows"])

    def test_shared_rust_change_checks_every_native_platform(self) -> None:
        selected = classify(["crates/protocol/src/lib.rs"])
        self.assertTrue(selected["rust"])
        self.assertTrue(selected["macos"])
        self.assertTrue(selected["windows"])

    def test_platform_packaging_change_is_platform_specific(self) -> None:
        macos = classify(["packaging/macos/install-user.sh"])
        windows = classify(["packaging/windows/Piqae.iss"])
        self.assertTrue(macos["macos"])
        self.assertFalse(macos["windows"])
        self.assertTrue(windows["windows"])
        self.assertFalse(windows["macos"])

    def test_contract_change_checks_contract_and_javascript(self) -> None:
        selected = classify(["contracts/openapi/piqae-v1.yaml"])
        self.assertTrue(selected["openapi"])
        self.assertTrue(selected["web"])

    def test_workflow_change_selects_every_group(self) -> None:
        self.assertEqual(
            classify([".github/workflows/ci.yml"]),
            {group: True for group in GROUPS},
        )

    def test_all_override_selects_every_group(self) -> None:
        self.assertEqual(classify([], run_all=True), {group: True for group in GROUPS})


if __name__ == "__main__":
    unittest.main()
