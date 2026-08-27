# Release evidence

Piqae treats a compiled installer as a candidate, not a release. A releasable
bundle contains all of the following in one directory:

- one or more immutable artifacts;
- `release-manifest.json`, conforming to
  `release/release-manifest.schema.json`;
- `SHA256SUMS`, covering every artifact, the manifest, SBOM, and provenance;
- `sbom.spdx.json`, an SPDX 2.x JSON software bill of materials; and
- `provenance.sigstore.json`, repository-bound SLSA build provenance.

The reusable `.github/workflows/release-evidence.yml` downloads artifacts from
the calling build, generates the SBOM and GitHub provenance, assembles the
manifest and checksums, verifies the provenance against the repository
identity, and uploads the audited candidate. Native release workflows should
call it only after platform signing and packaging have succeeded.

Embedded Apple and Windows native SDK candidates use a stricter native SBOM
gate. The SPDX document contains the exact outer archive SHA-256, every archived
binary and licence file hash, and the complete non-development dependency graph
reachable from `piqae-node-ffi`. That graph is regenerated from `Cargo.lock` and
`cargo metadata --locked --filter-platform`: all five Apple build targets are
unioned, while Windows is evaluated for `x86_64-pc-windows-msvc`. Each Cargo
package records its name, version, source, purl, source checksum, declared
licence, and `DEPENDS_ON` edges. Validation recomputes the target graph and
fails for an omitted package, forged checksum, or changed relationship. Because
the compiled archive aggregates third-party code, its SPDX
`licenseConcluded` deliberately remains `NOASSERTION`; the exact repository
`LICENSE` and `NOTICE` still travel inside each archive. Every native archive
also contains deterministic `THIRD_PARTY_LICENSES.json`: it binds each reachable
package to the exact target set and locked checksum, includes deduplicated exact
licence/attribution text from the package source, and is regenerated during
validation. Missing text, stale graphs, or tampering fail the release. The
Windows NuGet copy additionally binds the exact pinned managed dependency
package and its bundled licence text.

## Local audit

Use structural verification while developing fixtures:

```console
python3 release/tools/release_bundle.py audit PATH \
  --allow-structural-provenance
```

Structural verification checks paths, file sizes, digests, checksum coverage,
SPDX shape, in-toto/SLSA subjects, and accidental private-key material in
evidence. It does **not** establish who produced the attestation and is never
enough for a published release.

For a release decision, install GitHub CLI, authenticate if required, and bind
the provenance to the expected repository:

```console
python3 release/tools/release_bundle.py audit PATH \
  --github-repository OWNER/REPOSITORY
```

The command invokes `gh attestation verify` for every artifact and fails closed
if the CLI is absent or any subject cannot be verified.

## Building a bundle outside GitHub Actions

Place artifacts, an SPDX document named `sbom.spdx.json`, and a Sigstore bundle
named `provenance.sigstore.json` in a clean directory. The provenance must cover
every artifact by name and SHA-256 digest. Then run:

```console
python3 release/tools/release_bundle.py prepare PATH \
  --release v1.2.3 \
  --commit 0123456789abcdef0123456789abcdef01234567
python3 release/tools/release_bundle.py audit PATH \
  --github-repository OWNER/REPOSITORY
```

`prepare` rejects symlinks and unsafe paths and writes the manifest and
canonical checksum file. `checksums` can regenerate `SHA256SUMS` from an
existing manifest, but any regeneration invalidates review evidence and must be
re-audited.

## Gate ownership

Automated policy lives in `.github/workflows/supply-chain.yml`,
`.cargo/audit.toml`, `deny.toml`, and `.gitleaks.toml`. Live identity, billing,
observability, manual physical-printer, Apple notarisation, Authenticode,
disaster-recovery, security-review, and soak evidence remains open until its
external record exists. Neither an SBOM nor a successful spooler handoff proves
those claims.

Any vulnerability scanner ignore must have the same identifier in
`release/security-exceptions.json`, with an owner, narrow reachability
explanation, removal condition, and future review date. CI rejects undocumented
or expired exceptions.
