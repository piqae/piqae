# Piqae driver support packs

Support packs add evidence-backed normalization for capabilities reported by an
installed printer driver. They do not replace a driver and cannot contain code,
commands, native profile blobs, documents, credentials, licensed binaries or
permission changes.

Follow the cross-platform [contributor workflow](CONTRIBUTING.md), then start
from [`templates/minimal`](templates/minimal). A contribution must contain:

- an exact driver-package SHA-256, driver identifier and driver version;
- platform and, where relevant, exact device and firmware constraints;
- display-safe native capability fixtures with documented redistribution rights;
- bounded native-choice to semantic-choice mappings;
- platform-qualified positive and unknown-choice rejection conformance cases;
- evidence explaining how each mapping was observed; and
- an Apache-2.0-compatible licence for contributed data.

Do not infer executable settings from a friendly model name. Names can help an
operator discover a candidate pack, but Piqae activates a pack only after every
exact selector matches and its canonical digest is trusted. Multiple matches are
an error. Install order and version order never establish precedence.

## Trust and distribution

`piqae-support-packs::pack_digest` hashes a length-delimited, sorted inventory of
all regular files except `SIGNATURE`. Production deployments must either pin
that digest or configure an Ed25519 trust root and provide a hex-encoded detached
Ed25519 signature over the 32 digest bytes in `SIGNATURE`. Any content change
invalidates both forms of trust.

The durable node loads packs explicitly at startup. Directories and trust
material are comma-separated and may also be repeated on the command line:

```console
PIQAE_SUPPORT_PACK_DIRS=/opt/piqae/packs/vendor-family \
PIQAE_SUPPORT_PACK_DIGESTS=<canonical-sha256> \
piqae-agent
```

For publisher trust, use `PIQAE_SUPPORT_PACK_TRUST_KEYS` with one or more
hex-encoded Ed25519 public keys instead of a pinned digest. Configuring an
untrusted, malformed or ambiguous pack prevents the affected runtime operation
from proceeding; packs are never selected by install order. A pack is projected
only when discovery supplies its exact driver package digest, driver identity,
driver version and every optional device or firmware predicate declared by the
selector. Missing evidence produces no semantic facets.

Symlinks, traversal paths, oversized packs and sensitive fixture fields are
rejected before a mapping is usable. A trusted pack still supplies normalized
data only; the platform adapter must validate and apply a requested choice to a
job-scoped driver configuration.

## Evidence tiers

- `discovered`: a redacted capability response was captured.
- `mapped`: semantic meanings are supported by authoritative documentation.
- `replay_tested`: capture and job-scoped replay were tested on the declared
  driver matrix without asserting that output reached paper.
- `physically_certified`: named hardware, stock and controlled output evidence
  verify the specific claim.

A pack must use its lowest applicable tier. Spooler acceptance is not physical
certification. Per-facet evidence can be split into separate packs when claims
have different maturity.

## Review checklist

1. Run `cargo test -p piqae-support-packs` and workspace formatting/Clippy.
2. Confirm fixtures contain no serials, usernames, paths, documents or secrets.
3. Confirm mappings use existing canonical facets and never
   `profile.safe_overrides`.
4. Include a platform-qualified positive mapping and an unknown-choice rejection
   case; conformance resolves by both platform and native capability key, and
   unknown values remain unsupported.
5. Document fixture provenance and licences in `evidence/README.md` and
   `LICENSES.md`.
6. Sign commits with DCO (`git commit -s`).

Vendor-family directories should only be added with real, legally
redistributable evidence. The repository intentionally ships no illustrative
OKI, Brother or Epson native option keys because invented mappings would be
unsafe.

Model qualification kits may live under `qualifications/` when authoritative
product facts are known but exact native driver choices have not yet been
captured. They are never passed to `PIQAE_SUPPORT_PACK_DIRS` and cannot activate
runtime mappings. The OKI Pro1040/Pro1050 PostScript process starts at
[`qualifications/oki-pro10xx-postscript`](qualifications/oki-pro10xx-postscript/README.md).
