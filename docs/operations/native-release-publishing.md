# Native release publishing

**Status:** protected hosting and fail-closed package workflows are connected
for Preview candidates. No Piqae native platform is currently Supported.

Secret custody, local-build boundaries, and the low-cost Windows signing
options are defined in [Secrets, signing, and low-cost builds](secrets-and-signing.md).

This runbook separates four states:

1. **built** — a workflow produced a candidate;
2. **verified** — checksums, SBOM, provenance, platform signature, update
   signature, and applicable notarisation evidence passed;
3. **published** — immutable files exist in the release bucket;
4. **promoted** — the stable appcast and web download manifest reference those
   exact files.

Reaching an earlier state never implies a later one.

## Storage layout

Native release artifacts use the dedicated private Cloudflare R2 bucket
`piqae-releases`. The production web service has a bucket-scoped read-only
identity; the protected GitHub release environment has a separate bucket-scoped
read/write publisher identity.
Customer print documents use `piqae-documents`; credentials and retention
policies must not be shared between the two.

```text
native/
  releases/
    <version>/
      <platform>-<workflow-run>-<attempt>/
        <immutable package, appcast, checksum, SBOM, and platform record>
  stable/
    appcast-macos.xml
    appcast-windows.xml
    manifest.json
    manifest.json.sha256
    piqae-<version>-<platform>-<architecture>.<extension>
    piqae-<version>-<platform>-<architecture>.<extension>.sha256
    piqae-<version>-<platform>-SBOM.spdx.json
```

Use versioned filenames for packages and evidence. Do not overwrite a
versioned object. Only channel pointers such as an appcast are mutable, and
their replacement must be atomic.

The public web routes are:

```text
https://downloads.piqae.com/releases/preview/<allowed-artifact>
https://downloads.piqae.com/releases/stable/<allowed-artifact>
```

They return a short-lived signed bucket redirect. The application accepts only
the constrained appcast and `piqae-*` artifact filename set. It does not expose
arbitrary bucket keys or credentials.

## Signed-only promotion

Unsigned workflow artifacts may be retained privately for engineering tests,
but must never enter `native/stable`, a stable appcast, or a Supported download
manifest.

For each platform:

1. Build every bundled component from the same reviewed commit and version.
2. Run platform and non-physical queue/recovery tests.
3. Generate checksums, SBOM, and repository-bound provenance.
4. Sign native code and the installer/package.
5. Sign the update archive or installer payload with the separate update key
   and include that signature in the generated metadata.
6. Verify signatures using public keys only; verify Apple notarisation and
   stapling where applicable.
7. Upload versioned package and evidence to a run-unique immutable
   `native/releases/<version>/...` prefix.
8. Exercise clean install, upgrade, busy-node deferral, reconnect, and
   restoration using non-production nodes.
9. Record physical-printer evidence separately where the support claim
   requires it.
10. Promote the already-verified package and checksum into `native/stable/`.
11. Replace the platform's signed stable appcast.
12. Merge the verified platform record into a new immutable manifest snapshot,
    then promote `native/stable/manifest.json` last.
13. Confirm `/downloads` and both public appcast routes from outside Railway.
14. Roll out to canary nodes and observe before widening.

The weekly **Published update feed** check dereferences every published appcast
enclosure. Windows returning 404 is accepted while that platform has no public
feed; once present, the enclosure is required to remain below the stable Piqae
origin and return downloadable bytes.

Do not put signing keys, bucket credentials, notarisation credentials, or
release tokens in appcasts, workflow artifacts, command arguments, or logs.
The web service uses its release-origin credentials only for reads. Give its R2
API token `Object Read` access to only `piqae-releases`. Publishing uses a
separate token with `Object Read & Write` access to only that bucket. Do not
enable an `r2.dev` public URL: downloads remain behind the constrained Piqae
route and short-lived signed redirects.

## CI signing authority

GitHub remains the release orchestrator even when Blacksmith supplies the
runner compute. Keep two separate GitHub environments:

- `native-signing` exposes only platform code-signing, notarisation, and update
  signing material to the macOS and Windows candidate jobs.
- `native-release` exposes only the dedicated release-bucket
  credentials to the serialized promotion jobs.

The macOS signing environment requires:

```text
MACOS_CERTIFICATE_P12_BASE64
MACOS_CERTIFICATE_PASSWORD
MACOS_APPLICATION_IDENTITY
MACOS_INSTALLER_CERTIFICATE_P12_BASE64
MACOS_INSTALLER_CERTIFICATE_PASSWORD
MACOS_INSTALLER_IDENTITY
APPLE_ID
APPLE_TEAM_ID
APPLE_APP_PASSWORD
SPARKLE_PRIVATE_KEY_BASE64
SPARKLE_PUBLIC_ED_KEY
```

The Windows signing environment always requires the WinSparkle update key and
signer trust policy:

```text
WINDOWS_RFC3161_TIMESTAMP_URL
WINDOWS_EXPECTED_CERTIFICATE_SUBJECT
WINDOWS_EXPECTED_CERTIFICATE_THUMBPRINT
WINSPARKLE_ED25519_PRIVATE_KEY_BASE64
WINSPARKLE_ED25519_PUBLIC_KEY
```

Set `WINDOWS_SIGNING_PROVIDER=artifact-signing` for the preferred Microsoft
Artifact Signing path, then configure these non-secret `native-signing`
environment variables:

```text
AZURE_ARTIFACT_SIGNING_CLIENT_ID
AZURE_ARTIFACT_SIGNING_TENANT_ID
AZURE_ARTIFACT_SIGNING_SUBSCRIPTION_ID
AZURE_ARTIFACT_SIGNING_ENDPOINT
AZURE_ARTIFACT_SIGNING_ACCOUNT_NAME
AZURE_ARTIFACT_SIGNING_CERTIFICATE_PROFILE_NAME
```

The workflow authenticates through a GitHub-environment-scoped Entra federated
credential; do not create an Azure client secret. Configure
`WINDOWS_RFC3161_TIMESTAMP_URL=http://timestamp.acs.microsoft.com`, the exact
issued subject in `WINDOWS_EXPECTED_CERTIFICATE_SUBJECT`, and leave
`WINDOWS_EXPECTED_CERTIFICATE_THUMBPRINT` unset because Artifact Signing leaf
certificates rotate. Microsoft currently makes Public Trust available to New
Zealand organizations; the Azure billing profile must exactly match the legal
publisher identity.

DigiCert KeyLocker remains a separately reviewed remote-HSM fallback. Set
`WINDOWS_SIGNING_PROVIDER=digicert-keylocker`, then configure:

```text
DIGICERT_SM_HOST
DIGICERT_SM_API_KEY
DIGICERT_SM_CLIENT_CERT_FILE_B64
DIGICERT_SM_CLIENT_CERT_PASSWORD
DIGICERT_SM_KEYPAIR_ALIAS
```

The client certificate authenticates the narrowly scoped DigiCert service
user; it is not the Authenticode private key. The release workflow installs the
pinned DigiCert KSP integration, synchronizes only the public code-signing
certificate, signs hashes remotely, and verifies the resulting subject and
thumbprint. Retain `WINDOWS_SIGNING_PROVIDER=pfx` only as a compatibility path
using `WINDOWS_AUTHENTICODE_PFX_BASE64` and
`WINDOWS_AUTHENTICODE_PFX_PASSWORD`.

The Sparkle and WinSparkle private keys need encrypted offline recovery copies.
Losing an update key after clients trust its public key can strand installed
nodes. Do not rely on a GitHub secret as the only copy. Certificate and
notarisation credentials must be rotated independently from the update keys.

Unsigned manual builds remain private workflow artifacts. A tag or a manual
`publish=true` request fails unless the complete signing set for each requested
publisher exists. Candidate-only Linux and embedded SDK scopes reject
`publish=true`; Windows publication rejects a Disabled support tier. Each stable
publisher enters `native-release`, verifies its immutable object SHA-256 and
length, promotes the package before its appcast, and promotes the combined
manifest last.

`.github/workflows/release.yml` is the only tag entry point. It runs the shared
gates once, calls selected macOS and Windows workflows as reusable stages, and
builds selected Linux, container, and SDK candidates in parallel. macOS,
Windows when enabled, and containers promote independently through their own
verified path; one platform failure does not cancel or invalidate a successful
sibling promotion. The explicit `all` lane separately requires every effective
selected candidate and requested publisher before recording aggregate
certification and attaching candidate-only assets. Serialized platform
finalizers tolerate either a draft or an existing prerelease and merge their
machine-readable publication state, so macOS-first and Windows-first completion
produce the same notes without a last-writer-wins race. Aggregate failure
records **Failed** before the certification job fails. A selected-lane policy
failure does not begin aggregate attachment; an attachment failure certifies
none of the candidate-only assets even if GitHub accepted a partial upload.

Direct dispatch of `windows-release.yml` can build a private candidate or use
the explicit protected unsigned-preview tag flow only. It cannot request stable
publication. Stable Windows promotion requires a reusable call from canonical
`release.yml`, whose preparation and core jobs enforce product versioning,
support tier, repository identity, protected tag, `main` ancestry, database,
protocol, licence, SDK, and source-policy gates.
The Windows entry-point job rejects any other stable caller before the
`native-signing` environment or its credentials are reached.

The release bucket and
signed appcasts remain the updater authority; GitHub Releases is the
human-facing mirror and must never become a second independently built channel.

The `native-signing` environment may be used only from `main` and `v*` refs.
The `native-release` environment permits only `v*` refs and requires a reviewer.
Repository Actions default to read-only permissions; individual jobs request
write access only for attestations, packages, or release publication.

## Failure and rollback

If verification fails, stop before publication. If a preview fails, remove its
channel pointer but retain its immutable evidence according to release
retention policy.

If a promoted release fails:

1. stop widening the cohort;
2. restore the last known-good appcast atomically;
3. prevent uninstalled nodes from seeing the failed candidate;
4. restore already-updated nodes only through a separately verified prior
   package path;
5. preserve the node SQLite queue, identity, profiles, and handoff evidence;
6. do not resubmit ambiguous jobs;
7. record affected versions, nodes, jobs, and rollback result.

Changing an appcast does not prove installed nodes rolled back. The release is
not recovered until local health, executor handshake, control-plane reconnect,
and queue reconciliation have been observed.

## Current blockers

- `v0.1.11 (14)` completed universal macOS compilation, Developer ID signing,
  Apple notarisation and stapling for the app, PKG and DMG, Sparkle Ed25519
  signing, checksum/SBOM generation, repository-bound provenance validation,
  protected promotion and public-feed smoke checks in GitHub Actions run
  `31287558654` at commit
  `ce309cecd7740a7d8b43bc79aa8ecf67671e6a21`.
- The public stable macOS appcast and shared manifest now use the corrected
  `/releases/stable/` paths and advertise `0.1.11`. The earlier `0.1.0` feed
  defect is closed.
- No Windows appcast is published; its stable route correctly remains absent
  while Windows remains Disabled for production. Unsigned Windows builds may
  be used for Preview evaluation, but Microsoft Artifact Signing identity
  validation, a certificate profile and signed candidate evidence are still
  required before publishing a Windows updater feed.
- macOS Sparkle replaces the app bundle; the relaunched app transactionally
  activates its embedded, matching Rust agent and executor after an idle gate.
- Windows WinSparkle source integration still needs signed Windows CI,
  clean-install evidence, busy-node deferral evidence, and rollback evidence.
- Coordinated update restoration still needs destructive fault-injection
  evidence before it can move beyond Preview.
- Signed clean-install and physical-printer matrices remain open.
- The published macOS DMG is labelled Preview. No platform or automatic-update
  feature should be advertised as Supported until
  [`release/support-matrix.yaml`](../../release/support-matrix.yaml) records
  the required installation, physical-printer, busy-queue, upgrade, health,
  and rollback evidence.

See [Node updates](../nodes/updates.md) and
[Release checks](../contributing/releases.md).
