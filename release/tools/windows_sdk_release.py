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
        name = raw_name.replace("\\", "/")
        path = PurePosixPath(name)
        if path.is_absolute() or ".." in path.parts:
            raise ReleaseError("NuGet package contains an unsafe archive path")
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
        required = {MANAGED_ENTRY, NATIVE_ENTRY}
        if not required.issubset(entries):
            raise ReleaseError("staged NuGet is missing the managed facade or win-x64 runtime")
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


def validate_sbom(path: Path, version: str) -> None:
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
    files = {entry.get("fileName") for entry in document.get("files", [])}
    if f"./{MANAGED_ENTRY}" not in files or f"./{NATIVE_ENTRY}" not in files:
        raise ReleaseError("Windows SDK SBOM omits managed or native packaged files")
    file_entries = {
        entry.get("SPDXID"): entry for entry in document.get("files", []) if entry.get("SPDXID")
    }
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
        checksums = {
            checksum.get("algorithm"): checksum.get("checksumValue")
            for checksum in file_entries[spdx_id].get("checksums", [])
        }
        if not checksums.get("SHA1"):
            raise ReleaseError("Windows SDK SBOM NuGet file is missing its SHA1")
        sha1_values.append(checksums["SHA1"])
    expected_verification = digest("".join(sorted(sha1_values)).encode("ascii"), "sha1")
    piqae_package = next(
        package for package in document["packages"] if package.get("SPDXID") == "SPDXRef-Package-PiqaeNode"
    )
    actual_verification = piqae_package.get("packageVerificationCode", {}).get(
        "packageVerificationCodeValue"
    )
    if not piqae_package.get("filesAnalyzed") or actual_verification != expected_verification:
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
    validate.add_argument("--version", required=True)
    args = parser.parse_args()
    try:
        if args.command == "validate-package":
            validate_package(args.package, args.dependency_package, args.version)
        elif args.command == "generate-sbom":
            generate_sbom(args.package, args.dependency_package, args.version, args.output)
            validate_sbom(args.output, args.version)
        else:
            validate_sbom(args.input, args.version)
    except (OSError, ValueError, KeyError, zipfile.BadZipFile, ReleaseError) as error:
        parser.error(str(error))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
