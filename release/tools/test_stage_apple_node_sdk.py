import hashlib
import importlib.util
import json
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock


MODULE_PATH = Path(__file__).with_name("stage_apple_node_sdk.py")
SPEC = importlib.util.spec_from_file_location("stage_apple_node_sdk", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class AppleNodeSdkStageTests(unittest.TestCase):
    def setUp(self) -> None:
        self.fixture = tempfile.TemporaryDirectory()
        self.root = Path(self.fixture.name)
        self.stage = self.root / "stage"
        self.stage.mkdir()
        self.version = "1.2.3"
        self.revision = "a" * 40
        self.native_name = f"PiqaeNode.xcframework-{self.version}.zip"
        self.source_name = f"PiqaeNodeKit-{self.version}.zip"
        (self.stage / self.native_name).write_bytes(b"native archive")
        package_root = f"PiqaeNodeKit-{self.version}"
        with zipfile.ZipFile(self.stage / self.source_name, "w") as package:
            package.writestr(
                f"{package_root}/Package.swift",
                MODULE.package_manifest(self.version, self.native_name, "native-checksum"),
            )
            package.writestr(
                f"{package_root}/Sources/CPiqaeNodeABI/include/piqae_node.h", "header"
            )
            package.writestr(
                f"{package_root}/Sources/CPiqaeNodeABI/include/shim.h",
                '#include "piqae_node.h"\n',
            )
            package.writestr(f"{package_root}/Sources/PiqaeNodeKit/PiqaeNode.swift", "source")
        native_hash = hashlib.sha256(b"native archive").hexdigest()
        source_hash = MODULE.sha256(self.stage / self.source_name)
        (self.stage / f"{self.native_name}.sha256").write_text(
            f"{native_hash}  {self.native_name}\n", encoding="ascii"
        )
        (self.stage / f"{self.source_name}.sha256").write_text(
            f"{source_hash}  {self.source_name}\n", encoding="ascii"
        )
        self.manifest = {
            "schema": 2,
            "version": self.version,
            "tag": f"v{self.version}",
            "git_revision": self.revision,
            "artifact": self.native_name,
            "download_url": f"https://github.com/piqae/piqae/releases/download/v{self.version}/{self.native_name}",
            "swiftpm_checksum": "native-checksum",
            "sha256": native_hash,
            "source_package": self.source_name,
            "source_package_download_url": f"https://github.com/piqae/piqae/releases/download/v{self.version}/{self.source_name}",
            "source_package_sha256": source_hash,
            "slices": ["macos-arm64_x86_64"],
        }
        self.write_manifest()

    def tearDown(self) -> None:
        self.fixture.cleanup()

    def write_manifest(self) -> None:
        (self.stage / "PiqaeNode.artifact.json").write_text(
            json.dumps(self.manifest), encoding="utf-8"
        )

    def validate(self) -> None:
        with mock.patch.object(MODULE, "exact_git_revision", return_value=self.revision), mock.patch.object(
            MODULE, "swiftpm_checksum", return_value="native-checksum"
        ):
            MODULE.validate_stage(self.root, self.version, self.stage)

    def test_accepts_exact_versioned_distribution(self) -> None:
        self.validate()

    def test_rejects_manifest_that_names_unversioned_upload(self) -> None:
        self.manifest["artifact"] = "PiqaeNode.xcframework.zip"
        self.write_manifest()
        with self.assertRaisesRegex(MODULE.ReleaseError, "artifact"):
            self.validate()

    def test_rejects_checksum_drift(self) -> None:
        self.manifest["sha256"] = "0" * 64
        self.write_manifest()
        with self.assertRaisesRegex(MODULE.ReleaseError, "SHA-256"):
            self.validate()

    def test_rejects_package_manifest_without_release_checksum(self) -> None:
        package = self.stage / self.source_name
        package.unlink()
        package_root = f"PiqaeNodeKit-{self.version}"
        with zipfile.ZipFile(package, "w") as archive:
            archive.writestr(f"{package_root}/Package.swift", "// missing checksum")
            archive.writestr(f"{package_root}/Sources/CPiqaeNodeABI/include/piqae_node.h", "h")
            archive.writestr(
                f"{package_root}/Sources/CPiqaeNodeABI/include/shim.h", '#include "piqae_node.h"'
            )
            archive.writestr(f"{package_root}/Sources/PiqaeNodeKit/PiqaeNode.swift", "s")
        source_hash = MODULE.sha256(package)
        self.manifest["source_package_sha256"] = source_hash
        self.write_manifest()
        (self.stage / f"{self.source_name}.sha256").write_text(
            f"{source_hash}  {self.source_name}\n", encoding="ascii"
        )
        with self.assertRaisesRegex(MODULE.ReleaseError, "artifact URL"):
            self.validate()


if __name__ == "__main__":
    unittest.main()
