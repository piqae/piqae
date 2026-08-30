from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from release_prerelease_notes import render, update_state


class ReleasePrereleaseNotesTest(unittest.TestCase):
    def test_sibling_finalizer_order_is_idempotent(self) -> None:
        mac_first = update_state(
            "", scope="all", windows_enabled=True, platform="macos"
        )
        mac_then_windows = update_state(
            render(mac_first),
            scope="all",
            windows_enabled=True,
            platform="windows",
        )
        windows_first = update_state(
            "", scope="all", windows_enabled=True, platform="windows"
        )
        windows_then_mac = update_state(
            render(windows_first),
            scope="all",
            windows_enabled=True,
            platform="macos",
        )
        self.assertEqual(mac_then_windows, windows_then_mac)
        self.assertEqual(render(mac_then_windows), render(windows_then_mac))
        self.assertEqual(
            mac_then_windows["platforms"],
            {"macos": "published", "windows": "published"},
        )

    def test_repeated_platform_finalization_is_a_noop(self) -> None:
        once = update_state(
            "", scope="all", windows_enabled=True, platform="macos"
        )
        twice = update_state(
            render(once), scope="all", windows_enabled=True, platform="macos"
        )
        self.assertEqual(once, twice)

    def test_pending_transitions_to_failed_without_certifying_assets(self) -> None:
        macos = update_state(
            "", scope="all", windows_enabled=True, platform="macos"
        )
        failed = update_state(
            render(macos), scope="all", windows_enabled=True, aggregate="failed"
        )
        self.assertEqual(failed["aggregate"], "failed")
        self.assertEqual(failed["platforms"]["macos"], "published")
        self.assertEqual(failed["platforms"]["windows"], "failed")
        self.assertIn("Aggregate all-platform certification: **Failed**", render(failed))

    def test_pending_transitions_to_passed_after_both_publish(self) -> None:
        macos = update_state(
            "", scope="all", windows_enabled=True, platform="macos"
        )
        windows = update_state(
            render(macos),
            scope="all",
            windows_enabled=True,
            platform="windows",
        )
        passed = update_state(
            render(windows), scope="all", windows_enabled=True, aggregate="passed"
        )
        self.assertEqual(passed["aggregate"], "passed")
        self.assertIn("Aggregate all-platform certification: **Passed**", render(passed))

    def test_pending_platform_cannot_be_marked_aggregate_passed(self) -> None:
        macos = update_state(
            "", scope="all", windows_enabled=True, platform="macos"
        )
        with self.assertRaisesRegex(ValueError, "unpublished platforms"):
            update_state(
                render(macos),
                scope="all",
                windows_enabled=True,
                aggregate="passed",
            )

    def test_disabled_windows_is_not_misreported_as_failed(self) -> None:
        macos = update_state(
            "", scope="all", windows_enabled=False, platform="macos"
        )
        failed = update_state(
            render(macos), scope="all", windows_enabled=False, aggregate="failed"
        )
        self.assertEqual(failed["platforms"]["windows"], "not-selected")


if __name__ == "__main__":
    unittest.main()
