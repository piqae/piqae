---
name: build-native-release
description: Build Piqae macOS and Windows release candidates with checksums, SBOMs, signatures, notarisation evidence, and update metadata. Use for native release packaging or updater work.
---

# Build a native release

1. Read `AGENTS.md`, `docs/contributing/releases.md`, and platform guidance.
2. Build in a new ignored `.piqae-test-fixtures/native-release` directory.
3. Run platform tests before packaging and verify matching tray, node,
   executor, profile host, guardian, licence, and SBOM versions.
4. Generate checksums and canonical update metadata before signing.
5. Verify Apple/Windows code signatures and Ed25519 update signatures using
   public keys only; record notarisation status separately.
6. Record artifact names, sizes, hashes, SBOM/provenance locations, signatures,
   and supported/preview status.

Never print signing keys, passwords, notarisation credentials, tokens, or
private crash reports. Do not call signing services without configured release
authority. Remove scratch state, retain release evidence, and never publish
from this skill.
