from __future__ import annotations

import unittest
from pathlib import Path

from ci_changed_paths import GROUPS, classify


class CiChangedPathsTests(unittest.TestCase):
    def test_documentation_only_selects_no_expensive_jobs(self) -> None:
        self.assertFalse(any(classify(["docs/nodes/updates.md"]).values()))

    def test_javascript_packages_are_independently_scoped(self) -> None:
        web = classify(["apps/web/src/routes/+page.svelte"])
        sdk = classify(["sdk/typescript/src/index.ts"])
        mcp = classify(["apps/mcp/src/server.ts"])
        shopify = classify(["apps/shopify/app/routes/app.tsx"])
        self.assertTrue(web["web"])
        self.assertFalse(web["sdk"] or web["mcp"] or web["shopify"])
        self.assertTrue(sdk["sdk"])
        self.assertTrue(sdk["mcp"] and sdk["shopify"])
        self.assertFalse(sdk["web"])
        self.assertTrue(mcp["mcp"])
        self.assertFalse(mcp["web"] or mcp["sdk"] or mcp["shopify"])
        self.assertTrue(shopify["shopify"])
        self.assertFalse(shopify["web"] or shopify["sdk"] or shopify["mcp"])

    def test_root_javascript_files_select_every_javascript_package(self) -> None:
        for path in ("package.json", "pnpm-lock.yaml", "pnpm-workspace.yaml"):
            with self.subTest(path=path):
                selected = classify([path])
                self.assertTrue(
                    selected["web"]
                    and selected["sdk"]
                    and selected["mcp"]
                    and selected["shopify"]
                )

    def test_mcp_release_workflow_selects_javascript_package_checks(self) -> None:
        selected = classify([".github/workflows/mcp-release.yml"])
        self.assertTrue(selected["mcp"] and selected["release_tooling"])

    def test_shopify_workflows_select_shopify_checks(self) -> None:
        selected = classify([".github/workflows/shopify-deploy.yml"])
        self.assertTrue(selected["shopify"] and selected["release_tooling"])
        self.assertFalse(selected["sdk"])

    def test_every_checked_in_crate_selects_linux_rust(self) -> None:
        repository_root = Path(__file__).resolve().parents[2]
        crate_manifests = sorted((repository_root / "crates").glob("*/Cargo.toml"))
        self.assertTrue(crate_manifests)
        for manifest in crate_manifests:
            source_path = f"crates/{manifest.parent.name}/src/lib.rs"
            with self.subTest(crate=manifest.parent.name):
                self.assertTrue(classify([source_path])["rust_server"])

    def test_new_crates_fail_closed_to_linux_rust(self) -> None:
        selected = classify(["crates/future-crate/src/lib.rs"])
        self.assertTrue(selected["rust_server"])
        self.assertFalse(selected["rust_shared"])

    def test_document_renderer_and_support_packs_select_linux_rust(self) -> None:
        for path in (
            "crates/document-renderer/src/lib.rs",
            "crates/support-packs/src/lib.rs",
        ):
            with self.subTest(path=path):
                self.assertTrue(classify([path])["rust_server"])

    def test_shared_agent_change_checks_both_native_platforms(self) -> None:
        selected = classify(["crates/protocol/src/lib.rs"])
        self.assertTrue(selected["rust_shared"])
        self.assertTrue(selected["macos_rust"])
        self.assertTrue(selected["windows_rust"])

    def test_printpacket_core_checks_native_and_sdk_consumers(self) -> None:
        crate = classify(["crates/printpacket/src/lib.rs"])
        self.assertTrue(crate["rust_shared"])
        self.assertTrue(crate["macos_rust"] and crate["windows_rust"])

        standard = classify(["standards/printpacket/schema/printpacket-v1.schema.json"])
        self.assertTrue(standard["sdk"] and standard["web"])
        self.assertTrue(standard["mcp"] and standard["shopify"])

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

    def test_apple_node_sdk_selects_strict_swift_validation(self) -> None:
        for path in (
            "sdk/apple/Sources/PiqaeNodeKit/PiqaeNode.swift",
            "release/tools/test_apple_node_sdk.sh",
            "release/tools/test_apple_node_sdk_linked.sh",
        ):
            with self.subTest(path=path):
                selected = classify([path])
                self.assertTrue(selected["macos_shell"])
                self.assertFalse(selected["windows_shell"] or selected["windows_installer"])
        self.assertTrue(
            classify(["sdk/apple/Sources/PiqaeNodeKit/PiqaeNode.swift"])["sdk"]
        )

    def test_release_workflow_change_does_not_select_every_platform(self) -> None:
        selected = classify([".github/workflows/macos-promotion.yml"])
        self.assertTrue(selected["release_tooling"] and selected["macos_packaging"])
        self.assertFalse(selected["windows_installer"] or selected["sdk"] or selected["web"])

    def test_release_orchestration_selects_native_packaging_only(self) -> None:
        selected = classify([".github/workflows/release.yml"])
        self.assertTrue(selected["release_tooling"])
        self.assertTrue(selected["macos_packaging"] and selected["windows_installer"])
        self.assertTrue(selected["sdk"])
        self.assertFalse(selected["web"] or selected["terraform"])

    def test_ci_classifier_change_exercises_all_scopes(self) -> None:
        self.assertEqual(
            classify(["release/tools/ci_changed_paths.py"]),
            {group: True for group in GROUPS},
        )

    def test_contract_change_checks_contract_and_javascript(self) -> None:
        selected = classify(["contracts/openapi/piqae-v1.yaml"])
        self.assertTrue(
            selected["openapi"]
            and selected["web"]
            and selected["sdk"]
            and selected["mcp"]
            and selected["shopify"]
        )

    def test_root_lockfile_selects_every_rust_platform(self) -> None:
        selected = classify(["Cargo.lock"])
        self.assertTrue(selected["rust_server"] and selected["rust_shared"])
        self.assertTrue(selected["macos_rust"] and selected["windows_rust"])
        self.assertTrue(selected["dependency_policy"])

    def test_all_override_selects_every_group(self) -> None:
        self.assertEqual(classify([], run_all=True), {group: True for group in GROUPS})


if __name__ == "__main__":
    unittest.main()
