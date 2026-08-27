import importlib.util
import json
import tempfile
import unittest
import warnings
import zipfile
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("windows_sdk_release.py")
SPEC = importlib.util.spec_from_file_location("windows_sdk_release", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def nuspec(package_id: str, version: str, dependency: str = "[2.6.2]") -> str:
    dependency_xml = (
        f'<dependencies><group targetFramework="net8.0"><dependency id="BouncyCastle.Cryptography" version="{dependency}" /></group></dependencies>'
        if package_id == "Piqae.Node"
        else ""
    )
    return f'''<?xml version="1.0"?><package><metadata><id>{package_id}</id><version>{version}</version><license type="expression">MIT</license>{dependency_xml}</metadata></package>'''


class WindowsSdkReleaseTests(unittest.TestCase):
    def setUp(self) -> None:
        self.fixture = tempfile.TemporaryDirectory()
        self.root = Path(self.fixture.name)
        self.version = "1.2.3"
        self.package = self.root / f"Piqae.Node.{self.version}.nupkg"
        self.dependency = self.root / "BouncyCastle.Cryptography.2.6.2.nupkg"
        with zipfile.ZipFile(self.dependency, "w") as archive:
            archive.writestr("BouncyCastle.Cryptography.nuspec", nuspec("BouncyCastle.Cryptography", "2.6.2"))
            archive.writestr("LICENSE.md", "Copyright fixture\n\nMIT licence fixture\n")
        self.write_package()

    def tearDown(self) -> None:
        self.fixture.cleanup()

    def write_package(self, dependency: str = "[2.6.2]", extra_runtime: bool = False) -> None:
        report = self.root / MODULE.THIRD_PARTY_LICENSES_ENTRY
        MODULE.generate_windows_third_party_license_report(self.dependency, report)
        with zipfile.ZipFile(self.package, "w") as archive:
            archive.writestr("Piqae.Node.nuspec", nuspec("Piqae.Node", self.version, dependency))
            archive.writestr(MODULE.MANAGED_ENTRY, b"managed")
            archive.writestr(MODULE.NATIVE_ENTRY, b"native")
            archive.writestr("LICENSE", (MODULE.REPOSITORY_ROOT / "LICENSE").read_bytes())
            archive.writestr("NOTICE", (MODULE.REPOSITORY_ROOT / "NOTICE").read_bytes())
            archive.writestr(MODULE.THIRD_PARTY_LICENSES_ENTRY, report.read_bytes())
            if extra_runtime:
                archive.writestr("runtimes/win-arm64/native/piqae_node_ffi.dll", b"wrong")

    def test_generates_complete_spdx(self) -> None:
        output = self.root / "sdk.spdx.json"
        MODULE.generate_sbom(self.package, self.dependency, self.version, output)
        MODULE.validate_sbom(output, self.package, self.version)
        document = json.loads(output.read_text(encoding="utf-8"))
        names = {package["name"] for package in document["packages"]}
        self.assertEqual(names, {"Piqae.Node", "BouncyCastle.Cryptography", "piqae-node-ffi"})
        packages = {package["name"]: package for package in document["packages"]}
        self.assertEqual(packages["Piqae.Node"]["licenseConcluded"], "NOASSERTION")
        self.assertEqual(packages["piqae-node-ffi"]["licenseConcluded"], "NOASSERTION")
        native = next(
            file for file in document["files"] if file["fileName"] == f"./{MODULE.NATIVE_ENTRY}"
        )
        self.assertEqual(native["licenseConcluded"], "NOASSERTION")

    def test_rejects_unsafe_apache_only_native_license_conclusions(self) -> None:
        output = self.root / "sdk.spdx.json"
        MODULE.generate_sbom(self.package, self.dependency, self.version, output)
        document = json.loads(output.read_text(encoding="utf-8"))
        piqae = next(package for package in document["packages"] if package["name"] == "Piqae.Node")
        piqae["licenseConcluded"] = "Apache-2.0"
        output.write_text(json.dumps(document), encoding="utf-8")
        with self.assertRaisesRegex(MODULE.ReleaseError, "mixed-license"):
            MODULE.validate_sbom(output, self.package, self.version)

        MODULE.generate_sbom(self.package, self.dependency, self.version, output)
        document = json.loads(output.read_text(encoding="utf-8"))
        native = next(
            file for file in document["files"] if file["fileName"] == f"./{MODULE.NATIVE_ENTRY}"
        )
        native["licenseConcluded"] = "Apache-2.0"
        output.write_text(json.dumps(document), encoding="utf-8")
        with self.assertRaisesRegex(MODULE.ReleaseError, "native runtime"):
            MODULE.validate_sbom(output, self.package, self.version)

    def test_rejects_tampered_third_party_licence_report(self) -> None:
        rewritten = self.root / "tampered.nupkg"
        with zipfile.ZipFile(self.package) as source, zipfile.ZipFile(rewritten, "w") as output:
            for name in source.namelist():
                output.writestr(
                    name,
                    b"{}\n" if name == MODULE.THIRD_PARTY_LICENSES_ENTRY else source.read(name),
                )
        rewritten.replace(self.package)
        with self.assertRaisesRegex(MODULE.ReleaseError, "third-party licence report"):
            MODULE.validate_package(self.package, self.dependency, self.version)

    def test_rejects_dependency_without_exact_licence_text(self) -> None:
        with zipfile.ZipFile(self.dependency, "w") as archive:
            archive.writestr(
                "BouncyCastle.Cryptography.nuspec",
                nuspec("BouncyCastle.Cryptography", "2.6.2"),
            )
        with self.assertRaisesRegex(MODULE.ReleaseError, "no exact bundled LICENSE.md text"):
            MODULE.managed_dependency_license_package(self.dependency)

    def test_rejects_dependency_with_changed_licence_declaration(self) -> None:
        changed = nuspec("BouncyCastle.Cryptography", "2.6.2").replace(
            '>MIT</license>',
            '>Apache-2.0</license>',
        )
        with zipfile.ZipFile(self.dependency, "w") as archive:
            archive.writestr("BouncyCastle.Cryptography.nuspec", changed)
            archive.writestr("LICENSE.md", "Copyright fixture\n\nMIT licence fixture\n")
        with self.assertRaisesRegex(MODULE.ReleaseError, "declaration"):
            MODULE.managed_dependency_license_package(self.dependency)

    def test_rejects_floating_dependency(self) -> None:
        self.write_package(dependency="2.6.2")
        with self.assertRaisesRegex(MODULE.ReleaseError, "pin"):
            MODULE.validate_package(self.package, self.dependency, self.version)

    def test_rejects_unexpected_rid_asset(self) -> None:
        self.write_package(extra_runtime=True)
        with self.assertRaisesRegex(MODULE.ReleaseError, "RID"):
            MODULE.validate_package(self.package, self.dependency, self.version)

    def test_rejects_duplicate_or_non_portable_archive_paths(self) -> None:
        duplicate = self.root / "duplicate.zip"
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", UserWarning)
            with zipfile.ZipFile(duplicate, "w") as archive:
                archive.writestr("LICENSE", b"first")
                archive.writestr("LICENSE", b"second")
        with zipfile.ZipFile(duplicate) as archive, self.assertRaisesRegex(
            MODULE.ReleaseError, "duplicate"
        ):
            MODULE.safe_entries(archive)

        non_portable = self.root / "non-portable.zip"
        with zipfile.ZipFile(non_portable, "w") as archive:
            archive.writestr("folder\\file", b"fixture")
        with zipfile.ZipFile(non_portable) as archive, self.assertRaisesRegex(
            MODULE.ReleaseError, "path separator"
        ):
            MODULE.safe_entries(archive)

    def test_rejects_sbom_without_dependency(self) -> None:
        output = self.root / "sdk.spdx.json"
        MODULE.generate_sbom(self.package, self.dependency, self.version, output)
        document = json.loads(output.read_text(encoding="utf-8"))
        document["packages"] = [
            package for package in document["packages"] if package["name"] != "BouncyCastle.Cryptography"
        ]
        output.write_text(json.dumps(document), encoding="utf-8")
        with self.assertRaisesRegex(MODULE.ReleaseError, "omits"):
            MODULE.validate_sbom(output, self.package, self.version)

    def test_rejects_inconsistent_nuget_package_verification_code(self) -> None:
        output = self.root / "sdk.spdx.json"
        MODULE.generate_sbom(self.package, self.dependency, self.version, output)
        document = json.loads(output.read_text(encoding="utf-8"))
        piqae = next(package for package in document["packages"] if package["name"] == "Piqae.Node")
        piqae["packageVerificationCode"]["packageVerificationCodeValue"] = "0" * 40
        output.write_text(json.dumps(document), encoding="utf-8")
        with self.assertRaisesRegex(MODULE.ReleaseError, "verification code"):
            MODULE.validate_sbom(output, self.package, self.version)

    def test_rejects_nuget_sbom_file_checksum_not_bound_to_archive(self) -> None:
        output = self.root / "sdk.spdx.json"
        MODULE.generate_sbom(self.package, self.dependency, self.version, output)
        document = json.loads(output.read_text(encoding="utf-8"))
        document["files"][0]["checksums"][0]["checksumValue"] = "0" * 40
        output.write_text(json.dumps(document), encoding="utf-8")
        with self.assertRaisesRegex(MODULE.ReleaseError, "file checksum"):
            MODULE.validate_sbom(output, self.package, self.version)

    def test_native_archive_has_license_notice_and_complete_spdx(self) -> None:
        archive_path = self.root / f"PiqaeNode-native-windows-x64-{self.version}.zip"
        with zipfile.ZipFile(archive_path, "w") as archive:
            for name in MODULE.NATIVE_ARCHIVE_ENTRIES:
                contents = self.native_entry_contents(name)
                archive.writestr(name, contents)
        output = self.root / "native.spdx.json"
        MODULE.generate_native_sbom(archive_path, self.version, output)
        MODULE.validate_native_sbom(archive_path, output, self.version)

        document = json.loads(output.read_text(encoding="utf-8"))
        document["files"] = document["files"][:-1]
        output.write_text(json.dumps(document), encoding="utf-8")
        with self.assertRaisesRegex(MODULE.ReleaseError, "every archive file"):
            MODULE.validate_native_sbom(archive_path, output, self.version)

    def test_native_archive_rejects_missing_notice(self) -> None:
        archive_path = self.root / f"PiqaeNode-native-windows-x64-{self.version}.zip"
        with zipfile.ZipFile(archive_path, "w") as archive:
            for name in MODULE.NATIVE_ARCHIVE_ENTRIES - {"NOTICE"}:
                contents = self.native_entry_contents(name)
                archive.writestr(name, contents)
        with self.assertRaisesRegex(MODULE.ReleaseError, "incomplete"):
            MODULE.validate_native_archive(archive_path, self.version)

    def test_native_archive_rejects_tampered_third_party_licence_report(self) -> None:
        archive_path = self.root / f"PiqaeNode-native-windows-x64-{self.version}.zip"
        with zipfile.ZipFile(archive_path, "w") as archive:
            for name in MODULE.NATIVE_ARCHIVE_ENTRIES:
                contents = (
                    b"{}\n"
                    if name == MODULE.THIRD_PARTY_LICENSES_ENTRY
                    else self.native_entry_contents(name)
                )
                archive.writestr(name, contents)
        with self.assertRaisesRegex(MODULE.ReleaseError, "third-party licence report"):
            MODULE.validate_native_archive(archive_path, self.version)

    def test_sdk_manifest_binds_abi_contract_capabilities_and_both_archives(self) -> None:
        native_archive = self.root / f"PiqaeNode-native-windows-x64-{self.version}.zip"
        with zipfile.ZipFile(native_archive, "w") as archive:
            for name in MODULE.NATIVE_ARCHIVE_ENTRIES:
                contents = self.native_entry_contents(name)
                archive.writestr(name, contents)
        nuget_sbom = self.root / "nuget.spdx.json"
        native_sbom = self.root / "native.spdx.json"
        manifest = self.root / "PiqaeNode.windows-sdk-artifact.json"
        MODULE.generate_sbom(self.package, self.dependency, self.version, nuget_sbom)
        MODULE.generate_native_sbom(native_archive, self.version, native_sbom)
        MODULE.generate_sdk_manifest(
            self.package,
            native_archive,
            nuget_sbom,
            native_sbom,
            self.version,
            manifest,
        )
        MODULE.validate_sdk_manifest(
            self.package,
            native_archive,
            nuget_sbom,
            native_sbom,
            self.version,
            manifest,
        )
        document = json.loads(manifest.read_text(encoding="utf-8"))
        document["native_contract"] = {"current": 1, "supported": [1]}
        manifest.write_text(json.dumps(document), encoding="utf-8")
        with self.assertRaisesRegex(MODULE.ReleaseError, "ABI 1, contract 2"):
            MODULE.validate_sdk_manifest(
                self.package,
                native_archive,
                nuget_sbom,
                native_sbom,
                self.version,
                manifest,
            )

    def native_entry_contents(self, name: str) -> bytes:
        if name in MODULE.LICENSE_ENTRIES:
            return (MODULE.REPOSITORY_ROOT / name).read_bytes()
        if name == MODULE.THIRD_PARTY_LICENSES_ENTRY:
            return MODULE.native_cargo_sbom.third_party_license_report_bytes(
                MODULE.REPOSITORY_ROOT, (MODULE.WINDOWS_RUST_TARGET,)
            )
        return f"fixture {name}".encode()


if __name__ == "__main__":
    unittest.main()
