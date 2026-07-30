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

Before tagging:

1. Review `release/support-matrix.yaml`.
2. Confirm every enabled platform has current CI and packaging evidence.
3. Confirm SBOM, checksum, provenance, signing, and notarization gates relevant
   to that platform.
4. Record physical-printer evidence separately; never infer it from a simulated
   or spooler-only test.
5. Verify release notes distinguish implemented, preview, and certified
   behavior.

The release check never sends a print job. Hardware certification must be
explicitly scheduled with a named printer and controlled fixture.

Release artifacts additionally pass the fail-closed evidence audit documented
in [`release/README.md`](../../release/README.md). Local structural provenance
validation is suitable for tests only; a published release requires
cryptographic verification against the expected repository identity.
