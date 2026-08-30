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
`macos` artifact scope, producing a private, short-lived candidate. Operators
may select `macos`, `windows`, `linux`, `containers`, `apple-sdk`, or
`windows-sdk` to exercise only that candidate lane. The explicit `all` scope
retains every effective platform for cross-platform certification. Pushing a
protected `v*` tag selects `all`; a manual dispatch against that tag keeps the
requested scope. Stable publication still accepts only a tag whose commit is
already on `main`.

Shared protocol, database, SDK, licence, and source-policy gates run once.
Selected candidates then build in parallel with matrix `fail-fast: false`.
Each lane with a stable publisher crosses its own protected promotion gate as
soon as that lane's signature, notarisation where applicable, checksum, SBOM,
provenance, and evidence audit pass. A failed Windows, Linux, SDK, or container
candidate therefore cannot prevent a successful selected macOS candidate from
promoting its appcast and appearing as a clearly labelled macOS Preview
prerelease. The `all` job still fails its separate aggregate certification
result unless every effective selected candidate—and every requested stable
publisher—succeeds. A visible macOS prerelease is not evidence that `all`
certification passed; its notes say when aggregate certification remains
pending or failed.

Container jobs likewise build checksummed, provenance-attested Docker archives
as private 14-day workflow artifacts without authenticating to GHCR. Their
independent protected promotion matrix verifies and loads those exact archives,
pushes the version and commit tags, and attests the resulting registry digests.
It neither gates nor is gated by macOS promotion. Windows uses its own reusable
signed publisher when its support tier permits publication; an explicitly
selected Windows candidate may still be built privately while that tier is
Disabled.

Linux and both embedded SDK selectors are candidate-only because they do not
yet have stable registry or update-feed publishers. `publish=true` fails in
`prepare` for those scopes. Windows stable publication likewise fails while the
desktop tier remains Disabled. These fail-closed checks keep a successful build
from being misrepresented as a public or Supported release.

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
4. Let the single **Piqae release** workflow build and, where supported, publish
   exactly the selected artifact scope. `macos` publishes the signed universal macOS package,
   updater, appcast, checksum, SBOM, provenance, stable manifest entry, and a
   macOS-only GitHub prerelease. It does not claim or attach Windows, Linux,
   container, or embedded SDK artifacts. `all` builds the effective Windows,
   Linux, container, and provenance-bearing Apple/Windows embedded SDK
   candidates in parallel. macOS and containers promote independently; only
   after every effective lane succeeds does aggregate certification attach the
   Linux and SDK evidence and update the prerelease notes. Embedded SDK candidates remain
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
