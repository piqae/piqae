from __future__ import annotations

import base64
import hashlib
import json
import tempfile
import unittest
from pathlib import Path

import release_bundle


COMMIT = "1" * 40


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value), encoding="utf-8")


class ReleaseBundleTests(unittest.TestCase):
    def fixture(self, root: Path, *, sigstore: bool = False) -> Path:
        artifact = root / "spool-test.bin"
        artifact.write_bytes(b"deterministic release artifact\n")
        digest = hashlib.sha256(artifact.read_bytes()).hexdigest()
        write_json(
            root / release_bundle.DEFAULT_SBOM_NAME,
            {
                "spdxVersion": "SPDX-2.3",
                "SPDXID": "SPDXRef-DOCUMENT",
                "name": "Spool test release",
                "creationInfo": {"creators": ["Tool: test_release_bundle"]},
                "files": [{"SPDXID": "SPDXRef-File", "fileName": artifact.name}],
                "packages": [],
            },
        )
        statement = {
            "_type": "https://in-toto.io/Statement/v1",
            "subject": [{"name": artifact.name, "digest": {"sha256": digest}}],
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
        return artifact

    def test_prepare_and_structural_audit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root, sigstore=True)
            release_bundle.prepare(
                root, "v0.1.0-test", COMMIT, "2026-01-01T00:00:00Z"
            )
            release_bundle.audit(root, None, True)

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
            (root / "linked.bin").symlink_to(root / "spool-test.bin")
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
