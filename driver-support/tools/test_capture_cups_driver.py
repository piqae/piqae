import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("capture_cups_driver.py")
SPEC = importlib.util.spec_from_file_location("capture_cups_driver", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
SPEC.loader.exec_module(MODULE)


class CaptureCupsDriverTests(unittest.TestCase):
    def test_capture_is_redacted_and_preserves_native_choices(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            options = root / "options.txt"
            ppd = root / "private_queue_name.ppd"
            options.write_text(
                "PageSize/Page Size: *A4/A4 Letter/US\\ Letter\n"
                "VendorSensor/Sensor: *Gap/Gap BlackMark/Black\\ Mark Continuous/None\n",
                encoding="utf-8",
            )
            ppd.write_text(
                '*Manufacturer: "Example"\n*ModelName: "Label 1"\n*NickName: "Example PS 1.2"\n',
                encoding="utf-8",
            )
            result = MODULE.capture("private_queue_name", options, ppd)
            encoded = json.dumps(result)
            self.assertNotIn("private_queue_name", encoded)
            self.assertEqual(result["driver_package"]["files"][0]["name"], "driver.ppd")
            self.assertTrue(result["redacted"])
            sensor = next(item for item in result["capabilities"] if item["key"] == "VendorSensor")
            self.assertEqual([item["value"] for item in sensor["choices"]], ["Gap", "BlackMark", "Continuous"])
            self.assertEqual(len(result["driver_package"]["canonical_inventory_sha256"]), 64)

    def test_rejects_unsafe_queue_and_oversized_values(self):
        with self.assertRaises(ValueError):
            MODULE.capture("../queue", Path("missing"), None)
        with self.assertRaises(ValueError):
            MODULE.parse_lpoptions("K/Name: " + "x" * 513)


if __name__ == "__main__":
    unittest.main()
