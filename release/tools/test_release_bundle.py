from __future__ import annotations

import base64
import hashlib
import json
import tempfile
import unittest
from contextlib import nullcontext
from pathlib import Path, PurePosixPath
from unittest import mock

import release_bundle


COMMIT = "1" * 40


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value), encoding="utf-8")


class ReleaseBundleTests(unittest.TestCase):
    def fixture(
        self,
        root: Path,
        *,
        sigstore: bool = False,
        artifact_names: tuple[str, ...] = ("piqae-test.bin",),
    ) -> Path:
        artifacts = [root / name for name in artifact_names]
        for artifact in artifacts:
            artifact.parent.mkdir(parents=True, exist_ok=True)
            artifact.write_bytes(
                f"deterministic release artifact: {artifact.name}\n".encode()
            )
        write_json(
            root / release_bundle.DEFAULT_SBOM_NAME,
            {
                "spdxVersion": "SPDX-2.3",
                "SPDXID": "SPDXRef-DOCUMENT",
                "name": "Piqae test release",
                "creationInfo": {"creators": ["Tool: test_release_bundle"]},
                "files": [
                    {"SPDXID": f"SPDXRef-File-{index}", "fileName": artifact.name}
                    for index, artifact in enumerate(artifacts)
                ],
                "packages": [],
            },
        )
        statement = {
            "_type": "https://in-toto.io/Statement/v1",
            "subject": [
                {
                    "name": artifact.relative_to(root).as_posix(),
                    "digest": {
                        "sha256": hashlib.sha256(artifact.read_bytes()).hexdigest()
                    },
                }
                for artifact in artifacts
            ],
            "predicateType": "https://slsa.dev/provenance/v1",
            "predicate": {"buildDefinition": {}, "runDetails": {}},
        }
        provenance: object = statement
        if sigstore:
            provenance = {
                "mediaType": "application/vnd.dev.sigstore.bundle.v0.3+json",
                "dsseEnvelope": {
                    "payload": base64.b64encode(
                        json.dumps(statement).encode("utf-8")
                    ).decode("ascii"),
                    "payloadType": "application/vnd.in-toto+json",
                    "signatures": [{"keyid": "", "sig": "test-only"}],
                },
                "verificationMaterial": {},
            }
        write_json(root / release_bundle.DEFAULT_PROVENANCE_NAME, provenance)
        return artifacts[0]

    def test_prepare_and_structural_audit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root, sigstore=True)
            release_bundle.prepare(
                root, "v0.1.0-test", COMMIT, "2026-01-01T00:00:00Z"
            )
            release_bundle.audit(root, None, True)

    def test_native_sbom_and_release_evidence_names_do_not_collide(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root, artifact_names=("app.zip", "SBOM.spdx.json"))
            release_bundle.prepare(
                root, "v0.1.0-test", COMMIT, "2026-01-01T00:00:00Z"
            )
            release_bundle.audit(root, None, True)
            manifest = json.loads(
                (root / release_bundle.MANIFEST_NAME).read_text(encoding="utf-8")
            )
            self.assertEqual(
                manifest["evidence"]["sbom"], "release-evidence.spdx.json"
            )
            self.assertIn(
                "SBOM.spdx.json",
                {artifact["path"] for artifact in manifest["artifacts"]},
            )

    def test_case_colliding_bundle_paths_fail_before_manifest_creation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            collision = root / "PIQAE-TEST.BIN"
            if collision.exists():
                paths = list(root.rglob("*")) + [collision]
                context = mock.patch.object(
                    type(root), "rglob", return_value=iter(paths)
                )
            else:
                collision.write_bytes(b"case-colliding artifact\n")
                context = nullcontext()
            with context:
                with self.assertRaisesRegex(
                    release_bundle.AuditError, "case-colliding release bundle paths"
                ):
                    release_bundle.prepare(root, "v0.1.0-test", COMMIT, None)
            self.assertFalse((root / release_bundle.MANIFEST_NAME).exists())

    def test_case_colliding_paths_in_different_directories_fail(self) -> None:
        with self.assertRaisesRegex(
            release_bundle.AuditError, "case-colliding upload paths"
        ):
            release_bundle._require_casefold_unique_paths(
                ["nested/artifact.bin", "NESTED/ARTIFACT.BIN"], "upload paths"
            )

    def test_casefold_file_directory_conflicts_fail_in_either_order(self) -> None:
        for paths in (
            ["sdk", "SDK/client.bin"],
            ["SDK/client.bin", "sdk"],
        ):
            with self.subTest(paths=paths):
                with self.assertRaisesRegex(
                    release_bundle.AuditError, "conflicts with the directory"
                ):
                    release_bundle._require_casefold_unique_paths(
                        paths, "upload paths"
                    )

    def test_prepare_rejects_casefold_file_directory_conflict(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            (root / "sdk").write_bytes(b"file where a directory would be\n")
            paths: list[object] = list(root.rglob("*"))
            nested = mock.Mock()
            nested.is_symlink.return_value = False
            nested.is_file.return_value = True
            nested.relative_to.return_value = PurePosixPath("SDK/client.bin")
            paths.append(nested)
            with mock.patch.object(type(root), "rglob", return_value=iter(paths)):
                with self.assertRaisesRegex(
                    release_bundle.AuditError, "conflicts with the directory"
                ):
                    release_bundle.prepare(root, "v0.1.0-test", COMMIT, None)
            self.assertFalse((root / release_bundle.MANIFEST_NAME).exists())

    def test_manifest_artifact_cannot_case_collide_with_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            release_bundle.prepare(root, "v0.1.0-test", COMMIT, None)
            manifest_path = root / release_bundle.MANIFEST_NAME
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            manifest["artifacts"][0]["path"] = "RELEASE-EVIDENCE.spdx.json"
            write_json(manifest_path, manifest)
            with self.assertRaisesRegex(
                release_bundle.AuditError, "case-colliding manifest paths"
            ):
                release_bundle.audit(root, None, True)

    def test_case_colliding_checksum_entries_fail(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            checksums = Path(temporary) / release_bundle.CHECKSUMS_NAME
            checksums.write_text(
                f"{'0' * 64}  artifact.bin\n{'1' * 64}  ARTIFACT.BIN\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(
                release_bundle.AuditError, "case-colliding checksum entries"
            ):
                release_bundle._read_checksums(checksums)

    def test_tampered_artifact_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            artifact = self.fixture(root)
            release_bundle.prepare(root, "v0.1.0-test", COMMIT, None)
            artifact.write_bytes(b"tampered")
            with self.assertRaisesRegex(release_bundle.AuditError, "checksum mismatch"):
                release_bundle.audit(root, None, True)

    def test_missing_provenance_subject_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            write_json(
                root / release_bundle.DEFAULT_PROVENANCE_NAME,
                {
                    "_type": "https://in-toto.io/Statement/v1",
                    "subject": [
                        {"name": "other.bin", "digest": {"sha256": "0" * 64}}
                    ],
                    "predicateType": "https://slsa.dev/provenance/v1",
                    "predicate": {},
                },
            )
            with self.assertRaisesRegex(release_bundle.AuditError, "does not cover"):
                release_bundle.prepare(root, "v0.1.0-test", COMMIT, None)

    def test_symlink_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            (root / "linked.bin").symlink_to(root / "piqae-test.bin")
            with self.assertRaisesRegex(release_bundle.AuditError, "symlinks"):
                release_bundle.prepare(root, "v0.1.0-test", COMMIT, None)

    def test_release_audit_requires_identity_verification(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            release_bundle.prepare(root, "v0.1.0-test", COMMIT, None)
            with self.assertRaisesRegex(
                release_bundle.AuditError, "cryptographic provenance"
            ):
                release_bundle.audit(root, None, False)

    def test_untracked_bundle_file_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            release_bundle.prepare(root, "v0.1.0-test", COMMIT, None)
            (root / "untracked.txt").write_text("not in the manifest", encoding="utf-8")
            with self.assertRaisesRegex(release_bundle.AuditError, "file coverage"):
                release_bundle.audit(root, None, True)

    def test_private_key_artifact_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            (root / "unexpected.key").write_text("not really a key", encoding="utf-8")
            with self.assertRaisesRegex(release_bundle.AuditError, "sensitive file type"):
                release_bundle.prepare(root, "v0.1.0-test", COMMIT, None)


if __name__ == "__main__":
    unittest.main()
