import subprocess
import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from release_platform import classify


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "release/tools/release_platform.py"


class ReleasePlatformTest(unittest.TestCase):
    def test_macos_is_a_bounded_native_artifact_set(self) -> None:
        selection = classify("macos")
        self.assertTrue(selection.macos)
        self.assertFalse(selection.windows)
        self.assertFalse(selection.linux)
        self.assertFalse(selection.containers)
        self.assertFalse(selection.apple_sdk)
        self.assertFalse(selection.windows_sdk)
        self.assertFalse(selection.aggregate)

    def test_each_candidate_scope_selects_only_its_lane(self) -> None:
        expected = {
            "windows": "windows",
            "linux": "linux",
            "containers": "containers",
            "apple-sdk": "apple_sdk",
            "windows-sdk": "windows_sdk",
        }
        for scope, field in expected.items():
            with self.subTest(scope=scope):
                selection = classify(scope)
                enabled = {
                    name
                    for name in (
                        "macos",
                        "windows",
                        "linux",
                        "containers",
                        "apple_sdk",
                        "windows_sdk",
                    )
                    if getattr(selection, name)
                }
                self.assertEqual(enabled, {field})
                self.assertFalse(selection.aggregate)

    def test_all_retains_full_certification(self) -> None:
        selection = classify("all")
        self.assertTrue(selection.macos)
        self.assertTrue(selection.windows)
        self.assertTrue(selection.linux)
        self.assertTrue(selection.containers)
        self.assertTrue(selection.apple_sdk)
        self.assertTrue(selection.windows_sdk)
        self.assertTrue(selection.aggregate)

    def test_unknown_scope_fails_closed(self) -> None:
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "mobile"],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("platform=", result.stdout)


if __name__ == "__main__":
    unittest.main()
