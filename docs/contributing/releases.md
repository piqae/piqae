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
