---
name: release-audit
description: Audit a Piqae release candidate against code, tests, packaging, licensing, security, deployment, physical certification, and support claims. Use before preview, private-beta, or public release.
---

# Audit a Piqae release

1. Read `AGENTS.md`, `docs/contributing/releases.md`, and the support matrix.
2. Use `.piqae-test-fixtures/release-audit` only for temporary evidence.
3. Run `cargo xtask release check`.
4. Verify OpenAPI/SDK consistency, tenant isolation, Compose/Helm validation,
   installer SBOMs, provenance, signatures, update rollback, and N/N-1 support.
5. Classify every claim as implemented, tested, preview, or supported.
6. Keep Apple issuance/notarisation, Authenticode issuance, Windows/OKI
   physical tests, DR rehearsal, security review, and soak evidence open unless
   their external records exist.
7. Produce a pass/fail table with exact evidence locations.

Never include secrets, device credentials, documents, native profile payloads,
customer metadata, or signed URLs. Remove audit scratch state only; do not
publish, deploy, print, sign, or alter production.
