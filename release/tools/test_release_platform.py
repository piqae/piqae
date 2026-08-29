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
        self.assertFalse(selection.full)

    def test_all_retains_full_certification(self) -> None:
        selection = classify("all")
        self.assertTrue(selection.macos)
        self.assertTrue(selection.full)

    def test_unknown_scope_fails_closed(self) -> None:
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "windows"],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("platform=", result.stdout)


if __name__ == "__main__":
    unittest.main()
