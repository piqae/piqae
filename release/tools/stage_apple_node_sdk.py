#!/usr/bin/env python3
"""Stage and validate the versioned Apple NodeKit release distribution."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import tempfile
import zipfile
from pathlib import Path, PurePosixPath

try:
    from release.tools import native_cargo_sbom
except ModuleNotFoundError:  # Direct script execution uses release/tools as sys.path[0].
    import native_cargo_sbom  # type: ignore[no-redef]


SEMVER = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$")
REPOSITORY = "piqae/piqae"
FIXED_ZIP_TIME = (1980, 1, 1, 0, 0, 0)
APPLE_RUST_TARGETS = (
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-apple-ios",
    "aarch64-apple-ios-sim",
    "x86_64-apple-ios",
)


class ReleaseError(RuntimeError):
    """A staged release asset does not satisfy the release contract."""


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def swiftpm_checksum(path: Path) -> str:
    result = subprocess.run(
        ["swift", "package", "compute-checksum", str(path)],
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


def package_manifest(version: str, artifact: str, checksum: str) -> str:
    url = f"https://github.com/{REPOSITORY}/releases/download/v{version}/{artifact}"
    return f'''// swift-tools-version: 5.10
import PackageDescription
import Foundation

// The local path is used only by the release gate after it has separately
// downloaded and verified the versioned XCFramework asset. Normal consumers
// resolve the immutable GitHub release URL and SwiftPM checksum below.
let localNativePath = ".artifacts/PiqaeNode.xcframework"
let packageRoot = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
let hasValidatedLocalArtifact = FileManager.default.fileExists(
    atPath: packageRoot.appendingPathComponent(localNativePath).path
)

let nativeTarget: Target = hasValidatedLocalArtifact
    ? .binaryTarget(name: "PiqaeNodeNative", path: localNativePath)
    : .binaryTarget(
        name: "PiqaeNodeNative",
        url: "{url}",
        checksum: "{checksum}"
    )

let package = Package(
    name: "PiqaeNodeKit",
    platforms: [
        .iOS(.v16),
        .macOS(.v13),
    ],
    products: [
        .library(name: "PiqaeNodeKit", targets: ["PiqaeNodeKit"]),
        .library(name: "PiqaeNodeKitAirPrint", targets: ["PiqaeNodeKitAirPrint"]),
        .library(name: "PiqaeNodeKitUI", targets: ["PiqaeNodeKitUI"]),
        .library(name: "PiqaeNodeKitTesting", targets: ["PiqaeNodeKitTesting"]),
    ],
    targets: [
        nativeTarget,
        .target(
            name: "CPiqaeNodeABI",
            dependencies: ["PiqaeNodeNative"],
            publicHeadersPath: "include",
            cSettings: [.define("PIQAE_NODE_HAS_NATIVE_ARTIFACT")]
        ),
        .target(name: "PiqaeNodeKit", dependencies: ["CPiqaeNodeABI"]),
        .target(name: "PiqaeNodeKitAirPrint", dependencies: ["PiqaeNodeKit"]),
        .target(name: "PiqaeNodeKitUI", dependencies: ["PiqaeNodeKit"]),
        .target(name: "PiqaeNodeKitTesting", dependencies: ["PiqaeNodeKit"]),
    ]
)
'''


def deterministic_zip(source: Path, output: Path) -> None:
    root = source.parent
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path in sorted(source.rglob("*"), key=lambda item: item.as_posix()):
            relative = path.relative_to(root).as_posix()
            if path.is_dir():
                info = zipfile.ZipInfo(f"{relative}/", FIXED_ZIP_TIME)
                info.external_attr = (0o755 | 0o040000) << 16
                archive.writestr(info, b"")
                continue
            info = zipfile.ZipInfo(relative, FIXED_ZIP_TIME)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = (0o644 | 0o100000) << 16
            archive.writestr(info, path.read_bytes())


def archive_files(path: Path) -> list[dict[str, str]]:
    files: list[dict[str, str]] = []
    names: set[str] = set()
    with zipfile.ZipFile(path) as archive:
        for raw_name in sorted(archive.namelist()):
            if raw_name.endswith("/"):
                continue
            if "\\" in raw_name:
                raise ReleaseError("Apple release archive contains a non-portable path")
            name = raw_name
            pure = PurePosixPath(name)
            if pure.is_absolute() or ".." in pure.parts:
                raise ReleaseError("Apple release archive contains an unsafe path")
            if name in names:
                raise ReleaseError("Apple release archive contains a duplicate path")
            names.add(name)
            contents = archive.read(raw_name)
            files.append(
                {
                    "name": name,
                    "sha1": hashlib.sha1(contents, usedforsecurity=False).hexdigest(),
                    "sha256": hashlib.sha256(contents).hexdigest(),
                }
            )
    if not files:
        raise ReleaseError("Apple release archive is empty")
    return files


def generate_archive_sbom(archive: Path, package_name: str, version: str, output: Path) -> None:
    files = archive_files(archive)
    spdx_files = []
    relationships = []
    for index, entry in enumerate(files):
        spdx_id = f"SPDXRef-File-{index}-{entry['sha256'][:16]}"
        spdx_files.append(
            {
                "fileName": f"./{entry['name']}",
                "SPDXID": spdx_id,
                "checksums": [
                    {"algorithm": "SHA1", "checksumValue": entry["sha1"]},
                    {"algorithm": "SHA256", "checksumValue": entry["sha256"]},
                ],
                "licenseConcluded": "NOASSERTION",
                "licenseInfoInFiles": ["NOASSERTION"],
                "copyrightText": "NOASSERTION",
            }
        )
        relationships.append(
            {
                "spdxElementId": "SPDXRef-Package",
                "relationshipType": "CONTAINS",
                "relatedSpdxElement": spdx_id,
            }
        )
    verification = hashlib.sha1(
        "".join(sorted(entry["sha1"] for entry in files)).encode("ascii"),
        usedforsecurity=False,
    ).hexdigest()
    document = {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": f"{package_name}-{version}",
        "documentNamespace": f"https://spdx.org/spdxdocs/{package_name.lower()}-{version}-{sha256(archive)[:20]}",
        "creationInfo": {
            "created": "1980-01-01T00:00:00Z",
            "creators": ["Tool: piqae-apple-node-sdk-release"],
        },
        "packages": [
            {
                "name": package_name,
                "SPDXID": "SPDXRef-Package",
                "versionInfo": version,
                "downloadLocation": "NOASSERTION",
                "filesAnalyzed": True,
                "packageVerificationCode": {"packageVerificationCodeValue": verification},
                "checksums": [{"algorithm": "SHA256", "checksumValue": sha256(archive)}],
                "licenseConcluded": "NOASSERTION",
                "licenseDeclared": "Apache-2.0",
                "copyrightText": "NOASSERTION",
            }
        ],
        "files": spdx_files,
        "relationships": [
            {
                "spdxElementId": "SPDXRef-DOCUMENT",
                "relationshipType": "DESCRIBES",
                "relatedSpdxElement": "SPDXRef-Package",
            }
        ]
        + relationships,
    }
    output.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def validate_archive_sbom(
    archive: Path, sbom: Path, required_license_paths: set[str]
) -> None:
    files = archive_files(archive)
    names = {entry["name"] for entry in files}
    if not required_license_paths.issubset(names):
        raise ReleaseError("Apple release archive is missing LICENSE or NOTICE")
    document = json.loads(sbom.read_text(encoding="utf-8"))
    if document.get("spdxVersion") != "SPDX-2.3":
        raise ReleaseError("Apple archive SBOM must be SPDX 2.3 JSON")
    spdx_files = {
        entry.get("fileName", "").removeprefix("./"): entry
        for entry in document.get("files", [])
    }
    if set(spdx_files) != names:
        raise ReleaseError("Apple archive SBOM does not cover every archive file")
    contained = {
        relationship.get("relatedSpdxElement")
        for relationship in document.get("relationships", [])
        if relationship.get("spdxElementId") == "SPDXRef-Package"
        and relationship.get("relationshipType") == "CONTAINS"
    }
    file_ids = {entry.get("SPDXID") for entry in spdx_files.values()}
    if len(file_ids) != len(files) or None in file_ids or contained != file_ids:
        raise ReleaseError("Apple archive SBOM package containment is incomplete")
    sha1_values = []
    for entry in files:
        checksums = {
            value.get("algorithm"): value.get("checksumValue")
            for value in spdx_files[entry["name"]].get("checksums", [])
        }
        if checksums.get("SHA256") != entry["sha256"]:
            raise ReleaseError("Apple archive SBOM file checksum is inconsistent")
        if checksums.get("SHA1") != entry["sha1"]:
            raise ReleaseError("Apple archive SBOM file checksum is inconsistent")
        sha1_values.append(entry["sha1"])
    packages = document.get("packages", [])
    if len(packages) != 1 or packages[0].get("checksums") != [
        {"algorithm": "SHA256", "checksumValue": sha256(archive)}
    ]:
        raise ReleaseError("Apple archive SBOM package checksum is inconsistent")
    if packages[0].get("licenseConcluded") != "NOASSERTION" or any(
        entry.get("licenseConcluded") != "NOASSERTION" for entry in spdx_files.values()
    ):
        raise ReleaseError("Apple archive mixed-license conclusions must be NOASSERTION")
    verification = hashlib.sha1(
        "".join(sorted(sha1_values)).encode("ascii"), usedforsecurity=False
    ).hexdigest()
    if packages[0].get("packageVerificationCode", {}).get(
        "packageVerificationCodeValue"
    ) != verification:
        raise ReleaseError("Apple archive SBOM package verification code is inconsistent")


def exact_git_revision(repository_root: Path) -> str:
    return subprocess.run(
        ["git", "-C", str(repository_root), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def validate_version(version: str) -> None:
    if not SEMVER.fullmatch(version):
        raise ReleaseError("version must be a safe SemVer without a leading v")


def stage(repository_root: Path, version: str, output: Path) -> None:
    validate_version(version)
    source = repository_root / "sdk/apple/.artifacts"
    source_archive = source / "PiqaeNode.xcframework.zip"
    source_manifest = source / "PiqaeNode.artifact.json"
    if not source_archive.is_file() or not source_manifest.is_file():
        raise ReleaseError("build the Apple XCFramework and source manifest before staging")

    metadata = json.loads(source_manifest.read_text(encoding="utf-8"))
    revision = exact_git_revision(repository_root)
    archive_digest = sha256(source_archive)
    archive_checksum = swiftpm_checksum(source_archive)
    if metadata.get("schema") != 1:
        raise ReleaseError("source Apple artifact manifest schema must be 1")
    if metadata.get("native_abi") != 1 or metadata.get("native_contract") != {
        "current": 2,
        "supported": [2],
    }:
        raise ReleaseError("source Apple artifact must require native ABI 1 and contract 2")
    if metadata.get("capability_command") != "print_packet_capabilities" or metadata.get(
        "capability_contract"
    ) != "printpacket/v1":
        raise ReleaseError("source Apple artifact must expose PrintPacket capabilities")
    if metadata.get("rust_targets") != list(APPLE_RUST_TARGETS):
        raise ReleaseError("source Apple artifact must record all five locked Rust target graphs")
    if metadata.get("artifact") != source_archive.name:
        raise ReleaseError("source Apple manifest does not name the built archive")
    if metadata.get("git_revision") != revision:
        raise ReleaseError("source Apple artifact was not built from the checked-out revision")
    if metadata.get("sha256") != archive_digest:
        raise ReleaseError("source Apple artifact SHA-256 does not match its archive")
    if metadata.get("swiftpm_checksum") != archive_checksum:
        raise ReleaseError("source Apple artifact SwiftPM checksum does not match its archive")

    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        raise ReleaseError("Apple SDK staging directory must be empty")

    versioned_archive = f"PiqaeNode.xcframework-{version}.zip"
    staged_archive = output / versioned_archive
    shutil.copyfile(source_archive, staged_archive)
    (output / f"{versioned_archive}.sha256").write_text(
        f"{archive_digest}  {versioned_archive}\n", encoding="ascii"
    )

    package_name = f"PiqaeNodeKit-{version}"
    package_archive_name = f"{package_name}.zip"
    package_archive = output / package_archive_name
    with tempfile.TemporaryDirectory(prefix="piqae-nodekit-release.") as scratch:
        package_root = Path(scratch) / package_name
        shutil.copytree(repository_root / "sdk/apple/Sources", package_root / "Sources")
        shutil.copyfile(repository_root / "sdk/apple/README.md", package_root / "README.md")
        shutil.copyfile(repository_root / "LICENSE", package_root / "LICENSE")
        shutil.copyfile(repository_root / "NOTICE", package_root / "NOTICE")
        native_cargo_sbom.write_third_party_license_report(
            package_root / native_cargo_sbom.THIRD_PARTY_LICENSES_FILENAME,
            repository_root,
            APPLE_RUST_TARGETS,
        )
        shutil.copyfile(
            repository_root / "sdk/native/include/piqae_node.h",
            package_root / "Sources/CPiqaeNodeABI/include/piqae_node.h",
        )
        shim = package_root / "Sources/CPiqaeNodeABI/include/shim.h"
        shim_text = shim.read_text(encoding="utf-8")
        expected_include = '#include "../../../../native/include/piqae_node.h"'
        if expected_include not in shim_text:
            raise ReleaseError("Apple C shim no longer has the expected repository-relative include")
        shim.write_text(
            shim_text.replace(expected_include, '#include "piqae_node.h"', 1),
            encoding="utf-8",
        )
        (package_root / "Package.swift").write_text(
            package_manifest(version, versioned_archive, archive_checksum), encoding="utf-8"
        )
        deterministic_zip(package_root, package_archive)

    package_digest = sha256(package_archive)
    (output / f"{package_archive_name}.sha256").write_text(
        f"{package_digest}  {package_archive_name}\n", encoding="ascii"
    )
    native_sbom_name = f"PiqaeNode.xcframework-{version}.spdx.json"
    source_sbom_name = f"PiqaeNodeKit-{version}.spdx.json"
    try:
        native_cargo_sbom.generate_native_sbom(
            staged_archive,
            output / native_sbom_name,
            repository_root,
            APPLE_RUST_TARGETS,
            "PiqaeNodeNative",
            version,
            (".a",),
        )
    except native_cargo_sbom.NativeCargoSbomError as error:
        raise ReleaseError(str(error)) from error
    generate_archive_sbom(package_archive, "PiqaeNodeKit", version, output / source_sbom_name)
    release_manifest = {
        "schema": 2,
        "native_abi": 1,
        "native_contract": {"current": 2, "supported": [2]},
        "capability_command": "print_packet_capabilities",
        "capability_contract": "printpacket/v1",
        "rust_targets": list(APPLE_RUST_TARGETS),
        "version": version,
        "tag": f"v{version}",
        "git_revision": revision,
        "artifact": versioned_archive,
        "download_url": f"https://github.com/{REPOSITORY}/releases/download/v{version}/{versioned_archive}",
        "swiftpm_checksum": archive_checksum,
        "sha256": archive_digest,
        "source_package": package_archive_name,
        "source_package_download_url": f"https://github.com/{REPOSITORY}/releases/download/v{version}/{package_archive_name}",
        "source_package_sha256": package_digest,
        "sboms": {
            versioned_archive: native_sbom_name,
            package_archive_name: source_sbom_name,
        },
        "slices": metadata.get("slices"),
    }
    (output / "PiqaeNode.artifact.json").write_text(
        json.dumps(release_manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    validate_stage(repository_root, version, output)


def validate_stage(repository_root: Path, version: str, output: Path) -> dict[str, object]:
    validate_version(version)
    manifest_path = output / "PiqaeNode.artifact.json"
    if not manifest_path.is_file():
        raise ReleaseError("versioned Apple release manifest is missing")
    metadata = json.loads(manifest_path.read_text(encoding="utf-8"))
    versioned_archive = f"PiqaeNode.xcframework-{version}.zip"
    source_package = f"PiqaeNodeKit-{version}.zip"
    expected = {
        "schema": 2,
        "native_abi": 1,
        "native_contract": {"current": 2, "supported": [2]},
        "capability_command": "print_packet_capabilities",
        "capability_contract": "printpacket/v1",
        "rust_targets": list(APPLE_RUST_TARGETS),
        "version": version,
        "tag": f"v{version}",
        "git_revision": exact_git_revision(repository_root),
        "artifact": versioned_archive,
        "download_url": f"https://github.com/{REPOSITORY}/releases/download/v{version}/{versioned_archive}",
        "source_package": source_package,
        "source_package_download_url": f"https://github.com/{REPOSITORY}/releases/download/v{version}/{source_package}",
        "sboms": {
            versioned_archive: f"PiqaeNode.xcframework-{version}.spdx.json",
            source_package: f"PiqaeNodeKit-{version}.spdx.json",
        },
    }
    for key, value in expected.items():
        if metadata.get(key) != value:
            raise ReleaseError(f"Apple release manifest field {key!r} does not match the staged release")

    archive = output / versioned_archive
    package = output / source_package
    if not archive.is_file() or not package.is_file():
        raise ReleaseError("Apple release manifest references a missing staged asset")
    archive_digest = sha256(archive)
    package_digest = sha256(package)
    if metadata.get("sha256") != archive_digest:
        raise ReleaseError("versioned Apple archive SHA-256 does not match the release manifest")
    if metadata.get("swiftpm_checksum") != swiftpm_checksum(archive):
        raise ReleaseError("versioned Apple archive SwiftPM checksum does not match the release manifest")
    if metadata.get("source_package_sha256") != package_digest:
        raise ReleaseError("Apple source package SHA-256 does not match the release manifest")

    expected_checksums = {
        output / f"{versioned_archive}.sha256": f"{archive_digest}  {versioned_archive}\n",
        output / f"{source_package}.sha256": f"{package_digest}  {source_package}\n",
    }
    for path, contents in expected_checksums.items():
        if path.read_text(encoding="ascii") != contents:
            raise ReleaseError(f"checksum sidecar {path.name!r} does not match its staged asset")

    with zipfile.ZipFile(package) as package_zip:
        names = set(package_zip.namelist())
        root = f"PiqaeNodeKit-{version}"
        manifest_name = f"{root}/Package.swift"
        required = {
            manifest_name,
            f"{root}/LICENSE",
            f"{root}/NOTICE",
            f"{root}/{native_cargo_sbom.THIRD_PARTY_LICENSES_FILENAME}",
            f"{root}/Sources/CPiqaeNodeABI/include/piqae_node.h",
            f"{root}/Sources/CPiqaeNodeABI/include/shim.h",
            f"{root}/Sources/PiqaeNodeKit/PiqaeNode.swift",
        }
        if not required.issubset(names):
            raise ReleaseError("Apple source package is missing required package sources")
        package_swift = package_zip.read(manifest_name).decode("utf-8")
        if metadata["download_url"] not in package_swift:
            raise ReleaseError("release Package.swift does not reference the staged artifact URL")
        if metadata["swiftpm_checksum"] not in package_swift:
            raise ReleaseError("release Package.swift does not reference the staged SwiftPM checksum")
        shim = package_zip.read(f"{root}/Sources/CPiqaeNodeABI/include/shim.h").decode("utf-8")
        if '#include "piqae_node.h"' not in shim or "../../../../native" in shim:
            raise ReleaseError("release package C shim is not independent of the repository layout")
    with zipfile.ZipFile(archive) as native_zip:
        native_names = set(native_zip.namelist())
        if not {
            "PiqaeNode.xcframework/LICENSE",
            "PiqaeNode.xcframework/NOTICE",
            f"PiqaeNode.xcframework/{native_cargo_sbom.THIRD_PARTY_LICENSES_FILENAME}",
        }.issubset(native_names):
            raise ReleaseError("Apple native archive is missing licence evidence")
        if native_zip.read("PiqaeNode.xcframework/LICENSE") != (
            repository_root / "LICENSE"
        ).read_bytes() or native_zip.read("PiqaeNode.xcframework/NOTICE") != (
            repository_root / "NOTICE"
        ).read_bytes():
            raise ReleaseError("Apple native archive LICENSE or NOTICE does not match the repository")
        try:
            native_cargo_sbom.validate_third_party_license_report(
                native_zip.read(
                    f"PiqaeNode.xcframework/{native_cargo_sbom.THIRD_PARTY_LICENSES_FILENAME}"
                ),
                repository_root,
                APPLE_RUST_TARGETS,
            )
        except native_cargo_sbom.NativeCargoSbomError as error:
            raise ReleaseError(str(error)) from error
    with zipfile.ZipFile(package) as package_zip:
        package_root = f"PiqaeNodeKit-{version}"
        if package_zip.read(f"{package_root}/LICENSE") != (
            repository_root / "LICENSE"
        ).read_bytes() or package_zip.read(f"{package_root}/NOTICE") != (
            repository_root / "NOTICE"
        ).read_bytes():
            raise ReleaseError("Apple source archive LICENSE or NOTICE does not match the repository")
        try:
            native_cargo_sbom.validate_third_party_license_report(
                package_zip.read(
                    f"{package_root}/{native_cargo_sbom.THIRD_PARTY_LICENSES_FILENAME}"
                ),
                repository_root,
                APPLE_RUST_TARGETS,
            )
        except native_cargo_sbom.NativeCargoSbomError as error:
            raise ReleaseError(str(error)) from error
    try:
        native_cargo_sbom.validate_native_sbom(
            archive,
            output / metadata["sboms"][versioned_archive],
            repository_root,
            APPLE_RUST_TARGETS,
            "PiqaeNodeNative",
            version,
            (".a",),
        )
    except native_cargo_sbom.NativeCargoSbomError as error:
        raise ReleaseError(str(error)) from error
    validate_archive_sbom(
        package,
        output / metadata["sboms"][source_package],
        {f"PiqaeNodeKit-{version}/LICENSE", f"PiqaeNodeKit-{version}/NOTICE"},
    )
    return metadata


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("stage", "validate"))
    parser.add_argument("--repository-root", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    repository_root = args.repository_root.resolve()
    output = args.output.resolve()
    try:
        if args.command == "stage":
            stage(repository_root, args.version, output)
        else:
            validate_stage(repository_root, args.version, output)
    except (
        OSError,
        KeyError,
        ValueError,
        subprocess.CalledProcessError,
        ReleaseError,
        native_cargo_sbom.NativeCargoSbomError,
    ) as error:
        parser.error(str(error))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
