#!/usr/bin/env python3
"""Generate and verify target-specific SPDX evidence for a Rust native archive."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import tomllib
import zipfile
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path, PurePosixPath
from typing import Iterable
from urllib.parse import quote


class NativeCargoSbomError(RuntimeError):
    """The native artifact or locked Cargo evidence is incomplete."""


_SPDX_IDENTIFIER = re.compile(
    r"^(?:(?:DocumentRef-[A-Za-z0-9.-]+:)?LicenseRef-[A-Za-z0-9.-]+|"
    r"[A-Za-z0-9.-]+[+]?)$"
)
_LEGACY_SLASH_LICENSE = re.compile(
    r"^\s*([A-Za-z0-9.-]+[+]?)\s*/\s*([A-Za-z0-9.-]+[+]?)\s*$"
)
_RECOGNIZED_LEGACY_SLASH_PAIRS = {
    frozenset(("Apache-2.0", "MIT")),
    frozenset(("MIT", "Unlicense")),
}


def _spdx_expression_has_valid_shape(expression: str) -> bool:
    """Validate the SPDX expression grammar used by Cargo license metadata.

    Cargo manifests are expected to contain SPDX expressions, but older crates
    still publish the pre-SPDX slash shorthand. This deliberately validates the
    expression grammar rather than maintaining a second copy of the SPDX
    licence/exception registries.
    """

    tokens = re.findall(r"\(|\)|[^\s()]+", expression)
    if not tokens:
        return False
    position = 0

    def atom() -> bool:
        nonlocal position
        if position >= len(tokens):
            return False
        token = tokens[position]
        if token == "(":
            position += 1
            if not disjunction() or position >= len(tokens) or tokens[position] != ")":
                return False
            position += 1
            return True
        if token in {"AND", "OR", "WITH", ")", "("} or not _SPDX_IDENTIFIER.fullmatch(token):
            return False
        position += 1
        return True

    def with_expression() -> bool:
        nonlocal position
        if not atom():
            return False
        if position < len(tokens) and tokens[position] == "WITH":
            position += 1
            return atom()
        return True

    def conjunction() -> bool:
        nonlocal position
        if not with_expression():
            return False
        while position < len(tokens) and tokens[position] == "AND":
            position += 1
            if not with_expression():
                return False
        return True

    def disjunction() -> bool:
        nonlocal position
        if not conjunction():
            return False
        while position < len(tokens) and tokens[position] == "OR":
            position += 1
            if not conjunction():
                return False
        return True

    return disjunction() and position == len(tokens)


def _normalize_license_declared(value: object) -> str:
    if not isinstance(value, str) or not value.strip():
        return "NOASSERTION"
    expression = value.strip()
    legacy = _LEGACY_SLASH_LICENSE.fullmatch(expression)
    if legacy:
        if frozenset((legacy.group(1), legacy.group(2))) not in _RECOGNIZED_LEGACY_SLASH_PAIRS:
            return "NOASSERTION"
        expression = f"{legacy.group(1)} OR {legacy.group(2)}"
    if not _spdx_expression_has_valid_shape(expression):
        return "NOASSERTION"
    return expression


@dataclass(frozen=True)
class CargoPackage:
    key: str
    name: str
    version: str
    source: str
    purl: str
    checksum: str
    license_declared: str
    spdx_id: str
    package_root: Path
    targets: tuple[str, ...]
    workspace: bool
    license_source_info: str


@dataclass(frozen=True)
class CargoGraph:
    packages: tuple[CargoPackage, ...]
    relationships: tuple[tuple[str, str], ...]
    root_spdx_id: str
    targets: tuple[str, ...]


def _sha256_bytes(contents: bytes) -> str:
    return hashlib.sha256(contents).hexdigest()


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _safe_archive_files(archive_path: Path) -> list[dict[str, str]]:
    files: list[dict[str, str]] = []
    names: set[str] = set()
    with zipfile.ZipFile(archive_path) as archive:
        for raw_name in sorted(archive.namelist()):
            if raw_name.endswith("/"):
                continue
            if "\\" in raw_name:
                raise NativeCargoSbomError("native archive contains a non-portable path")
            path = PurePosixPath(raw_name)
            if path.is_absolute() or ".." in path.parts:
                raise NativeCargoSbomError("native archive contains an unsafe path")
            if raw_name in names:
                raise NativeCargoSbomError("native archive contains a duplicate path")
            names.add(raw_name)
            contents = archive.read(raw_name)
            files.append(
                {
                    "name": raw_name,
                    "sha1": hashlib.sha1(contents, usedforsecurity=False).hexdigest(),
                    "sha256": _sha256_bytes(contents),
                }
            )
    if not files:
        raise NativeCargoSbomError("native archive is empty")
    return files


@lru_cache(maxsize=8)
def _git_revision(repository_root: Path) -> str:
    return subprocess.run(
        ["git", "-C", str(repository_root), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def _workspace_source_checksum(repository_root: Path, package_root: Path) -> str:
    try:
        relative_root = package_root.resolve().relative_to(repository_root.resolve())
    except ValueError as error:
        raise NativeCargoSbomError("workspace Cargo package is outside the repository") from error
    result = subprocess.run(
        [
            "git",
            "-C",
            str(repository_root),
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            relative_root.as_posix(),
        ],
        check=True,
        capture_output=True,
    )
    paths = sorted(path for path in result.stdout.split(b"\0") if path)
    if not paths:
        raise NativeCargoSbomError("workspace Cargo package has no source files")
    digest = hashlib.sha256()
    for raw_path in paths:
        relative = Path(raw_path.decode("utf-8"))
        absolute = repository_root / relative
        if not absolute.is_file():
            raise NativeCargoSbomError("workspace Cargo source inventory contains a non-file")
        contents = absolute.read_bytes()
        digest.update(relative.as_posix().encode("utf-8"))
        digest.update(b"\0")
        digest.update(hashlib.sha256(contents).digest())
        digest.update(b"\n")
    return digest.hexdigest()


def _lock_checksums(repository_root: Path) -> dict[tuple[str, str, str], str]:
    with (repository_root / "Cargo.lock").open("rb") as source:
        lock = tomllib.load(source)
    checksums: dict[tuple[str, str, str], str] = {}
    for package in lock.get("package", []):
        source = package.get("source")
        checksum = package.get("checksum")
        if source and checksum:
            checksums[(package["name"], package["version"], source)] = checksum
    return checksums


@lru_cache(maxsize=16)
def _cargo_metadata(repository_root: Path, target: str) -> dict[str, object]:
    result = subprocess.run(
        [
            "cargo",
            "metadata",
            "--locked",
            "--filter-platform",
            target,
            "--format-version",
            "1",
        ],
        cwd=repository_root,
        check=True,
        capture_output=True,
        text=True,
    )
    document = json.loads(result.stdout)
    if not isinstance(document, dict):
        raise NativeCargoSbomError("cargo metadata did not return an object")
    return document


def _package_identity(package: dict[str, object], repository_root: Path) -> tuple[str, str]:
    source = package.get("source")
    if isinstance(source, str):
        return source, source
    manifest = Path(str(package["manifest_path"])).resolve()
    try:
        relative = manifest.parent.relative_to(repository_root.resolve()).as_posix()
    except ValueError as error:
        raise NativeCargoSbomError("path Cargo dependency is outside the repository") from error
    revision = _git_revision(repository_root)
    source_info = f"git+https://github.com/piqae/piqae@{revision}#{relative}"
    return f"path:{relative}", source_info


def _package_purl(name: str, version: str, source_info: str) -> str:
    base = f"pkg:cargo/{quote(name, safe='')}@{quote(version, safe='')}"
    if source_info.startswith("registry+"):
        return base
    return f"{base}?vcs_url={quote(source_info, safe='')}"


def _package_spdx_id(key: str, name: str, version: str) -> str:
    slug = "".join(character if character.isalnum() else "-" for character in name).strip("-")
    suffix = hashlib.sha256(key.encode("utf-8")).hexdigest()[:20]
    return f"SPDXRef-Cargo-{slug}-{version.replace('.', '-')}-{suffix}"


def load_cargo_graph(
    repository_root: Path,
    targets: Iterable[str],
    root_package_name: str = "piqae-node-ffi",
) -> CargoGraph:
    repository_root = repository_root.resolve()
    target_tuple = tuple(targets)
    if not target_tuple or len(set(target_tuple)) != len(target_tuple):
        raise NativeCargoSbomError("native Cargo targets must be present and unique")
    lock_checksums = _lock_checksums(repository_root)
    raw_packages: dict[str, dict[str, object]] = {}
    raw_edges: set[tuple[str, str]] = set()
    raw_targets: dict[str, set[str]] = {}
    root_keys: set[str] = set()

    for target in target_tuple:
        metadata = _cargo_metadata(repository_root, target)
        packages = {
            str(package["id"]): package for package in metadata.get("packages", [])
        }
        nodes = {
            str(node["id"]): node
            for node in (metadata.get("resolve") or {}).get("nodes", [])
        }
        roots = [
            package_id
            for package_id, package in packages.items()
            if package.get("name") == root_package_name
            and Path(str(package["manifest_path"])).resolve().is_relative_to(repository_root)
        ]
        if len(roots) != 1:
            raise NativeCargoSbomError(
                f"cargo metadata must contain exactly one workspace {root_package_name!r} package"
            )
        stack = [roots[0]]
        seen: set[str] = set()
        while stack:
            package_id = stack.pop()
            if package_id in seen:
                continue
            seen.add(package_id)
            package = packages.get(package_id)
            node = nodes.get(package_id)
            if package is None or node is None:
                raise NativeCargoSbomError("cargo metadata dependency graph is incomplete")
            identity, _ = _package_identity(package, repository_root)
            key = f"{package['name']}@{package['version']}|{identity}"
            raw_packages[key] = package
            raw_targets.setdefault(key, set()).add(target)
            if package_id == roots[0]:
                root_keys.add(key)
            for dependency in node.get("deps", []):
                kinds = dependency.get("dep_kinds", [])
                if kinds and not any(kind.get("kind") != "dev" for kind in kinds):
                    continue
                dependency_id = str(dependency["pkg"])
                dependency_package = packages.get(dependency_id)
                if dependency_package is None:
                    raise NativeCargoSbomError("cargo metadata names an unknown dependency")
                dependency_identity, _ = _package_identity(dependency_package, repository_root)
                dependency_key = (
                    f"{dependency_package['name']}@{dependency_package['version']}|"
                    f"{dependency_identity}"
                )
                raw_edges.add((key, dependency_key))
                stack.append(dependency_id)

    if len(root_keys) != 1:
        raise NativeCargoSbomError("target graphs disagree about the native root package")

    packages_out: list[CargoPackage] = []
    ids: set[str] = set()
    for key in sorted(raw_packages):
        package = raw_packages[key]
        name = str(package["name"])
        version = str(package["version"])
        identity, source_info = _package_identity(package, repository_root)
        source = package.get("source")
        if isinstance(source, str):
            checksum = lock_checksums.get((name, version, source))
            if checksum is None:
                raise NativeCargoSbomError(
                    f"Cargo.lock has no checksum for {name} {version} from {source}"
                )
        else:
            checksum = _workspace_source_checksum(
                repository_root, Path(str(package["manifest_path"])).resolve().parent
            )
        spdx_id = _package_spdx_id(key, name, version)
        if spdx_id in ids:
            raise NativeCargoSbomError("Cargo packages produced duplicate SPDX identifiers")
        ids.add(spdx_id)
        raw_license = package.get("license")
        normalized_license = _normalize_license_declared(raw_license)
        license_source_info = source_info
        if normalized_license == "NOASSERTION":
            observed = raw_license.strip() if isinstance(raw_license, str) else "not provided"
            license_source_info = (
                f"{source_info}; Cargo license metadata was not a supported SPDX expression: "
                f"{observed}"
            )
        packages_out.append(
            CargoPackage(
                key=key,
                name=name,
                version=version,
                source=source_info,
                purl=_package_purl(name, version, source_info),
                checksum=checksum,
                license_declared=normalized_license,
                spdx_id=spdx_id,
                package_root=Path(str(package["manifest_path"])).resolve().parent,
                targets=tuple(sorted(raw_targets[key])),
                workspace=not isinstance(source, str),
                license_source_info=license_source_info,
            )
        )
    ids_by_key = {package.key: package.spdx_id for package in packages_out}
    if not all(source in ids_by_key and target in ids_by_key for source, target in raw_edges):
        raise NativeCargoSbomError("Cargo dependency graph contains an unreachable package")
    relationships = tuple(
        sorted((ids_by_key[source], ids_by_key[target]) for source, target in raw_edges)
    )
    return CargoGraph(
        packages=tuple(packages_out),
        relationships=relationships,
        root_spdx_id=ids_by_key[next(iter(root_keys))],
        targets=target_tuple,
    )


THIRD_PARTY_LICENSES_FILENAME = "THIRD_PARTY_LICENSES.json"
_LICENSE_FILE_NAME = re.compile(
    r"^(?:licen[cs]e(?:[-_.].*)?|copying(?:[-_.].*)?|notice(?:[-_.].*)?|"
    r"copyright(?:[-_.].*)?|authors?(?:[-_.].*)?)$",
    re.IGNORECASE,
)
_MAX_LICENSE_FILE_BYTES = 256 * 1024
_MAX_UNIQUE_LICENSE_BYTES = 8 * 1024 * 1024


def _third_party_license_files(package: CargoPackage) -> tuple[Path, ...]:
    candidates = [
        path
        for path in package.package_root.iterdir()
        if path.is_file() and _LICENSE_FILE_NAME.fullmatch(path.name)
    ]
    licenses_directory = package.package_root / "LICENSES"
    if licenses_directory.is_dir():
        candidates.extend(path for path in licenses_directory.rglob("*") if path.is_file())
    files = tuple(
        sorted(
            set(candidates),
            key=lambda path: path.relative_to(package.package_root).as_posix(),
        )
    )
    if not files:
        raise NativeCargoSbomError(
            f"reachable Cargo package {package.name} {package.version} has no licence text"
        )
    for path in files:
        if path.is_symlink():
            raise NativeCargoSbomError("Cargo licence evidence must not be a symlink")
        try:
            path.resolve().relative_to(package.package_root.resolve())
        except ValueError as error:
            raise NativeCargoSbomError("Cargo licence evidence escapes its package") from error
        if path.stat().st_size > _MAX_LICENSE_FILE_BYTES:
            raise NativeCargoSbomError("Cargo licence evidence exceeds the bounded file size")
    return files


def third_party_license_report(
    repository_root: Path,
    targets: Iterable[str],
    managed_packages: Iterable[dict[str, object]] = (),
) -> dict[str, object]:
    graph = load_cargo_graph(repository_root, targets)
    texts: dict[str, str] = {}
    packages: list[dict[str, object]] = []
    for package in graph.packages:
        if package.workspace:
            continue
        license_files = []
        for path in _third_party_license_files(package):
            contents = path.read_bytes()
            try:
                exact_text = contents.decode("utf-8")
            except UnicodeDecodeError as error:
                raise NativeCargoSbomError("Cargo licence evidence must be UTF-8 text") from error
            digest = _sha256_bytes(contents)
            previous = texts.setdefault(digest, exact_text)
            if previous != exact_text:
                raise NativeCargoSbomError("Cargo licence text digest collision")
            license_files.append(
                {
                    "path": path.relative_to(package.package_root).as_posix(),
                    "sha256": digest,
                }
            )
        packages.append(
            {
                "name": package.name,
                "version": package.version,
                "source": package.source,
                "purl": package.purl,
                "cargo_checksum_sha256": package.checksum,
                "license_declared": package.license_declared,
                "license_source_info": package.license_source_info,
                "targets": list(package.targets),
                "license_files": license_files,
            }
        )
    if sum(len(text.encode("utf-8")) for text in texts.values()) > _MAX_UNIQUE_LICENSE_BYTES:
        raise NativeCargoSbomError("Cargo licence evidence exceeds the bounded aggregate size")

    managed: list[dict[str, object]] = []
    for package in managed_packages:
        if not isinstance(package, dict):
            raise NativeCargoSbomError("managed third-party licence entries must be objects")
        normalized = {key: value for key, value in package.items() if key != "license_files"}
        raw_files = package.get("license_files")
        if not isinstance(raw_files, list) or not raw_files:
            raise NativeCargoSbomError("managed third-party package has no licence text")
        license_files = []
        for raw_file in raw_files:
            if not isinstance(raw_file, dict):
                raise NativeCargoSbomError("managed third-party licence file must be an object")
            path = raw_file.get("path")
            exact_text = raw_file.get("text")
            if not isinstance(path, str) or not path or not isinstance(exact_text, str):
                raise NativeCargoSbomError("managed third-party licence text is incomplete")
            contents = exact_text.encode("utf-8")
            if len(contents) > _MAX_LICENSE_FILE_BYTES:
                raise NativeCargoSbomError("managed licence evidence exceeds the bounded file size")
            digest = _sha256_bytes(contents)
            previous = texts.setdefault(digest, exact_text)
            if previous != exact_text:
                raise NativeCargoSbomError("managed licence text digest collision")
            license_files.append({"path": path, "sha256": digest})
        normalized["license_files"] = license_files
        managed.append(normalized)
    if sum(len(text.encode("utf-8")) for text in texts.values()) > _MAX_UNIQUE_LICENSE_BYTES:
        raise NativeCargoSbomError("third-party licence evidence exceeds the bounded aggregate size")
    return {
        "schema": 1,
        "cargo_root": "piqae-node-ffi",
        "cargo_targets": list(graph.targets),
        "cargo_packages": packages,
        "managed_packages": managed,
        "texts": texts,
    }


def third_party_license_report_bytes(
    repository_root: Path,
    targets: Iterable[str],
    managed_packages: Iterable[dict[str, object]] = (),
) -> bytes:
    document = third_party_license_report(repository_root, targets, managed_packages)
    return (json.dumps(document, indent=2, sort_keys=True) + "\n").encode("utf-8")


def write_third_party_license_report(
    output: Path,
    repository_root: Path,
    targets: Iterable[str],
    managed_packages: Iterable[dict[str, object]] = (),
) -> None:
    output.write_bytes(
        third_party_license_report_bytes(repository_root, targets, managed_packages)
    )


def validate_third_party_license_report(
    contents: bytes,
    repository_root: Path,
    targets: Iterable[str],
    managed_packages: Iterable[dict[str, object]] = (),
) -> None:
    expected = third_party_license_report_bytes(repository_root, targets, managed_packages)
    if contents != expected:
        raise NativeCargoSbomError(
            "third-party licence report is missing, stale, or does not match the locked target graph"
        )


def _package_document(package: CargoPackage) -> dict[str, object]:
    return {
        "name": package.name,
        "SPDXID": package.spdx_id,
        "versionInfo": package.version,
        "downloadLocation": package.source,
        "sourceInfo": package.license_source_info,
        "filesAnalyzed": False,
        "checksums": [{"algorithm": "SHA256", "checksumValue": package.checksum}],
        "licenseConcluded": "NOASSERTION",
        "licenseDeclared": package.license_declared,
        "copyrightText": "NOASSERTION",
        "externalRefs": [
            {
                "referenceCategory": "PACKAGE-MANAGER",
                "referenceType": "purl",
                "referenceLocator": package.purl,
            }
        ],
    }


def generate_native_sbom(
    archive: Path,
    output: Path,
    repository_root: Path,
    targets: Iterable[str],
    package_name: str,
    version: str,
    binary_suffixes: tuple[str, ...],
) -> None:
    files = _safe_archive_files(archive)
    binaries = [entry for entry in files if entry["name"].endswith(binary_suffixes)]
    if not binaries:
        raise NativeCargoSbomError("native archive does not contain a compiled library")
    graph = load_cargo_graph(repository_root, targets)
    file_documents: list[dict[str, object]] = []
    relationships: list[dict[str, str]] = []
    file_ids: set[str] = set()
    for index, entry in enumerate(files):
        spdx_id = f"SPDXRef-NativeFile-{index}-{entry['sha256'][:20]}"
        if spdx_id in file_ids:
            raise NativeCargoSbomError("native archive produced duplicate SPDX file identifiers")
        file_ids.add(spdx_id)
        file_documents.append(
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
                "spdxElementId": "SPDXRef-Package-NativeArtifact",
                "relationshipType": "CONTAINS",
                "relatedSpdxElement": spdx_id,
            }
        )
    verification = hashlib.sha1(
        "".join(sorted(entry["sha1"] for entry in files)).encode("ascii"),
        usedforsecurity=False,
    ).hexdigest()
    revision = _git_revision(repository_root)
    artifact_source = f"git+https://github.com/piqae/piqae@{revision}"
    artifact_package = {
        "name": package_name,
        "SPDXID": "SPDXRef-Package-NativeArtifact",
        "versionInfo": version,
        "downloadLocation": "NOASSERTION",
        "sourceInfo": artifact_source,
        "filesAnalyzed": True,
        "packageVerificationCode": {"packageVerificationCodeValue": verification},
        "checksums": [{"algorithm": "SHA256", "checksumValue": file_sha256(archive)}],
        "licenseConcluded": "NOASSERTION",
        "licenseDeclared": "Apache-2.0",
        "copyrightText": "NOASSERTION",
        "externalRefs": [
            {
                "referenceCategory": "PACKAGE-MANAGER",
                "referenceType": "purl",
                "referenceLocator": (
                    f"pkg:generic/{quote(package_name, safe='')}@{quote(version, safe='')}?"
                    f"vcs_url={quote(artifact_source, safe='')}"
                ),
            }
        ],
        "comment": "Locked Cargo targets: " + ", ".join(graph.targets),
    }
    relationships.extend(
        [
            {
                "spdxElementId": "SPDXRef-DOCUMENT",
                "relationshipType": "DESCRIBES",
                "relatedSpdxElement": "SPDXRef-Package-NativeArtifact",
            },
            {
                "spdxElementId": "SPDXRef-Package-NativeArtifact",
                "relationshipType": "DEPENDS_ON",
                "relatedSpdxElement": graph.root_spdx_id,
            },
        ]
    )
    relationships.extend(
        {
            "spdxElementId": source,
            "relationshipType": "DEPENDS_ON",
            "relatedSpdxElement": dependency,
        }
        for source, dependency in graph.relationships
    )
    document = {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": f"{package_name}-{version}",
        "documentNamespace": (
            f"https://spdx.org/spdxdocs/{package_name.lower()}-{version}-"
            f"{file_sha256(archive)[:20]}"
        ),
        "creationInfo": {
            "created": "1980-01-01T00:00:00Z",
            "creators": ["Tool: piqae-native-cargo-sbom"],
        },
        "packages": [artifact_package]
        + [_package_document(package) for package in graph.packages],
        "files": file_documents,
        "relationships": relationships,
    }
    output.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def validate_native_sbom(
    archive: Path,
    sbom: Path,
    repository_root: Path,
    targets: Iterable[str],
    package_name: str,
    version: str,
    binary_suffixes: tuple[str, ...],
) -> None:
    files = _safe_archive_files(archive)
    if not any(entry["name"].endswith(binary_suffixes) for entry in files):
        raise NativeCargoSbomError("native archive does not contain a compiled library")
    graph = load_cargo_graph(repository_root, targets)
    document = json.loads(sbom.read_text(encoding="utf-8"))
    if not isinstance(document, dict):
        raise NativeCargoSbomError("native Cargo SBOM must be a JSON object")
    if document.get("spdxVersion") != "SPDX-2.3":
        raise NativeCargoSbomError("native Cargo SBOM must be SPDX 2.3 JSON")
    package_documents = document.get("packages")
    file_documents = document.get("files")
    relationships = document.get("relationships")
    if (
        not isinstance(package_documents, list)
        or not isinstance(file_documents, list)
        or not isinstance(relationships, list)
    ):
        raise NativeCargoSbomError(
            "native Cargo SBOM packages, files, and relationships must be arrays"
        )
    if (
        not all(isinstance(package, dict) for package in package_documents)
        or not all(isinstance(file, dict) for file in file_documents)
        or not all(isinstance(relationship, dict) for relationship in relationships)
    ):
        raise NativeCargoSbomError(
            "native Cargo SBOM package, file, and relationship entries must be objects"
        )
    all_ids = [document.get("SPDXID")]
    all_ids.extend(package.get("SPDXID") for package in package_documents)
    all_ids.extend(file.get("SPDXID") for file in file_documents)
    if None in all_ids or len(all_ids) != len(set(all_ids)):
        raise NativeCargoSbomError("native Cargo SBOM identifiers must be present and unique")

    packages_by_id = {package["SPDXID"]: package for package in package_documents}
    artifact = packages_by_id.get("SPDXRef-Package-NativeArtifact")
    if (
        artifact is None
        or artifact.get("name") != package_name
        or artifact.get("versionInfo") != version
    ):
        raise NativeCargoSbomError("native Cargo SBOM artifact package is inconsistent")
    if artifact.get("licenseConcluded") != "NOASSERTION":
        raise NativeCargoSbomError("native aggregate license must remain NOASSERTION")
    if (
        artifact.get("licenseDeclared") != "Apache-2.0"
        or artifact.get("downloadLocation") != "NOASSERTION"
        or artifact.get("filesAnalyzed") is not True
    ):
        raise NativeCargoSbomError("native Cargo SBOM artifact licence or analysis is inconsistent")
    revision = _git_revision(repository_root)
    artifact_source = f"git+https://github.com/piqae/piqae@{revision}"
    if artifact.get("sourceInfo") != artifact_source or artifact.get("externalRefs") != [
        {
            "referenceCategory": "PACKAGE-MANAGER",
            "referenceType": "purl",
            "referenceLocator": (
                f"pkg:generic/{quote(package_name, safe='')}@{quote(version, safe='')}?"
                f"vcs_url={quote(artifact_source, safe='')}"
            ),
        }
    ]:
        raise NativeCargoSbomError("native Cargo SBOM source or purl is inconsistent")
    if artifact.get("checksums") != [
        {"algorithm": "SHA256", "checksumValue": file_sha256(archive)}
    ]:
        raise NativeCargoSbomError("native Cargo SBOM outer archive checksum is inconsistent")
    if artifact.get("comment") != "Locked Cargo targets: " + ", ".join(graph.targets):
        raise NativeCargoSbomError("native Cargo SBOM target set is inconsistent")

    files_by_name = {
        str(entry.get("fileName", "")).removeprefix("./"): entry
        for entry in file_documents
    }
    if len(files_by_name) != len(file_documents) or set(files_by_name) != {
        entry["name"] for entry in files
    }:
        raise NativeCargoSbomError("native Cargo SBOM does not cover every archive file")
    contained = {
        relationship.get("relatedSpdxElement")
        for relationship in relationships
        if relationship.get("spdxElementId") == "SPDXRef-Package-NativeArtifact"
        and relationship.get("relationshipType") == "CONTAINS"
    }
    file_ids = {entry.get("SPDXID") for entry in file_documents}
    if contained != file_ids:
        raise NativeCargoSbomError("native Cargo SBOM archive containment is incomplete")
    sha1_values: list[str] = []
    for evidence in files:
        file_document = files_by_name[evidence["name"]]
        if (
            file_document.get("licenseConcluded") != "NOASSERTION"
            or file_document.get("licenseInfoInFiles") != ["NOASSERTION"]
        ):
            raise NativeCargoSbomError("native Cargo SBOM file licence is inconsistent")
        checksums = {
            checksum.get("algorithm"): checksum.get("checksumValue")
            for checksum in file_document.get("checksums", [])
        }
        if checksums != {"SHA1": evidence["sha1"], "SHA256": evidence["sha256"]}:
            raise NativeCargoSbomError("native Cargo SBOM file checksum is inconsistent")
        sha1_values.append(evidence["sha1"])
    expected_verification = hashlib.sha1(
        "".join(sorted(sha1_values)).encode("ascii"), usedforsecurity=False
    ).hexdigest()
    if artifact.get("packageVerificationCode") != {
        "packageVerificationCodeValue": expected_verification
    }:
        raise NativeCargoSbomError("native Cargo SBOM verification code is inconsistent")

    expected_packages = {package.spdx_id: _package_document(package) for package in graph.packages}
    actual_cargo_packages = {
        spdx_id: package
        for spdx_id, package in packages_by_id.items()
        if spdx_id != "SPDXRef-Package-NativeArtifact"
    }
    if actual_cargo_packages != expected_packages:
        raise NativeCargoSbomError(
            "native Cargo SBOM dependency name, version, source, purl, "
            "checksum, or license is inconsistent"
        )
    actual_edges = {
        (relationship.get("spdxElementId"), relationship.get("relatedSpdxElement"))
        for relationship in relationships
        if relationship.get("relationshipType") == "DEPENDS_ON"
        and relationship.get("spdxElementId") in expected_packages
    }
    if actual_edges != set(graph.relationships):
        raise NativeCargoSbomError("native Cargo SBOM dependency relationships are inconsistent")
    root_relationships = {
        relationship.get("relatedSpdxElement")
        for relationship in relationships
        if relationship.get("spdxElementId") == "SPDXRef-Package-NativeArtifact"
        and relationship.get("relationshipType") == "DEPENDS_ON"
    }
    if root_relationships != {graph.root_spdx_id}:
        raise NativeCargoSbomError("native Cargo SBOM does not depend on the exact root crate")
    known_ids = set(all_ids)
    relationship_keys: list[tuple[object, object, object]] = []
    for relationship in relationships:
        source = relationship.get("spdxElementId")
        related = relationship.get("relatedSpdxElement")
        relationship_type = relationship.get("relationshipType")
        if source not in known_ids or related not in known_ids:
            raise NativeCargoSbomError("native Cargo SBOM relationship names an unknown identifier")
        relationship_keys.append((source, relationship_type, related))
    if len(relationship_keys) != len(set(relationship_keys)):
        raise NativeCargoSbomError("native Cargo SBOM relationships must be unique")


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    generate = subparsers.add_parser("generate-license-report")
    generate.add_argument("--repository-root", type=Path, required=True)
    generate.add_argument("--target", action="append", dest="targets", required=True)
    generate.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        write_third_party_license_report(
            args.output,
            args.repository_root.resolve(),
            args.targets,
        )
    except (OSError, ValueError, subprocess.CalledProcessError, NativeCargoSbomError) as error:
        parser.error(str(error))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
