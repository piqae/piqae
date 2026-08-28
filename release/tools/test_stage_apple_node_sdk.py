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
    @classmethod
    def setUpClass(cls) -> None:
        cls.repository_root = MODULE_PATH.parents[2]
        cls.apple_graph = MODULE.native_cargo_sbom.load_cargo_graph(
            cls.repository_root, MODULE.APPLE_RUST_TARGETS
        )

    def setUp(self) -> None:
        self.fixture = tempfile.TemporaryDirectory()
        self.root = Path(self.fixture.name)
        self.stage = self.root / "stage"
        self.stage.mkdir()
        (self.root / "LICENSE").write_bytes((self.repository_root / "LICENSE").read_bytes())
        (self.root / "NOTICE").write_bytes((self.repository_root / "NOTICE").read_bytes())
        self.version = "1.2.3"
        self.revision = "a" * 40
        self.native_name = f"PiqaeNode.xcframework-{self.version}.zip"
        self.source_name = f"PiqaeNodeKit-{self.version}.zip"
        with mock.patch.object(
            MODULE.native_cargo_sbom, "load_cargo_graph", return_value=self.apple_graph
        ):
            third_party_licenses = MODULE.native_cargo_sbom.third_party_license_report_bytes(
                self.repository_root, MODULE.APPLE_RUST_TARGETS
            )
        with zipfile.ZipFile(self.stage / self.native_name, "w") as native:
            native.writestr("PiqaeNode.xcframework/LICENSE", (self.root / "LICENSE").read_bytes())
            native.writestr("PiqaeNode.xcframework/NOTICE", (self.root / "NOTICE").read_bytes())
            native.writestr("PiqaeNode.xcframework/Info.plist", "fixture")
            native.writestr("PiqaeNode.xcframework/macos/libPiqaeNode.a", "native")
            native.writestr(
                f"PiqaeNode.xcframework/{MODULE.native_cargo_sbom.THIRD_PARTY_LICENSES_FILENAME}",
                third_party_licenses,
            )
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
            package.writestr(f"{package_root}/LICENSE", (self.root / "LICENSE").read_bytes())
            package.writestr(f"{package_root}/NOTICE", (self.root / "NOTICE").read_bytes())
            package.writestr(
                f"{package_root}/{MODULE.native_cargo_sbom.THIRD_PARTY_LICENSES_FILENAME}",
                third_party_licenses,
            )
        native_hash = MODULE.sha256(self.stage / self.native_name)
        source_hash = MODULE.sha256(self.stage / self.source_name)
        self.native_sbom = f"PiqaeNode.xcframework-{self.version}.spdx.json"
        self.source_sbom = f"PiqaeNodeKit-{self.version}.spdx.json"
        with mock.patch.object(
            MODULE.native_cargo_sbom, "load_cargo_graph", return_value=self.apple_graph
        ):
            MODULE.native_cargo_sbom.generate_native_sbom(
                self.stage / self.native_name,
                self.stage / self.native_sbom,
                self.repository_root,
                MODULE.APPLE_RUST_TARGETS,
                "PiqaeNodeNative",
                self.version,
                (".a",),
            )
        MODULE.generate_archive_sbom(
            self.stage / self.source_name,
            "PiqaeNodeKit",
            self.version,
            self.stage / self.source_sbom,
        )
        (self.stage / f"{self.native_name}.sha256").write_text(
            f"{native_hash}  {self.native_name}\n", encoding="ascii"
        )
        (self.stage / f"{self.source_name}.sha256").write_text(
            f"{source_hash}  {self.source_name}\n", encoding="ascii"
        )
        self.manifest = {
            "schema": 2,
            "native_abi": 1,
            "native_contract": {"current": 2, "supported": [2]},
            "capability_command": "print_packet_capabilities",
            "capability_contract": "printpacket/v1",
            "rust_targets": list(MODULE.APPLE_RUST_TARGETS),
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
            "sboms": {
                self.native_name: self.native_sbom,
                self.source_name: self.source_sbom,
            },
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
        ), mock.patch.object(
            MODULE.native_cargo_sbom, "load_cargo_graph", return_value=self.apple_graph
        ):
            MODULE.validate_stage(self.repository_root, self.version, self.stage)

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

    def test_rejects_native_contract_drift(self) -> None:
        self.manifest["native_contract"] = {"current": 1, "supported": [1]}
        self.write_manifest()
        with self.assertRaisesRegex(MODULE.ReleaseError, "native_contract"):
            self.validate()

    def test_rejects_sbom_that_omits_an_archive_file(self) -> None:
        path = self.stage / self.native_sbom
        document = json.loads(path.read_text(encoding="utf-8"))
        document["files"] = document["files"][:-1]
        path.write_text(json.dumps(document), encoding="utf-8")
        with self.assertRaisesRegex(MODULE.ReleaseError, "every archive file"):
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
            archive.writestr(f"{package_root}/LICENSE", (self.root / "LICENSE").read_bytes())
            archive.writestr(f"{package_root}/NOTICE", (self.root / "NOTICE").read_bytes())
            with mock.patch.object(
                MODULE.native_cargo_sbom, "load_cargo_graph", return_value=self.apple_graph
            ):
                archive.writestr(
                    f"{package_root}/{MODULE.native_cargo_sbom.THIRD_PARTY_LICENSES_FILENAME}",
                    MODULE.native_cargo_sbom.third_party_license_report_bytes(
                        self.repository_root, MODULE.APPLE_RUST_TARGETS
                    ),
                )
        source_hash = MODULE.sha256(package)
        self.manifest["source_package_sha256"] = source_hash
        self.write_manifest()
        (self.stage / f"{self.source_name}.sha256").write_text(
            f"{source_hash}  {self.source_name}\n", encoding="ascii"
        )
        with self.assertRaisesRegex(MODULE.ReleaseError, "artifact URL"):
            self.validate()

    def test_rejects_package_manifest_without_native_linker_settings(self) -> None:
        package = self.stage / self.source_name
        rewritten = self.root / "missing-linker-settings.zip"
        package_root = f"PiqaeNodeKit-{self.version}"
        manifest_name = f"{package_root}/Package.swift"
        with zipfile.ZipFile(package) as source, zipfile.ZipFile(rewritten, "w") as output:
            for name in source.namelist():
                contents = source.read(name)
                if name == manifest_name:
                    contents = contents.replace(
                        b'.linkedLibrary("bsm", .when(platforms: [.macOS]))', b""
                    )
                output.writestr(name, contents)
        rewritten.replace(package)
        source_hash = MODULE.sha256(package)
        self.manifest["source_package_sha256"] = source_hash
        self.write_manifest()
        (self.stage / f"{self.source_name}.sha256").write_text(
            f"{source_hash}  {self.source_name}\n", encoding="ascii"
        )
        with self.assertRaisesRegex(MODULE.ReleaseError, "native linker settings"):
            self.validate()

    def test_rejects_tampered_third_party_licence_report(self) -> None:
        archive_path = self.stage / self.native_name
        rewritten = self.root / "tampered.zip"
        report = f"PiqaeNode.xcframework/{MODULE.native_cargo_sbom.THIRD_PARTY_LICENSES_FILENAME}"
        with zipfile.ZipFile(archive_path) as source, zipfile.ZipFile(rewritten, "w") as output:
            for name in source.namelist():
                contents = source.read(name)
                output.writestr(name, b"{}\n" if name == report else contents)
        rewritten.replace(archive_path)
        native_hash = MODULE.sha256(archive_path)
        self.manifest["sha256"] = native_hash
        self.write_manifest()
        (self.stage / f"{self.native_name}.sha256").write_text(
            f"{native_hash}  {self.native_name}\n", encoding="ascii"
        )
        with self.assertRaisesRegex(MODULE.ReleaseError, "third-party licence report"):
            self.validate()


if __name__ == "__main__":
    unittest.main()
