from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]


class CiReleaseContractTest(unittest.TestCase):
    def test_shopify_image_builds_and_resolves_every_workspace_dependency(self) -> None:
        dockerfile = (ROOT / "deploy/docker/Dockerfile.shopify").read_text(
            encoding="utf-8"
        )
        manifest = dockerfile.index(
            "COPY sdk/printpacket/package.json sdk/printpacket/package.json"
        )
        sdk_manifest = dockerfile.index(
            "COPY sdk/typescript/package.json sdk/typescript/package.json"
        )
        install = dockerfile.index("RUN pnpm install --frozen-lockfile")
        source = dockerfile.index("COPY sdk/printpacket sdk/printpacket")
        sdk_source = dockerfile.index("COPY sdk/typescript sdk/typescript")
        build = dockerfile.index("RUN pnpm --filter @printpacket/core build")
        sdk_build = dockerfile.index("RUN pnpm --filter @piqae/sdk build")
        resolve = dockerfile.index("await import('@printpacket/core')")
        application = dockerfile.index("COPY apps/shopify apps/shopify")
        self.assertLess(manifest, install)
        self.assertLess(sdk_manifest, install)
        self.assertLess(install, source)
        self.assertLess(install, sdk_source)
        self.assertLess(source, build)
        self.assertLess(sdk_source, sdk_build)
        self.assertLess(build, resolve)
        self.assertLess(sdk_build, application)
        self.assertLess(resolve, application)

        clean_target = (
            "docker build --target shopify-workspace-dependencies "
            "--file deploy/docker/Dockerfile.shopify ."
        )
        workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        xtask = (ROOT / "xtask/src/main.rs").read_text(encoding="utf-8")
        self.assertIn(clean_target, workflow)
        for argument in (
            '"docker"',
            '"build"',
            '"--target"',
            '"shopify-workspace-dependencies"',
            '"deploy/docker/Dockerfile.shopify"',
        ):
            self.assertIn(argument, xtask)

    def test_postgres_evidence_checkout_includes_release_tags(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        postgres = workflow.split("\n  rust-postgres:", 1)[1].split("\n  rust-macos:", 1)[0]
        self.assertIn("fetch-depth: 0", postgres)
        self.assertIn("fetch-tags: true", postgres)

    def test_apple_app_and_sdk_share_one_explicit_linked_artifact(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        apple = workflow.split("\n  macos-shell:", 1)[1].split("\n  rust-windows:", 1)[0]
        job_configuration = apple.split("\n    steps:", 1)[0]
        self.assertIn("timeout-minutes: 35", job_configuration)
        build = apple.index("sdk/apple/scripts/build-xcframework.sh --replace")
        linked = apple.index("test_apple_node_sdk_linked.sh use-existing")
        app = apple.index("test_apple_node_app.sh")
        self.assertLess(build, linked)
        self.assertLess(linked, app)

    def test_project_validation_uses_the_complete_app_tree(self) -> None:
        script = (ROOT / "release/tools/test_apple_node_app.sh").read_text(encoding="utf-8")
        self.assertIn("for source in Config Resources Sources Tests", script)
        self.assertIn("project-only|linked-simulator", script)

    def test_native_sdk_archives_generate_licence_evidence_before_packaging(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        windows = workflow.split("\n  windows_sdk:", 1)[1].split("\n  linux:", 1)[0]
        self.assertLess(
            windows.index("generate-license-report"),
            windows.index("Compress-Archive"),
        )

        apple = (ROOT / "sdk/apple/scripts/build-xcframework.sh").read_text(
            encoding="utf-8"
        )
        self.assertLess(
            apple.index("generate-license-report"),
            apple.index('zip -X -q "$archive"'),
        )

        windows_pack = (ROOT / "release/tools/test_windows_node_sdk.ps1").read_text(
            encoding="utf-8"
        )
        self.assertLess(
            windows_pack.index("generate-third-party-licenses"),
            windows_pack.index("dotnet pack"),
        )
        self.assertIn("/p:PiqaeThirdPartyLicenses=", windows_pack)


if __name__ == "__main__":
    unittest.main()
