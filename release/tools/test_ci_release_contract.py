from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]


class CiReleaseContractTest(unittest.TestCase):
    def test_postgres_evidence_checkout_includes_release_tags(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        postgres = workflow.split("\n  rust-postgres:", 1)[1].split("\n  rust-macos:", 1)[0]
        self.assertIn("fetch-depth: 0", postgres)
        self.assertIn("fetch-tags: true", postgres)

    def test_apple_app_and_sdk_share_one_explicit_linked_artifact(self) -> None:
        workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        apple = workflow.split("\n  macos-shell:", 1)[1].split("\n  rust-windows:", 1)[0]
        build = apple.index("sdk/apple/scripts/build-xcframework.sh --replace")
        linked = apple.index("test_apple_node_sdk_linked.sh use-existing")
        app = apple.index("test_apple_node_app.sh")
        self.assertLess(build, linked)
        self.assertLess(linked, app)

    def test_project_validation_uses_the_complete_app_tree(self) -> None:
        script = (ROOT / "release/tools/test_apple_node_app.sh").read_text(encoding="utf-8")
        self.assertIn("for source in Config Resources Sources Tests", script)
        self.assertIn("project-only|linked-simulator", script)


if __name__ == "__main__":
    unittest.main()
