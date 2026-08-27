#!/usr/bin/env python3
"""Validate the staged Windows Node SDK NuGet and generate its SPDX SBOM."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import zipfile
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
from xml.etree import ElementTree


SEMVER = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$")
DEPENDENCY_ID = "BouncyCastle.Cryptography"
DEPENDENCY_VERSION = "2.6.2"
MANAGED_ENTRY = "lib/net8.0/Piqae.Node.dll"
NATIVE_ENTRY = "runtimes/win-x64/native/piqae_node_ffi.dll"
LICENSE_ENTRIES = {"LICENSE", "NOTICE"}
NATIVE_ARCHIVE_ENTRIES = {
    "LICENSE",
    "NOTICE",
    "README.md",
    "piqae_node.h",
    "piqae_node_ffi.dll",
    "piqae_node_ffi.dll.lib",
}
REPOSITORY_ROOT = Path(__file__).resolve().parents[2]


class ReleaseError(RuntimeError):
    """A Windows SDK release asset violates the package contract."""


def digest(data: bytes, algorithm: str) -> str:
    return hashlib.new(algorithm, data).hexdigest()


def file_digest(path: Path, algorithm: str = "sha256") -> str:
    value = hashlib.new(algorithm)
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def local_name(element: ElementTree.Element) -> str:
    return element.tag.rsplit("}", 1)[-1]


def child_text(parent: ElementTree.Element, name: str) -> str:
    for element in parent.iter():
        if local_name(element) == name and element.text:
            return element.text.strip()
    raise ReleaseError(f"NuGet metadata field {name!r} is missing")


def nuspec(archive: zipfile.ZipFile) -> ElementTree.Element:
    candidates = [name for name in archive.namelist() if name.lower().endswith(".nuspec")]
    if len(candidates) != 1:
        raise ReleaseError("NuGet package must contain exactly one .nuspec")
    try:
        return ElementTree.fromstring(archive.read(candidates[0]))
    except ElementTree.ParseError as error:
        raise ReleaseError("NuGet package contains invalid .nuspec XML") from error


def safe_entries(archive: zipfile.ZipFile) -> set[str]:
    names: set[str] = set()
    for raw_name in archive.namelist():
        if "\\" in raw_name:
            raise ReleaseError("release archive contains a non-portable path separator")
        name = raw_name
        path = PurePosixPath(name)
        if path.is_absolute() or ".." in path.parts:
            raise ReleaseError("release archive contains an unsafe archive path")
        if name in names:
            raise ReleaseError("release archive contains a duplicate path")
        names.add(name)
    return names


def validate_package(package: Path, dependency_package: Path, version: str) -> dict[str, object]:
    if not SEMVER.fullmatch(version):
        raise ReleaseError("version must be a safe SemVer without a leading v")
    if package.name != f"Piqae.Node.{version}.nupkg":
        raise ReleaseError("Piqae.Node NuGet filename does not match the release version")
    if dependency_package.name.lower() != f"bouncycastle.cryptography.{DEPENDENCY_VERSION}.nupkg".lower():
        raise ReleaseError("BouncyCastle dependency filename does not match the pinned version")

    with zipfile.ZipFile(package) as archive:
        entries = safe_entries(archive)
        metadata = nuspec(archive)
        if child_text(metadata, "id") != "Piqae.Node":
            raise ReleaseError("staged NuGet package ID is not Piqae.Node")
        if child_text(metadata, "version") != version:
            raise ReleaseError("staged NuGet package version does not match the release")
        required = {MANAGED_ENTRY, NATIVE_ENTRY} | LICENSE_ENTRIES
        if not required.issubset(entries):
            raise ReleaseError("staged NuGet is missing the managed facade or win-x64 runtime")
        if archive.read("LICENSE") != (REPOSITORY_ROOT / "LICENSE").read_bytes() or archive.read(
            "NOTICE"
        ) != (REPOSITORY_ROOT / "NOTICE").read_bytes():
            raise ReleaseError("staged NuGet LICENSE or NOTICE does not match the repository")
        runtime_entries = sorted(name for name in entries if name.startswith("runtimes/"))
        if runtime_entries != [NATIVE_ENTRY]:
            raise ReleaseError("staged NuGet contains an unexpected or incomplete RID runtime set")
        dependencies = [
            element
            for element in metadata.iter()
            if local_name(element) == "dependency" and element.attrib.get("id") == DEPENDENCY_ID
        ]
        if len(dependencies) != 1 or dependencies[0].attrib.get("version") != f"[{DEPENDENCY_VERSION}]":
            raise ReleaseError("Piqae.Node must pin the audited BouncyCastle dependency exactly")
        archive_files = []
        for entry in sorted(name for name in entries if not name.endswith("/")):
            contents = archive.read(entry)
            archive_files.append(
                {
                    "name": entry,
                    "sha1": digest(contents, "sha1"),
                    "sha256": digest(contents, "sha256"),
                }
            )
        managed = archive.read(MANAGED_ENTRY)
        native = archive.read(NATIVE_ENTRY)

    with zipfile.ZipFile(dependency_package) as dependency_archive:
        safe_entries(dependency_archive)
        dependency_metadata = nuspec(dependency_archive)
        if child_text(dependency_metadata, "id") != DEPENDENCY_ID:
            raise ReleaseError("dependency package ID is not BouncyCastle.Cryptography")
        if child_text(dependency_metadata, "version") != DEPENDENCY_VERSION:
            raise ReleaseError("dependency package version does not match the audited pin")

    return {
        "version": version,
        "package_sha256": file_digest(package),
        "dependency_sha256": file_digest(dependency_package),
        "managed_sha1": digest(managed, "sha1"),
        "managed_sha256": digest(managed, "sha256"),
        "native_sha1": digest(native, "sha1"),
        "native_sha256": digest(native, "sha256"),
        "archive_files": archive_files,
    }


def validate_native_archive(archive_path: Path, version: str) -> list[dict[str, str]]:
    expected_name = f"PiqaeNode-native-windows-x64-{version}.zip"
    if archive_path.name != expected_name:
        raise ReleaseError("Windows native SDK archive filename does not match the release version")
    with zipfile.ZipFile(archive_path) as archive:
        entries = safe_entries(archive)
        files = {name for name in entries if not name.endswith("/")}
        if files != NATIVE_ARCHIVE_ENTRIES:
            raise ReleaseError("Windows native SDK archive contents are incomplete or unexpected")
        if archive.read("LICENSE") != (REPOSITORY_ROOT / "LICENSE").read_bytes() or archive.read(
            "NOTICE"
        ) != (REPOSITORY_ROOT / "NOTICE").read_bytes():
            raise ReleaseError("Windows native SDK archive LICENSE or NOTICE does not match the repository")
        return [
            {
                "name": name,
                "sha1": digest(archive.read(name), "sha1"),
                "sha256": digest(archive.read(name), "sha256"),
            }
            for name in sorted(files)
        ]


def generate_native_sbom(archive: Path, version: str, output: Path) -> None:
    files = validate_native_archive(archive, version)
    spdx_files = []
    relationships = []
    for index, entry in enumerate(files):
        spdx_id = f"SPDXRef-File-Native-{index}-{entry['sha256'][:16]}"
        spdx_files.append(
            {
                "fileName": f"./{entry['name']}",
                "SPDXID": spdx_id,
                "checksums": [
                    {"algorithm": "SHA1", "checksumValue": entry["sha1"]},
                    {"algorithm": "SHA256", "checksumValue": entry["sha256"]},
                ],
                "licenseConcluded": "Apache-2.0",
                "licenseInfoInFiles": ["NOASSERTION"],
                "copyrightText": "NOASSERTION",
            }
        )
        relationships.append(
            {
                "spdxElementId": "SPDXRef-Package-PiqaeNodeNativeBundle",
                "relationshipType": "CONTAINS",
                "relatedSpdxElement": spdx_id,
            }
        )
    verification = digest(
        "".join(sorted(entry["sha1"] for entry in files)).encode("ascii"), "sha1"
    )
    document = {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": f"PiqaeNode-native-windows-x64-{version}",
        "documentNamespace": f"https://spdx.org/spdxdocs/piqae-node-native-{version}-{file_digest(archive)[:20]}",
        "creationInfo": {
            "created": datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
            "creators": ["Tool: piqae-windows-sdk-release"],
        },
        "packages": [
            {
                "name": "piqae-node-ffi-native-bundle",
                "SPDXID": "SPDXRef-Package-PiqaeNodeNativeBundle",
                "versionInfo": version,
                "downloadLocation": "NOASSERTION",
                "filesAnalyzed": True,
                "packageVerificationCode": {"packageVerificationCodeValue": verification},
                "checksums": [{"algorithm": "SHA256", "checksumValue": file_digest(archive)}],
                "licenseConcluded": "Apache-2.0",
                "licenseDeclared": "Apache-2.0",
                "copyrightText": "NOASSERTION",
            }
        ],
        "files": spdx_files,
        "relationships": [
            {
                "spdxElementId": "SPDXRef-DOCUMENT",
                "relationshipType": "DESCRIBES",
                "relatedSpdxElement": "SPDXRef-Package-PiqaeNodeNativeBundle",
            }
        ]
        + relationships,
    }
    output.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def validate_native_sbom(archive: Path, sbom: Path, version: str) -> None:
    evidence = validate_native_archive(archive, version)
    document = json.loads(sbom.read_text(encoding="utf-8"))
    if document.get("spdxVersion") != "SPDX-2.3":
        raise ReleaseError("Windows native SDK SBOM must be SPDX 2.3 JSON")
    spdx_files = {
        entry.get("fileName", "").removeprefix("./"): entry
        for entry in document.get("files", [])
    }
    if set(spdx_files) != {entry["name"] for entry in evidence}:
        raise ReleaseError("Windows native SDK SBOM does not cover every archive file")
    contained = {
        relationship.get("relatedSpdxElement")
        for relationship in document.get("relationships", [])
        if relationship.get("spdxElementId") == "SPDXRef-Package-PiqaeNodeNativeBundle"
        and relationship.get("relationshipType") == "CONTAINS"
    }
    file_ids = {entry.get("SPDXID") for entry in spdx_files.values()}
    if len(file_ids) != len(evidence) or None in file_ids or contained != file_ids:
        raise ReleaseError("Windows native SDK SBOM package containment is incomplete")
    sha1_values = []
    for entry in evidence:
        checksums = {
            value.get("algorithm"): value.get("checksumValue")
            for value in spdx_files[entry["name"]].get("checksums", [])
        }
        if checksums.get("SHA256") != entry["sha256"]:
            raise ReleaseError("Windows native SDK SBOM file checksum is inconsistent")
        if checksums.get("SHA1") != entry["sha1"]:
            raise ReleaseError("Windows native SDK SBOM file checksum is inconsistent")
        sha1_values.append(entry["sha1"])
    packages = document.get("packages", [])
    if len(packages) != 1 or packages[0].get("checksums") != [
        {"algorithm": "SHA256", "checksumValue": file_digest(archive)}
    ]:
        raise ReleaseError("Windows native SDK SBOM package checksum is inconsistent")
    verification = digest("".join(sorted(sha1_values)).encode("ascii"), "sha1")
    if packages[0].get("packageVerificationCode", {}).get(
        "packageVerificationCodeValue"
    ) != verification:
        raise ReleaseError("Windows native SDK SBOM package verification code is inconsistent")


def generate_sdk_manifest(
    package: Path,
    native_archive: Path,
    nuget_sbom: Path,
    native_sbom: Path,
    version: str,
    output: Path,
) -> None:
    document = {
        "schema": 1,
        "version": version,
        "native_abi": 1,
        "native_contract": {"current": 2, "supported": [2]},
        "capability_command": "print_packet_capabilities",
        "capability_contract": "printpacket/v1",
        "artifacts": {
            package.name: {"sha256": file_digest(package), "sbom": nuget_sbom.name},
            native_archive.name: {
                "sha256": file_digest(native_archive),
                "sbom": native_sbom.name,
            },
        },
    }
    output.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    validate_sdk_manifest(package, native_archive, nuget_sbom, native_sbom, version, output)


def validate_sdk_manifest(
    package: Path,
    native_archive: Path,
    nuget_sbom: Path,
    native_sbom: Path,
    version: str,
    manifest: Path,
) -> None:
    if package.name != f"Piqae.Node.{version}.nupkg":
        raise ReleaseError("Windows SDK manifest package filename does not match the release")
    validate_sbom(nuget_sbom, package, version)
    validate_native_sbom(native_archive, native_sbom, version)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    expected = {
        "schema": 1,
        "version": version,
        "native_abi": 1,
        "native_contract": {"current": 2, "supported": [2]},
        "capability_command": "print_packet_capabilities",
        "capability_contract": "printpacket/v1",
        "artifacts": {
            package.name: {"sha256": file_digest(package), "sbom": nuget_sbom.name},
            native_archive.name: {
                "sha256": file_digest(native_archive),
                "sbom": native_sbom.name,
            },
        },
    }
    if document != expected:
        raise ReleaseError("Windows SDK manifest does not match ABI 1, contract 2, and staged artifacts")
    nuget_document = json.loads(nuget_sbom.read_text(encoding="utf-8"))
    piqae_package = next(
        (
            value
            for value in nuget_document.get("packages", [])
            if value.get("SPDXID") == "SPDXRef-Package-PiqaeNode"
        ),
        None,
    )
    package_checksums = {
        value.get("algorithm"): value.get("checksumValue")
        for value in (piqae_package or {}).get("checksums", [])
    }
    if package_checksums.get("SHA256") != file_digest(package):
        raise ReleaseError("Windows NuGet SBOM is not bound to the staged package")


def verification_code(file_sha1: str) -> str:
    return digest(file_sha1.encode("ascii"), "sha1")


def generate_sbom(package: Path, dependency_package: Path, version: str, output: Path) -> None:
    evidence = validate_package(package, dependency_package, version)
    namespace_suffix = str(evidence["package_sha256"])[:20]
    archive_files = evidence["archive_files"]
    if not isinstance(archive_files, list):
        raise ReleaseError("NuGet archive evidence is invalid")
    spdx_files = []
    archive_file_ids: dict[str, str] = {}
    used_ids: set[str] = set()
    for index, entry in enumerate(archive_files):
        if not isinstance(entry, dict):
            raise ReleaseError("NuGet archive file evidence is invalid")
        name = str(entry["name"])
        if name == MANAGED_ENTRY:
            spdx_id = "SPDXRef-File-ManagedFacade"
            license_concluded = "Apache-2.0"
        elif name == NATIVE_ENTRY:
            spdx_id = "SPDXRef-File-NativeRuntime"
            license_concluded = "Apache-2.0"
        else:
            spdx_id = f"SPDXRef-File-NuGet-{index}-{str(entry['sha256'])[:16]}"
            license_concluded = "NOASSERTION"
        if spdx_id in used_ids:
            raise ReleaseError("NuGet archive produced a duplicate SPDX file identifier")
        used_ids.add(spdx_id)
        archive_file_ids[name] = spdx_id
        spdx_files.append(
            {
                "fileName": f"./{name}",
                "SPDXID": spdx_id,
                "checksums": [
                    {"algorithm": "SHA1", "checksumValue": entry["sha1"]},
                    {"algorithm": "SHA256", "checksumValue": entry["sha256"]},
                ],
                "licenseConcluded": license_concluded,
                "licenseInfoInFiles": ["NOASSERTION"],
                "copyrightText": "NOASSERTION",
            }
        )
    package_verification = digest(
        "".join(sorted(str(entry["sha1"]) for entry in archive_files)).encode("ascii"), "sha1"
    )
    document = {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": f"Piqae.Node-{version}-windows-x64",
        "documentNamespace": f"https://spdx.org/spdxdocs/piqae-node-sdk-{version}-{namespace_suffix}",
        "creationInfo": {
            "created": datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
            "creators": ["Tool: piqae-windows-sdk-release"],
        },
        "packages": [
            {
                "name": "Piqae.Node",
                "SPDXID": "SPDXRef-Package-PiqaeNode",
                "versionInfo": version,
                "downloadLocation": "NOASSERTION",
                "filesAnalyzed": True,
                "packageVerificationCode": {
                    "packageVerificationCodeValue": package_verification
                },
                "checksums": [{"algorithm": "SHA256", "checksumValue": evidence["package_sha256"]}],
                "licenseConcluded": "Apache-2.0",
                "licenseDeclared": "Apache-2.0",
                "copyrightText": "NOASSERTION",
                "externalRefs": [
                    {
                        "referenceCategory": "PACKAGE-MANAGER",
                        "referenceType": "purl",
                        "referenceLocator": f"pkg:nuget/Piqae.Node@{version}",
                    }
                ],
            },
            {
                "name": DEPENDENCY_ID,
                "SPDXID": "SPDXRef-Package-BouncyCastle",
                "versionInfo": DEPENDENCY_VERSION,
                "downloadLocation": f"https://www.nuget.org/packages/{DEPENDENCY_ID}/{DEPENDENCY_VERSION}",
                "filesAnalyzed": False,
                "checksums": [{"algorithm": "SHA256", "checksumValue": evidence["dependency_sha256"]}],
                "licenseConcluded": "NOASSERTION",
                "licenseDeclared": "MIT",
                "copyrightText": "NOASSERTION",
                "externalRefs": [
                    {
                        "referenceCategory": "PACKAGE-MANAGER",
                        "referenceType": "purl",
                        "referenceLocator": f"pkg:nuget/{DEPENDENCY_ID}@{DEPENDENCY_VERSION}",
                    }
                ],
            },
            {
                "name": "piqae-node-ffi",
                "SPDXID": "SPDXRef-Package-PiqaeNodeNative",
                "versionInfo": version,
                "downloadLocation": "NOASSERTION",
                "filesAnalyzed": True,
                "packageVerificationCode": {
                    "packageVerificationCodeValue": verification_code(str(evidence["native_sha1"]))
                },
                "checksums": [{"algorithm": "SHA256", "checksumValue": evidence["native_sha256"]}],
                "licenseConcluded": "Apache-2.0",
                "licenseDeclared": "Apache-2.0",
                "copyrightText": "NOASSERTION",
            },
        ],
        "files": spdx_files,
        "relationships": [
            {"spdxElementId": "SPDXRef-DOCUMENT", "relationshipType": "DESCRIBES", "relatedSpdxElement": "SPDXRef-Package-PiqaeNode"},
            {"spdxElementId": "SPDXRef-DOCUMENT", "relationshipType": "DESCRIBES", "relatedSpdxElement": "SPDXRef-Package-BouncyCastle"},
            {"spdxElementId": "SPDXRef-DOCUMENT", "relationshipType": "DESCRIBES", "relatedSpdxElement": "SPDXRef-Package-PiqaeNodeNative"},
            {"spdxElementId": "SPDXRef-Package-PiqaeNode", "relationshipType": "DEPENDS_ON", "relatedSpdxElement": "SPDXRef-Package-BouncyCastle"},
            {"spdxElementId": "SPDXRef-Package-PiqaeNode", "relationshipType": "DEPENDS_ON", "relatedSpdxElement": "SPDXRef-Package-PiqaeNodeNative"},
            {"spdxElementId": "SPDXRef-Package-PiqaeNodeNative", "relationshipType": "CONTAINS", "relatedSpdxElement": "SPDXRef-File-NativeRuntime"},
        ]
        + [
            {
                "spdxElementId": "SPDXRef-Package-PiqaeNode",
                "relationshipType": "CONTAINS",
                "relatedSpdxElement": archive_file_ids[str(entry["name"])],
            }
            for entry in archive_files
        ],
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def validate_sbom(path: Path, package: Path, version: str) -> None:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("spdxVersion") != "SPDX-2.3":
        raise ReleaseError("Windows SDK SBOM must be SPDX 2.3 JSON")
    packages = {
        (package.get("name"), package.get("versionInfo")) for package in document.get("packages", [])
    }
    required_packages = {
        ("Piqae.Node", version),
        (DEPENDENCY_ID, DEPENDENCY_VERSION),
        ("piqae-node-ffi", version),
    }
    if not required_packages.issubset(packages):
        raise ReleaseError("Windows SDK SBOM omits the facade, dependency, or native runtime")
    with zipfile.ZipFile(package) as archive:
        archive_names = {
            name for name in safe_entries(archive) if not name.endswith("/")
        }
        archive_checksums = {
            name: {
                "SHA1": digest(archive.read(name), "sha1"),
                "SHA256": digest(archive.read(name), "sha256"),
            }
            for name in archive_names
        }
    document_files = document.get("files", [])
    if not isinstance(document_files, list):
        raise ReleaseError("Windows SDK SBOM files must be an array")
    files = {entry.get("fileName") for entry in document_files}
    if len(document_files) != len(archive_names) or files != {
        f"./{name}" for name in archive_names
    }:
        raise ReleaseError("Windows SDK SBOM does not cover every file in the NuGet package")
    if f"./{MANAGED_ENTRY}" not in files or f"./{NATIVE_ENTRY}" not in files:
        raise ReleaseError("Windows SDK SBOM omits managed or native packaged files")
    file_entries = {entry.get("SPDXID"): entry for entry in document_files if entry.get("SPDXID")}
    if len(file_entries) != len(document_files):
        raise ReleaseError("Windows SDK SBOM file identifiers must be present and unique")
    contained = {
        relationship.get("relatedSpdxElement")
        for relationship in document.get("relationships", [])
        if relationship.get("spdxElementId") == "SPDXRef-Package-PiqaeNode"
        and relationship.get("relationshipType") == "CONTAINS"
    }
    if contained != set(file_entries):
        raise ReleaseError("Windows SDK SBOM does not analyze every file in the NuGet package")
    sha1_values = []
    for spdx_id in contained:
        file_name = str(file_entries[spdx_id].get("fileName", "")).removeprefix("./")
        checksums = {
            checksum.get("algorithm"): checksum.get("checksumValue")
            for checksum in file_entries[spdx_id].get("checksums", [])
        }
        if checksums != archive_checksums.get(file_name):
            raise ReleaseError("Windows SDK SBOM NuGet file checksum is inconsistent")
        sha1_values.append(checksums["SHA1"])
    expected_verification = digest("".join(sorted(sha1_values)).encode("ascii"), "sha1")
    piqae_package = next(
        package for package in document["packages"] if package.get("SPDXID") == "SPDXRef-Package-PiqaeNode"
    )
    actual_verification = piqae_package.get("packageVerificationCode", {}).get(
        "packageVerificationCodeValue"
    )
    package_checksums = {
        checksum.get("algorithm"): checksum.get("checksumValue")
        for checksum in piqae_package.get("checksums", [])
    }
    if (
        not piqae_package.get("filesAnalyzed")
        or actual_verification != expected_verification
        or package_checksums.get("SHA256") != file_digest(package)
    ):
        raise ReleaseError("Windows SDK SBOM NuGet package verification code is inconsistent")


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    for name in ("validate-package", "generate-sbom"):
        command = subparsers.add_parser(name)
        command.add_argument("--package", type=Path, required=True)
        command.add_argument("--dependency-package", type=Path, required=True)
        command.add_argument("--version", required=True)
        if name == "generate-sbom":
            command.add_argument("--output", type=Path, required=True)
    validate = subparsers.add_parser("validate-sbom")
    validate.add_argument("--input", type=Path, required=True)
    validate.add_argument("--package", type=Path, required=True)
    validate.add_argument("--version", required=True)
    native_generate = subparsers.add_parser("generate-native-sbom")
    native_generate.add_argument("--archive", type=Path, required=True)
    native_generate.add_argument("--version", required=True)
    native_generate.add_argument("--output", type=Path, required=True)
    native_validate = subparsers.add_parser("validate-native-sbom")
    native_validate.add_argument("--archive", type=Path, required=True)
    native_validate.add_argument("--input", type=Path, required=True)
    native_validate.add_argument("--version", required=True)
    for name in ("generate-sdk-manifest", "validate-sdk-manifest"):
        manifest_command = subparsers.add_parser(name)
        manifest_command.add_argument("--package", type=Path, required=True)
        manifest_command.add_argument("--native-archive", type=Path, required=True)
        manifest_command.add_argument("--nuget-sbom", type=Path, required=True)
        manifest_command.add_argument("--native-sbom", type=Path, required=True)
        manifest_command.add_argument("--version", required=True)
        manifest_command.add_argument(
            "--output" if name == "generate-sdk-manifest" else "--input",
            type=Path,
            required=True,
        )
    args = parser.parse_args()
    try:
        if args.command == "validate-package":
            validate_package(args.package, args.dependency_package, args.version)
        elif args.command == "generate-sbom":
            generate_sbom(args.package, args.dependency_package, args.version, args.output)
            validate_sbom(args.output, args.package, args.version)
        elif args.command == "validate-sbom":
            validate_sbom(args.input, args.package, args.version)
        elif args.command == "generate-native-sbom":
            generate_native_sbom(args.archive, args.version, args.output)
            validate_native_sbom(args.archive, args.output, args.version)
        elif args.command == "validate-native-sbom":
            validate_native_sbom(args.archive, args.input, args.version)
        elif args.command == "generate-sdk-manifest":
            generate_sdk_manifest(
                args.package,
                args.native_archive,
                args.nuget_sbom,
                args.native_sbom,
                args.version,
                args.output,
            )
        else:
            validate_sdk_manifest(
                args.package,
                args.native_archive,
                args.nuget_sbom,
                args.native_sbom,
                args.version,
                args.input,
            )
    except (OSError, ValueError, KeyError, zipfile.BadZipFile, ReleaseError) as error:
        parser.error(str(error))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
