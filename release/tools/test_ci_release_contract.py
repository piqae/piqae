from pathlib import Path
import re
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
        workspace_resolve = dockerfile.index("await import('@printpacket/core')")
        application = dockerfile.index("COPY apps/shopify apps/shopify")
        deploy = dockerfile.index(
            "RUN pnpm --filter @piqae/shopify-app --prod deploy --legacy /app"
        )
        production_resolve = dockerfile.index(
            "RUN cd /app && node --input-type=module"
        )
        final_copy = dockerfile.index(
            "COPY --from=shopify-production-build --chown=node:node /app /app"
        )
        self.assertLess(manifest, install)
        self.assertLess(sdk_manifest, install)
        self.assertLess(install, source)
        self.assertLess(install, sdk_source)
        self.assertLess(source, build)
        self.assertLess(sdk_source, sdk_build)
        self.assertLess(build, workspace_resolve)
        self.assertLess(sdk_build, application)
        self.assertLess(workspace_resolve, application)
        self.assertLess(application, deploy)
        self.assertLess(deploy, production_resolve)
        self.assertLess(production_resolve, final_copy)

        clean_target = (
            "docker build --target shopify-production-build "
            "--file deploy/docker/Dockerfile.shopify ."
        )
        workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        xtask = (ROOT / "xtask/src/main.rs").read_text(encoding="utf-8")
        railway_runbook = (
            ROOT / "docs/operations/shopify-release.md"
        ).read_text(encoding="utf-8")
        self.assertIn(clean_target, workflow)
        self.assertIn("/sdk/printpacket/**", railway_runbook)
        for argument in (
            '"docker"',
            '"build"',
            '"--target"',
            '"shopify-production-build"',
            '"deploy/docker/Dockerfile.shopify"',
        ):
            self.assertIn(argument, xtask)

    def test_postgres_evidence_checkout_includes_release_tags(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        postgres = workflow.split("\n  rust-postgres:", 1)[1].split("\n  rust-macos:", 1)[0]
        self.assertIn("fetch-depth: 0", postgres)
        self.assertIn("fetch-tags: true", postgres)

    def test_railway_migration_lane_invokes_the_container_binary(self) -> None:
        migrate = (ROOT / "railway.migrate.toml").read_text(encoding="utf-8")
        self.assertIn(
            'startCommand = "/usr/local/bin/piqae-server migrate"', migrate
        )
        self.assertIn('restartPolicyType = "NEVER"', migrate)

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

    def test_apple_marketing_version_tracks_the_workspace_release(self) -> None:
        workspace = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
        workspace_version = re.search(
            r'(?ms)^\[workspace\.package\].*?^version = "([^"]+)"', workspace
        ).group(1)
        project = (ROOT / "apps/node-apple/project.yml").read_text(encoding="utf-8")
        project_version = re.search(
            r"(?m)^\s+MARKETING_VERSION: ([^\s]+)$", project
        ).group(1)
        generated = (
            ROOT / "apps/node-apple/PiqaeNodeApple.xcodeproj/project.pbxproj"
        ).read_text(encoding="utf-8")

        self.assertEqual(project_version, workspace_version)
        self.assertEqual(
            generated.count(f"MARKETING_VERSION = {workspace_version};"), 2
        )

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

    def test_release_workflow_has_independent_scopes_and_strict_aggregate(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        self.assertIn("platform:\n        description: Candidate scope", workflow)
        self.assertIn("default: macos", workflow)
        for scope in (
            "macos",
            "windows",
            "linux",
            "containers",
            "apple-sdk",
            "windows-sdk",
            "all",
        ):
            self.assertIn(f"          - {scope}\n", workflow)
        self.assertIn('release_platform.py "$platform"', workflow)
        self.assertIn("platform=all", workflow)
        self.assertIn("node-version: 22", workflow)
        self.assertNotIn("node-version-file: .node-version", workflow)
        self.assertIn("pnpm install --frozen-lockfile", workflow)
        self.assertIn("produces a private candidate only", workflow)
        self.assertIn("Windows stable publication is Disabled", workflow)
        self.assertEqual(
            workflow.count("cargo xtask release check --platform core"), 1
        )
        self.assertIn("needs: [prepare, core]", workflow)
        selectors = {
            "macos": "macos_enabled",
            "windows": "windows_selected",
            "apple_sdk": "apple_sdk_enabled",
            "windows_sdk": "windows_sdk_enabled",
            "linux": "linux_enabled",
            "server": "containers_enabled",
        }
        for job, selector in selectors.items():
            match = re.search(rf"(?ms)^  {job}:.*?(?=^  [a-z_]+:|\Z)", workflow)
            self.assertIsNotNone(match)
            section = match.group(0)
            self.assertIn(f"needs.prepare.outputs.{selector} == 'true'", section)

        macos_call = re.search(r"(?ms)^  macos:.*?(?=^  [a-z_]+:|\Z)", workflow)
        self.assertIn("publish: false", macos_call.group(0))
        macos_promotion = re.search(
            r"(?ms)^  promote_macos:.*?(?=^  [a-z_]+:|\Z)", workflow
        )
        self.assertIsNotNone(macos_promotion)
        self.assertIn("needs: [prepare, macos]", macos_promotion.group(0))
        for sibling in (
            "windows",
            "apple_sdk",
            "windows_sdk",
            "linux",
            "server",
            "promote_containers",
        ):
            self.assertNotIn(f"needs.{sibling}.result", macos_promotion.group(0))
        self.assertIn(
            "uses: ./.github/workflows/macos-promotion.yml",
            macos_promotion.group(0),
        )
        finalizer = re.search(
            r"(?ms)^  finalize_macos:.*?(?=^  [a-z_]+:|\Z)", workflow
        )
        self.assertIn("needs: [prepare, promote_macos]", finalizer.group(0))
        self.assertIn("needs.promote_macos.result == 'success'", finalizer.group(0))
        self.assertIn("Aggregate all-platform certification is still pending", finalizer.group(0))

        macos_workflow = (ROOT / ".github/workflows/macos-release.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("value: ${{ jobs.candidate.outputs.artifact_name }}", macos_workflow)

        windows_call = re.search(r"(?ms)^  windows:.*?(?=^  [a-z_]+:|\Z)", workflow)
        self.assertIn(
            "publish: ${{ needs.prepare.outputs.publish == 'true' }}",
            windows_call.group(0),
        )

        server = re.search(r"(?ms)^  server:.*?(?=^  [a-z_]+:|\Z)", workflow)
        self.assertIn("push: false", server.group(0))
        self.assertIn("outputs: type=docker", server.group(0))
        self.assertIn("name: piqae-container-${{ matrix.name }}", server.group(0))
        self.assertNotIn("docker/login-action", server.group(0))

        container_promotion = re.search(
            r"(?ms)^  promote_containers:.*?(?=^  [a-z_]+:|\Z)", workflow
        )
        self.assertIsNotNone(container_promotion)
        self.assertIn("needs: [prepare, server]", container_promotion.group(0))
        self.assertIn("needs.server.result == 'success'", container_promotion.group(0))
        self.assertIn("environment: native-release", container_promotion.group(0))
        self.assertIn("fail-fast: false", container_promotion.group(0))
        self.assertIn("sha256sum --check", container_promotion.group(0))
        self.assertIn("docker push", container_promotion.group(0))

        aggregate = re.search(
            r"(?ms)^  certify_all:.*?(?=^  [a-z_]+:|\Z)", workflow
        )
        self.assertIsNotNone(aggregate)
        for selected in (
            "core",
            "macos",
            "windows",
            "apple_sdk",
            "windows_sdk",
            "linux",
            "server",
            "promote_containers",
            "promote_macos",
            "finalize_macos",
        ):
            self.assertIn(f"      - {selected}\n", aggregate.group(0))
        self.assertIn("aggregate_enabled == 'true'", aggregate.group(0))
        self.assertIn("python3 release/tools/release_completion.py", aggregate.group(0))
        for lane in (
            "core",
            "macos",
            "windows",
            "apple-sdk",
            "windows-sdk",
            "linux",
            "containers",
            "macos-promotion",
            "macos-prerelease",
            "container-promotion",
        ):
            self.assertIn(f'--{lane} "$', aggregate.group(0))
        self.assertIn("No physical-print or Supported-platform claim is implied", aggregate.group(0))

    def test_sibling_platform_promoters_accept_only_draft_or_prerelease(self) -> None:
        macos = (ROOT / "packaging/release/promote-macos-release.sh").read_text(
            encoding="utf-8"
        )
        windows = (ROOT / ".github/workflows/windows-release.yml").read_text(
            encoding="utf-8"
        )
        state_gate = (
            "--json isDraft,isPrerelease \\\n"
            "    --jq '(.isDraft == true) or (.isPrerelease == true)'"
        )
        self.assertIn(state_gate, macos)
        self.assertIn(
            "--jq '(.isDraft == true) or (.isPrerelease == true)'", windows
        )

    def test_registry_provenance_publishers_have_attestation_write(self) -> None:
        publishers = []
        for path in sorted((ROOT / ".github/workflows").glob("*.yml")):
            workflow = path.read_text(encoding="utf-8")
            for match in re.finditer(
                r"(?ms)^  ([a-z_]+):.*?(?=^  [a-z_]+:|\Z)", workflow
            ):
                job_name = match.group(1)
                job = match.group(0)
                if "push-to-registry: true" not in job:
                    continue
                publishers.append(f"{path.name}:{job_name}")
                configuration = job.split("\n    steps:", 1)[0]
                self.assertIn(
                    "attestations: write",
                    configuration,
                    f"{path.name}:{job_name} persists registry provenance",
                )
                self.assertNotIn("attestations: read", configuration)
        self.assertIn("release.yml:promote_containers", publishers)
        self.assertGreaterEqual(len(publishers), 1)

    def test_release_artifacts_fan_out_after_one_shared_gate(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        for job in ("macos", "windows", "apple_sdk", "windows_sdk", "linux", "server"):
            match = re.search(rf"(?ms)^  {job}:.*?(?=^  [a-z_]+:|\Z)", workflow)
            self.assertIsNotNone(match)
            self.assertIn("needs: [prepare, core]", match.group(0))
        for matrix_job in ("linux", "server"):
            match = re.search(rf"(?ms)^  {matrix_job}:.*?(?=^  [a-z_]+:|\Z)", workflow)
            self.assertIn("fail-fast: false", match.group(0))

    def test_apple_xcframework_honors_cargo_target_directory(self) -> None:
        script = (ROOT / "sdk/apple/scripts/build-xcframework.sh").read_text(
            encoding="utf-8"
        )
        self.assertIn('cargo_target_directory=$(cargo metadata', script)
        self.assertIn(
            '"$cargo_target_directory/aarch64-apple-darwin/release/libpiqae_node_ffi.a"',
            script,
        )
        self.assertNotIn(
            '"$repository_root/target/aarch64-apple-darwin/release/libpiqae_node_ffi.a"',
            script,
        )


if __name__ == "__main__":
    unittest.main()
