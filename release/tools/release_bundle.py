#!/usr/bin/env python3
"""Create and audit deterministic Spool release evidence bundles."""

from __future__ import annotations

import argparse
import base64
import datetime as dt
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path, PurePosixPath
from typing import Any, Iterable

SCHEMA_VERSION = 1
MANIFEST_NAME = "release-manifest.json"
CHECKSUMS_NAME = "SHA256SUMS"
DEFAULT_SBOM_NAME = "sbom.spdx.json"
DEFAULT_PROVENANCE_NAME = "provenance.sigstore.json"
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
CHECKSUM_RE = re.compile(r"^([0-9a-f]{64})  (.+)$")
PRIVATE_KEY_MARKERS = (
    b"-----BEGIN PRIVATE KEY-----",
    b"-----BEGIN RSA PRIVATE KEY-----",
    b"-----BEGIN EC PRIVATE KEY-----",
    b"-----BEGIN OPENSSH PRIVATE KEY-----",
)
SENSITIVE_ARTIFACT_SUFFIXES = (".key", ".p12", ".pfx")
SENSITIVE_ARTIFACT_NAMES = {".env", "id_dsa", "id_ecdsa", "id_ed25519", "id_rsa"}


class AuditError(RuntimeError):
    """A release candidate violated a required evidence invariant."""


def _safe_relative_path(value: str) -> PurePosixPath:
    path = PurePosixPath(value)
    if (
        not value
        or path.is_absolute()
        or "\\" in value
        or any(ord(character) < 32 for character in value)
        or any(part in {"", ".", ".."} for part in path.parts)
    ):
        raise AuditError(f"unsafe release path: {value!r}")
    return path


def _resolve_regular(root: Path, value: str) -> Path:
    relative = _safe_relative_path(value)
    path = root.joinpath(*relative.parts)
    if path.is_symlink():
        raise AuditError(f"release evidence must not be a symlink: {value}")
    if not path.is_file():
        raise AuditError(f"required release file is missing: {value}")
    resolved_root = root.resolve()
    resolved = path.resolve()
    if resolved_root not in resolved.parents:
        raise AuditError(f"release path escapes its bundle: {value}")
    return path


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise AuditError(f"invalid JSON in {path.name}: {error}") from error


def _write_json(path: Path, value: Any) -> None:
    path.write_text(
        json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )


def _relative_files(root: Path, excluded: set[str]) -> list[str]:
    result: list[str] = []
    for path in root.rglob("*"):
        if path.is_symlink():
            raise AuditError(f"release bundles must not contain symlinks: {path}")
        if not path.is_file():
            continue
        relative = path.relative_to(root).as_posix()
        if relative not in excluded:
            result.append(relative)
    return sorted(result)


def _validate_spdx(path: Path) -> None:
    document = _json(path)
    if not isinstance(document, dict):
        raise AuditError("SPDX SBOM must be a JSON object")
    version = document.get("spdxVersion")
    if not isinstance(version, str) or not version.startswith("SPDX-2."):
        raise AuditError("SBOM must declare an SPDX 2.x spdxVersion")
    if document.get("SPDXID") != "SPDXRef-DOCUMENT":
        raise AuditError("SBOM must identify the SPDX document")
    creation = document.get("creationInfo")
    if not isinstance(creation, dict) or not creation.get("creators"):
        raise AuditError("SBOM must include creationInfo.creators")
    packages = document.get("packages", [])
    files = document.get("files", [])
    if not isinstance(packages, list) or not isinstance(files, list):
        raise AuditError("SBOM packages and files must be arrays")
    if not packages and not files:
        raise AuditError("SBOM must describe at least one package or file")


def _decode_statement(value: Any) -> dict[str, Any]:
    if isinstance(value, dict) and "dsseEnvelope" in value:
        envelope = value.get("dsseEnvelope")
        if not isinstance(envelope, dict) or not isinstance(envelope.get("payload"), str):
            raise AuditError("Sigstore provenance has no DSSE payload")
        try:
            decoded = base64.b64decode(envelope["payload"], validate=True)
            value = json.loads(decoded)
        except (ValueError, json.JSONDecodeError, UnicodeDecodeError) as error:
            raise AuditError(f"invalid Sigstore DSSE payload: {error}") from error
    if not isinstance(value, dict):
        raise AuditError("provenance statement must be a JSON object")
    return value


def _provenance_statements(path: Path) -> list[dict[str, Any]]:
    text = path.read_text(encoding="utf-8")
    try:
        parsed = json.loads(text)
    except json.JSONDecodeError:
        statements: list[dict[str, Any]] = []
        for line_number, line in enumerate(text.splitlines(), start=1):
            if not line.strip():
                continue
            try:
                statements.append(_decode_statement(json.loads(line)))
            except json.JSONDecodeError as error:
                raise AuditError(
                    f"invalid provenance JSON on line {line_number}: {error}"
                ) from error
        return statements
    return [_decode_statement(parsed)]


def _validate_provenance(path: Path, artifacts: dict[str, str]) -> None:
    statements = _provenance_statements(path)
    if not statements:
        raise AuditError("provenance must contain at least one statement")
    subjects: dict[str, str] = {}
    for statement in statements:
        if statement.get("_type") != "https://in-toto.io/Statement/v1":
            raise AuditError("provenance must use in-toto Statement v1")
        predicate_type = statement.get("predicateType")
        if not isinstance(predicate_type, str) or not predicate_type.startswith(
            "https://slsa.dev/provenance/"
        ):
            raise AuditError("provenance must use a SLSA provenance predicate")
        raw_subjects = statement.get("subject")
        if not isinstance(raw_subjects, list) or not raw_subjects:
            raise AuditError("provenance must contain subjects")
        for subject in raw_subjects:
            if not isinstance(subject, dict):
                raise AuditError("provenance subject must be an object")
            name = subject.get("name")
            digest = subject.get("digest")
            sha256 = digest.get("sha256") if isinstance(digest, dict) else None
            if isinstance(name, str) and isinstance(sha256, str):
                subjects[name] = sha256.lower()

    for artifact, expected in artifacts.items():
        matches = {
            digest
            for name, digest in subjects.items()
            if name == artifact or name.endswith(f"/{artifact}")
        }
        if not matches:
            raise AuditError(f"provenance does not cover artifact: {artifact}")
        if matches != {expected}:
            raise AuditError(f"provenance digest mismatch for artifact: {artifact}")


def _read_checksums(path: Path) -> dict[str, str]:
    checksums: dict[str, str] = {}
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        match = CHECKSUM_RE.fullmatch(line)
        if not match:
            raise AuditError(f"invalid checksum line {line_number}")
        digest, name = match.groups()
        _safe_relative_path(name)
        if name in checksums:
            raise AuditError(f"duplicate checksum entry: {name}")
        checksums[name] = digest
    if not checksums:
        raise AuditError("checksum file is empty")
    return checksums


def _scan_evidence_for_private_keys(paths: Iterable[Path]) -> None:
    for path in paths:
        if path.stat().st_size > 16 * 1024 * 1024:
            continue
        content = path.read_bytes()
        if any(marker in content for marker in PRIVATE_KEY_MARKERS):
            raise AuditError(f"private key material found in evidence: {path.name}")


def _validate_artifact_safety(paths: Iterable[Path]) -> None:
    for path in paths:
        lowered = path.name.casefold()
        if lowered in SENSITIVE_ARTIFACT_NAMES or lowered.endswith(
            SENSITIVE_ARTIFACT_SUFFIXES
        ):
            raise AuditError(f"sensitive file type cannot be a release artifact: {path.name}")
        if path.stat().st_size <= 16 * 1024 * 1024:
            content = path.read_bytes()
            if any(marker in content for marker in PRIVATE_KEY_MARKERS):
                raise AuditError(f"private key material found in artifact: {path.name}")


def _validate_timestamp(value: Any) -> None:
    if not isinstance(value, str) or not value.endswith("Z"):
        raise AuditError("manifest release.created_at must be an RFC 3339 UTC timestamp")
    try:
        parsed = dt.datetime.fromisoformat(value.removesuffix("Z") + "+00:00")
    except ValueError as error:
        raise AuditError("manifest release.created_at is invalid") from error
    if parsed.tzinfo != dt.timezone.utc:
        raise AuditError("manifest release.created_at must use UTC")


def _load_manifest(root: Path) -> tuple[dict[str, Any], dict[str, str], list[str]]:
    manifest_path = _resolve_regular(root, MANIFEST_NAME)
    manifest = _json(manifest_path)
    if not isinstance(manifest, dict) or manifest.get("schema_version") != SCHEMA_VERSION:
        raise AuditError(f"manifest schema_version must be {SCHEMA_VERSION}")
    release = manifest.get("release")
    if not isinstance(release, dict) or not release.get("name"):
        raise AuditError("manifest release.name is required")
    if not COMMIT_RE.fullmatch(str(release.get("commit", ""))):
        raise AuditError("manifest release.commit must be a lowercase full Git SHA")
    _validate_timestamp(release.get("created_at"))
    evidence = manifest.get("evidence")
    expected_evidence = {
        "checksums": CHECKSUMS_NAME,
        "sbom": DEFAULT_SBOM_NAME,
        "provenance": DEFAULT_PROVENANCE_NAME,
    }
    if evidence != expected_evidence:
        raise AuditError(f"manifest evidence must equal {expected_evidence!r}")
    raw_artifacts = manifest.get("artifacts")
    if not isinstance(raw_artifacts, list) or not raw_artifacts:
        raise AuditError("manifest must contain at least one artifact")
    artifacts: dict[str, str] = {}
    paths: list[str] = []
    casefolded_paths: set[str] = set()
    for artifact in raw_artifacts:
        if not isinstance(artifact, dict):
            raise AuditError("manifest artifact must be an object")
        path_value = artifact.get("path")
        digest = artifact.get("sha256")
        size = artifact.get("size")
        if not isinstance(path_value, str):
            raise AuditError("manifest artifact path must be a string")
        _safe_relative_path(path_value)
        if path_value in artifacts:
            raise AuditError(f"duplicate manifest artifact: {path_value}")
        folded = path_value.casefold()
        if folded in casefolded_paths:
            raise AuditError(f"case-colliding manifest artifact: {path_value}")
        casefolded_paths.add(folded)
        if not isinstance(digest, str) or not SHA256_RE.fullmatch(digest):
            raise AuditError(f"invalid manifest digest: {path_value}")
        if not isinstance(size, int) or size < 0:
            raise AuditError(f"invalid manifest size: {path_value}")
        artifacts[path_value] = digest
        paths.append(path_value)
    return manifest, artifacts, paths


def write_checksums(root: Path) -> None:
    manifest, _, artifact_paths = _load_manifest(root)
    evidence = manifest["evidence"]
    names = sorted(
        artifact_paths
        + [MANIFEST_NAME, evidence["sbom"], evidence["provenance"]]
    )
    lines = [f"{_sha256(_resolve_regular(root, name))}  {name}" for name in names]
    root.joinpath(CHECKSUMS_NAME).write_text("\n".join(lines) + "\n", encoding="utf-8")


def prepare(
    root: Path,
    release_name: str,
    commit: str,
    created_at: str | None,
) -> None:
    if not root.is_dir():
        raise AuditError(f"release directory does not exist: {root}")
    if not release_name.strip():
        raise AuditError("release name must not be empty")
    if not COMMIT_RE.fullmatch(commit):
        raise AuditError("commit must be a lowercase full Git SHA")
    sbom = _resolve_regular(root, DEFAULT_SBOM_NAME)
    provenance = _resolve_regular(root, DEFAULT_PROVENANCE_NAME)
    _validate_spdx(sbom)
    excluded = {
        MANIFEST_NAME,
        CHECKSUMS_NAME,
        DEFAULT_SBOM_NAME,
        DEFAULT_PROVENANCE_NAME,
    }
    artifact_paths = _relative_files(root, excluded)
    if not artifact_paths:
        raise AuditError("release directory has no artifacts")
    artifacts = []
    digest_by_path: dict[str, str] = {}
    for name in artifact_paths:
        path = _resolve_regular(root, name)
        digest = _sha256(path)
        digest_by_path[name] = digest
        artifacts.append({"path": name, "sha256": digest, "size": path.stat().st_size})
    _validate_artifact_safety(_resolve_regular(root, name) for name in artifact_paths)
    _validate_provenance(provenance, digest_by_path)
    timestamp = created_at or dt.datetime.now(dt.timezone.utc).replace(
        microsecond=0
    ).isoformat().replace("+00:00", "Z")
    manifest = {
        "schema_version": SCHEMA_VERSION,
        "release": {"name": release_name, "commit": commit, "created_at": timestamp},
        "artifacts": artifacts,
        "evidence": {
            "checksums": CHECKSUMS_NAME,
            "sbom": DEFAULT_SBOM_NAME,
            "provenance": DEFAULT_PROVENANCE_NAME,
        },
    }
    _write_json(root / MANIFEST_NAME, manifest)
    write_checksums(root)


def _verify_with_github(root: Path, artifacts: list[str], repository: str) -> None:
    gh = shutil.which("gh")
    if gh is None:
        raise AuditError("GitHub CLI is required for cryptographic provenance verification")
    provenance = str(_resolve_regular(root, DEFAULT_PROVENANCE_NAME))
    for artifact in artifacts:
        command = [
            gh,
            "attestation",
            "verify",
            str(_resolve_regular(root, artifact)),
            "--repo",
            repository,
            "--bundle",
            provenance,
        ]
        result = subprocess.run(
            command, capture_output=True, check=False, text=True, timeout=60
        )
        if result.returncode != 0:
            detail = (result.stderr or result.stdout).strip()
            raise AuditError(
                f"cryptographic provenance verification failed for {artifact}: {detail}"
            )


def audit(root: Path, repository: str | None, allow_structural: bool) -> None:
    manifest, artifacts, artifact_paths = _load_manifest(root)
    evidence = manifest["evidence"]
    sbom = _resolve_regular(root, evidence["sbom"])
    provenance = _resolve_regular(root, evidence["provenance"])
    checksum_path = _resolve_regular(root, evidence["checksums"])
    _validate_spdx(sbom)
    _validate_provenance(provenance, artifacts)

    expected_names = set(artifact_paths) | {
        MANIFEST_NAME,
        evidence["checksums"],
        evidence["sbom"],
        evidence["provenance"],
    }
    actual_names = set(_relative_files(root, set()))
    if actual_names != expected_names:
        missing = sorted(expected_names - actual_names)
        extra = sorted(actual_names - expected_names)
        raise AuditError(f"bundle file coverage mismatch; missing={missing}, extra={extra}")
    expected_checksum_names = expected_names - {evidence["checksums"]}
    checksums = _read_checksums(checksum_path)
    if set(checksums) != expected_checksum_names:
        missing = sorted(expected_checksum_names - set(checksums))
        extra = sorted(set(checksums) - expected_checksum_names)
        raise AuditError(f"checksum coverage mismatch; missing={missing}, extra={extra}")
    for name, expected in checksums.items():
        actual = _sha256(_resolve_regular(root, name))
        if actual != expected:
            raise AuditError(f"checksum mismatch: {name}")
    for artifact in manifest["artifacts"]:
        path = _resolve_regular(root, artifact["path"])
        if path.stat().st_size != artifact["size"]:
            raise AuditError(f"manifest size mismatch: {artifact['path']}")
        if _sha256(path) != artifact["sha256"]:
            raise AuditError(f"manifest digest mismatch: {artifact['path']}")
    _validate_artifact_safety(
        _resolve_regular(root, artifact) for artifact in artifact_paths
    )
    _scan_evidence_for_private_keys(
        [
            _resolve_regular(root, MANIFEST_NAME),
            sbom,
            provenance,
            checksum_path,
        ]
    )
    if repository:
        _verify_with_github(root, artifact_paths, repository)
    elif not allow_structural:
        raise AuditError(
            "cryptographic provenance was not verified; pass --github-repository "
            "OWNER/REPO (or --allow-structural-provenance for non-release tests)"
        )


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    prepare_parser = subparsers.add_parser(
        "prepare", help="create the manifest and canonical SHA256SUMS"
    )
    prepare_parser.add_argument("directory", type=Path)
    prepare_parser.add_argument("--release", required=True)
    prepare_parser.add_argument("--commit", required=True)
    prepare_parser.add_argument("--created-at")
    checksum_parser = subparsers.add_parser(
        "checksums", help="regenerate SHA256SUMS from an existing manifest"
    )
    checksum_parser.add_argument("directory", type=Path)
    sbom_parser = subparsers.add_parser(
        "validate-sbom", help="validate the minimum SPDX release contract"
    )
    sbom_parser.add_argument("file", type=Path)
    audit_parser = subparsers.add_parser(
        "audit", help="fail unless the release evidence bundle is complete"
    )
    audit_parser.add_argument("directory", type=Path)
    provenance = audit_parser.add_mutually_exclusive_group(required=True)
    provenance.add_argument("--github-repository", metavar="OWNER/REPO")
    provenance.add_argument("--allow-structural-provenance", action="store_true")
    return parser


def main() -> int:
    args = _parser().parse_args()
    try:
        if args.command == "validate-sbom":
            _validate_spdx(args.file.resolve())
            target = args.file
        else:
            root = args.directory.resolve()
            target = args.directory
        if args.command == "prepare":
            prepare(root, args.release, args.commit, args.created_at)
        elif args.command == "checksums":
            write_checksums(root)
        elif args.command == "audit":
            audit(root, args.github_repository, args.allow_structural_provenance)
    except (AuditError, OSError, subprocess.SubprocessError) as error:
        print(f"release audit failed: {error}", file=sys.stderr)
        return 1
    print(f"release {args.command} passed: {target}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
