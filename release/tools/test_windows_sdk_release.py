import importlib.util
import json
import tempfile
import unittest
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
    return f'''<?xml version="1.0"?><package><metadata><id>{package_id}</id><version>{version}</version>{dependency_xml}</metadata></package>'''


class WindowsSdkReleaseTests(unittest.TestCase):
    def setUp(self) -> None:
        self.fixture = tempfile.TemporaryDirectory()
        self.root = Path(self.fixture.name)
        self.version = "1.2.3"
        self.package = self.root / f"Piqae.Node.{self.version}.nupkg"
        self.dependency = self.root / "BouncyCastle.Cryptography.2.6.2.nupkg"
        self.write_package()
        with zipfile.ZipFile(self.dependency, "w") as archive:
            archive.writestr("BouncyCastle.Cryptography.nuspec", nuspec("BouncyCastle.Cryptography", "2.6.2"))

    def tearDown(self) -> None:
        self.fixture.cleanup()

    def write_package(self, dependency: str = "[2.6.2]", extra_runtime: bool = False) -> None:
        with zipfile.ZipFile(self.package, "w") as archive:
            archive.writestr("Piqae.Node.nuspec", nuspec("Piqae.Node", self.version, dependency))
            archive.writestr(MODULE.MANAGED_ENTRY, b"managed")
            archive.writestr(MODULE.NATIVE_ENTRY, b"native")
            if extra_runtime:
                archive.writestr("runtimes/win-arm64/native/piqae_node_ffi.dll", b"wrong")

    def test_generates_complete_spdx(self) -> None:
        output = self.root / "sdk.spdx.json"
        MODULE.generate_sbom(self.package, self.dependency, self.version, output)
        MODULE.validate_sbom(output, self.version)
        document = json.loads(output.read_text(encoding="utf-8"))
        names = {package["name"] for package in document["packages"]}
        self.assertEqual(names, {"Piqae.Node", "BouncyCastle.Cryptography", "piqae-node-ffi"})

    def test_rejects_floating_dependency(self) -> None:
        self.write_package(dependency="2.6.2")
        with self.assertRaisesRegex(MODULE.ReleaseError, "pin"):
            MODULE.validate_package(self.package, self.dependency, self.version)

    def test_rejects_unexpected_rid_asset(self) -> None:
        self.write_package(extra_runtime=True)
        with self.assertRaisesRegex(MODULE.ReleaseError, "RID"):
            MODULE.validate_package(self.package, self.dependency, self.version)

    def test_rejects_sbom_without_dependency(self) -> None:
        output = self.root / "sdk.spdx.json"
        MODULE.generate_sbom(self.package, self.dependency, self.version, output)
        document = json.loads(output.read_text(encoding="utf-8"))
        document["packages"] = [
            package for package in document["packages"] if package["name"] != "BouncyCastle.Cryptography"
        ]
        output.write_text(json.dumps(document), encoding="utf-8")
        with self.assertRaisesRegex(MODULE.ReleaseError, "omits"):
            MODULE.validate_sbom(output, self.version)

    def test_rejects_inconsistent_nuget_package_verification_code(self) -> None:
        output = self.root / "sdk.spdx.json"
        MODULE.generate_sbom(self.package, self.dependency, self.version, output)
        document = json.loads(output.read_text(encoding="utf-8"))
        piqae = next(package for package in document["packages"] if package["name"] == "Piqae.Node")
        piqae["packageVerificationCode"]["packageVerificationCodeValue"] = "0" * 40
        output.write_text(json.dumps(document), encoding="utf-8")
        with self.assertRaisesRegex(MODULE.ReleaseError, "verification code"):
            MODULE.validate_sbom(output, self.version)


if __name__ == "__main__":
    unittest.main()
