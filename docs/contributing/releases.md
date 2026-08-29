# Release checks

**Status:** evidence-gated release process; current native tiers remain Preview
or Disabled rather than stable Supported.

Release support is evidence-gated. Run:

```console
cargo xtask release check
```

The default is the explicit full-certification scope, equivalent to
`cargo xtask release check --platform all`. A macOS release operator can use
`--platform macos` to run the shared database, protocol, JavaScript, licence,
and source-policy gates plus the macOS shell and linked Apple runtime checks,
while excluding Windows and Linux native packages. CI uses
`--platform core` for the shared gates exactly once before platform packaging;
that internal scope does not produce a releasable native candidate by itself.

The command performs the full non-physical test suite, builds the JavaScript
workspace, checks dependency policy when `cargo-deny` is available, validates
license declarations, and requires a clean working tree.

Unlike normal contributor tests, `cargo xtask release check` fails immediately
unless `PIQAE_TEST_DATABASE_URL` points to a disposable PostgreSQL database. It
runs the routing-recovery, platform-service-account authorization, and platform
customer-account lifecycle database suites through
`release/tools/check_postgres_release_tests.py`. The wrapper rejects successful
commands that report a skip, run zero tests, or do not include the exact
required database test. A compile-only or policy-only check is not release
evidence.

The release database account must be allowed to create and drop isolated test
schemas. Never point the variable at a production database.

## Build and publication boundary

Local builds are the fastest preflight and are encouraged. They are not release
inputs: do not upload a locally built app, installer, archive, checksum, or
appcast for publication. A user-facing package is rebuilt from the reviewed tag
on the target hosted runner so its platform signature, update signature, SBOM,
checksum, and GitHub provenance all cover the exact published bytes.

The manual **Piqae release** workflow defaults to `publish=false` and the
`macos` artifact scope, producing a private, short-lived candidate. The
explicit `all` scope retains Windows, Linux, container, Apple SDK, and Windows
SDK builds for full certification. A protected `v*` tag always selects `all`
and fails closed unless every selected artifact succeeds; stable publication
still accepts only a tag whose commit is already on `main`. Use the manual
macOS scope before tagging when a faster signed candidate is required.
The macOS candidate is built and audited in parallel with the selected sibling
artifacts, but its protected promotion, appcast, stable manifest, and GitHub
release remain blocked until the aggregate gate proves every selected job
succeeded. Matrix jobs use `fail-fast: false`, and independent sibling jobs are
not cancelled when another candidate fails, so their evidence remains useful
without permitting a partial publication.

Container jobs likewise build checksummed, provenance-attested Docker archives
as private 14-day workflow artifacts without authenticating to GHCR. Only after
the aggregate gate succeeds does a separate `fail-fast: false` promotion matrix
verify and load those exact archives, push the version and commit tags, and
attest the resulting registry digests. macOS promotion follows successful
container promotion. The parent invokes Windows as candidate-only; if its
support tier is enabled, stable publication fails in `prepare` until Windows has
an equivalent aggregate-gated promoter, preventing the dormant workflow from
advancing its installer or appcast early.

Windows-only and Linux-only selectors are intentionally not exposed yet.
Windows desktop remains Disabled in the support matrix, while the Linux bundle
does not have an independent signed update-feed publisher. Adding either choice
before those platform-specific publication contracts exist would create a
selector that could build artifacts but could not truthfully complete a stable
release. Use `all` for their current certification evidence.

Before tagging:

1. Review `release/support-matrix.yaml`.
2. Confirm every enabled platform has current CI and packaging evidence.
3. Confirm SBOM, checksum, provenance, signing, and notarization gates relevant
   to that platform.
4. Record physical-printer evidence separately; never infer it from a simulated
   or spooler-only test.
5. Verify release notes distinguish implemented, preview, and certified
   behavior.

Then:

1. Update the workspace and native component versions together and merge the
   reviewed change to `main` after required CI passes.
2. Create and push `v<version>` at that merged commit.
3. Review the candidate evidence and approve the protected `native-release`
   environment.
4. Let the single **Piqae release** workflow publish exactly the selected
   artifact scope. `macos` publishes the signed universal macOS package,
   updater, appcast, checksum, SBOM, provenance, stable manifest entry, and a
   macOS-only GitHub prerelease. It does not claim or attach Windows, Linux,
   container, or embedded SDK artifacts. `all` additionally builds and attaches
   the Windows packages, Linux bundles, container images, and provenance-bearing
   Apple/Windows embedded SDK candidates. Embedded SDK candidates remain
   unsigned Preview assets until their package-signing gates are configured;
   the workflow does not publish them to a package registry. Never start the
   platform workflows separately for the same version.
5. Confirm the public-feed smoke checks, then canary the release before widening
   availability.

If a signed candidate and its evidence audit succeed but promotion fails, do
not move the tag or upload locally rebuilt bytes. Use **Recover macOS
promotion** with the original run, tag, commit, version, build, candidate, and
evidence identities. The recovery workflow verifies all identities and
provenance before it reaches the protected `native-release` environment, then
uses the same reusable publisher as an ordinary release. Recovery inputs are
not an override: every mismatch fails closed. Dispatch also requires `confirm`
to equal `PROMOTE-VERIFIED-CANDIDATE`; an absent or incorrect value stops
before candidate validation.

The release check never sends a print job. Hardware certification must be
explicitly scheduled with a named printer and controlled fixture.

Release artifacts additionally pass the fail-closed evidence audit documented
in [`release/README.md`](../../release/README.md). Local structural provenance
validation is suitable for tests only; a published release requires
cryptographic verification against the expected repository identity.

Runner selection and cost controls are documented in
[CI and release operations](../operations/ci-and-release.md).

Release caches are platform-scoped (`release-core`, macOS universal, Apple SDK,
Windows SDK, and per-Linux-target keys) so a fast macOS run does not restore or
evict unrelated native outputs. Apple XCFramework construction resolves Cargo's
authoritative target directory, including an absolute `CARGO_TARGET_DIR`, and
keeps its temporary consumer and reproducibility builds in trap-cleaned bounded
directories. Hosted artifacts retain their existing 14-day limit.
