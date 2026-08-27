import json
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock

from release.tools import native_cargo_sbom as MODULE
from release.tools import release_bundle


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
WINDOWS_TARGET = "x86_64-pc-windows-msvc"
APPLE_TARGETS = (
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-apple-ios",
    "aarch64-apple-ios-sim",
    "x86_64-apple-ios",
)


class NativeCargoSbomTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        # This is deliberately real cargo metadata evidence, not a handwritten fixture.
        cls.windows_graph = MODULE.load_cargo_graph(REPOSITORY_ROOT, (WINDOWS_TARGET,))
        cls.apple_graph = MODULE.load_cargo_graph(REPOSITORY_ROOT, APPLE_TARGETS)

    def setUp(self) -> None:
        self.fixture = tempfile.TemporaryDirectory()
        self.root = Path(self.fixture.name)
        self.archive = self.root / "native.zip"
        self.sbom = self.root / "native.spdx.json"
        with zipfile.ZipFile(self.archive, "w") as archive:
            archive.writestr("LICENSE", (REPOSITORY_ROOT / "LICENSE").read_bytes())
            archive.writestr("NOTICE", (REPOSITORY_ROOT / "NOTICE").read_bytes())
            archive.writestr("piqae_node_ffi.dll", b"native-binary-fixture")
        with mock.patch.object(MODULE, "load_cargo_graph", return_value=self.windows_graph):
            MODULE.generate_native_sbom(
                self.archive,
                self.sbom,
                REPOSITORY_ROOT,
                (WINDOWS_TARGET,),
                "piqae-node-ffi-native-bundle",
                "1.2.3",
                (".dll",),
            )

    def tearDown(self) -> None:
        self.fixture.cleanup()

    def validate(self) -> None:
        with mock.patch.object(MODULE, "load_cargo_graph", return_value=self.windows_graph):
            MODULE.validate_native_sbom(
                self.archive,
                self.sbom,
                REPOSITORY_ROOT,
                (WINDOWS_TARGET,),
                "piqae-node-ffi-native-bundle",
                "1.2.3",
                (".dll",),
            )

    def document(self) -> dict[str, object]:
        return json.loads(self.sbom.read_text(encoding="utf-8"))

    def write_document(self, document: dict[str, object]) -> None:
        self.sbom.write_text(json.dumps(document), encoding="utf-8")

    def test_locked_metadata_graph_is_complete_for_windows_and_all_apple_targets(self) -> None:
        self.assertEqual(self.windows_graph.targets, (WINDOWS_TARGET,))
        self.assertEqual(self.apple_graph.targets, APPLE_TARGETS)
        self.assertGreater(len(self.windows_graph.packages), 1)
        self.assertGreater(len(self.apple_graph.packages), 1)
        self.assertTrue(all(package.checksum for package in self.windows_graph.packages))
        self.assertTrue(
            all(
                package.purl.startswith("pkg:cargo/")
                for package in self.apple_graph.packages
            )
        )
        self.validate()
        release_bundle._validate_spdx(self.sbom)

    def test_rejects_missing_locked_dependency(self) -> None:
        document = self.document()
        packages = document["packages"]
        assert isinstance(packages, list)
        packages.pop()
        self.write_document(document)
        with self.assertRaisesRegex(MODULE.NativeCargoSbomError, "dependency"):
            self.validate()

    def test_rejects_forged_dependency_checksum(self) -> None:
        document = self.document()
        packages = document["packages"]
        assert isinstance(packages, list)
        dependency = next(
            package
            for package in packages
            if package["SPDXID"] != "SPDXRef-Package-NativeArtifact"
        )
        dependency["checksums"][0]["checksumValue"] = "0" * 64
        self.write_document(document)
        with self.assertRaisesRegex(MODULE.NativeCargoSbomError, "checksum"):
            self.validate()

    def test_rejects_missing_dependency_relationship(self) -> None:
        document = self.document()
        relationships = document["relationships"]
        assert isinstance(relationships, list)
        for index, relationship in enumerate(relationships):
            if (
                relationship["relationshipType"] == "DEPENDS_ON"
                and relationship["spdxElementId"] != "SPDXRef-Package-NativeArtifact"
            ):
                relationships.pop(index)
                break
        self.write_document(document)
        with self.assertRaisesRegex(MODULE.NativeCargoSbomError, "relationships"):
            self.validate()

    def test_rejects_forged_outer_archive_or_binary_checksum(self) -> None:
        document = self.document()
        packages = document["packages"]
        assert isinstance(packages, list)
        packages[0]["checksums"][0]["checksumValue"] = "0" * 64
        self.write_document(document)
        with self.assertRaisesRegex(MODULE.NativeCargoSbomError, "outer archive"):
            self.validate()

        with mock.patch.object(MODULE, "load_cargo_graph", return_value=self.windows_graph):
            MODULE.generate_native_sbom(
                self.archive,
                self.sbom,
                REPOSITORY_ROOT,
                (WINDOWS_TARGET,),
                "piqae-node-ffi-native-bundle",
                "1.2.3",
                (".dll",),
            )
        document = self.document()
        files = document["files"]
        assert isinstance(files, list)
        binary = next(entry for entry in files if entry["fileName"].endswith(".dll"))
        binary["checksums"][1]["checksumValue"] = "f" * 64
        self.write_document(document)
        with self.assertRaisesRegex(MODULE.NativeCargoSbomError, "file checksum"):
            self.validate()


if __name__ == "__main__":
    unittest.main()
