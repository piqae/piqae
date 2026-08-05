# Release checks

**Status:** evidence-gated release process; current native tiers remain Preview
or Disabled rather than stable Supported.

Release support is evidence-gated. Run:

```console
cargo xtask release check
```

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

The manual **Piqae release** workflow defaults to `publish=false` and produces
private, short-lived candidates. Stable publication accepts only a protected
`v*` tag whose commit is already on `main`.

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
4. Let the single **Piqae release** workflow publish the signed macOS and
   Windows packages, Linux bundles, container images, stable appcasts, release
   manifest, and GitHub prerelease. Never start the platform workflows
   separately for the same version.
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
