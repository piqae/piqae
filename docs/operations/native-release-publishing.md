# Native release publishing

**Status:** protected hosting and fail-closed package workflows are being
connected. No Piqae native release is currently Supported or published as a
stable signed download.

This runbook separates four states:

1. **built** — a workflow produced a candidate;
2. **verified** — checksums, SBOM, provenance, platform signature, update
   signature, and applicable notarisation evidence passed;
3. **published** — immutable files exist in the release bucket;
4. **promoted** — the stable appcast and web download manifest reference those
   exact files.

Reaching an earlier state never implies a later one.

## Storage layout

Native release artifacts use the dedicated Railway bucket `piqae-releases`.
Customer print documents use `spool-documents`; credentials and retention
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

Do not put signing keys, bucket credentials, notarisation credentials, or
release tokens in appcasts, workflow artifacts, command arguments, or logs.
The web service uses its release-origin credentials only for reads. Enforce
read-only scope where Railway exposes it; otherwise isolate them to this bucket
and never reuse the publisher credential. Publishing uses a separate
write-capable credential.

## CI signing authority

GitHub remains the release orchestrator even when Blacksmith supplies the
runner compute. Keep two separate GitHub environments:

- `native-signing` exposes only platform code-signing, notarisation, and update
  signing material to the macOS and Windows candidate jobs.
- `native-release` exposes only the dedicated Railway release-bucket
  credentials to the serialized promotion jobs.

The macOS signing environment requires:

```text
MACOS_CERTIFICATE_P12_BASE64
MACOS_CERTIFICATE_PASSWORD
MACOS_APPLICATION_IDENTITY
APPLE_ID
APPLE_TEAM_ID
APPLE_APP_PASSWORD
SPARKLE_PRIVATE_KEY_BASE64
SPARKLE_PUBLIC_ED_KEY
```

The Windows signing environment requires:

```text
WINDOWS_AUTHENTICODE_PFX_BASE64
WINDOWS_AUTHENTICODE_PFX_PASSWORD
WINDOWS_RFC3161_TIMESTAMP_URL
WINSPARKLE_ED25519_PRIVATE_KEY_BASE64
WINSPARKLE_ED25519_PUBLIC_KEY
```

The Sparkle and WinSparkle private keys need encrypted offline recovery copies.
Losing an update key after clients trust its public key can strand installed
nodes. Do not rely on a GitHub secret as the only copy. Certificate and
notarisation credentials must be rotated independently from the update keys.

Unsigned manual builds remain private workflow artifacts. A tag or a manual
`publish=true` request fails unless the complete platform signing set exists.
Stable promotion then enters `native-release`, verifies each immutable S3
object's SHA-256 and length, promotes the installer before its appcast, and
promotes the combined manifest last.

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

- macOS build `0.1.0 (7)` completed Developer ID signing for the app, agent,
  executor, and installer; Apple notarisation/stapling for the app and DMG;
  Sparkle Ed25519 signing; immutable publication; and public checksum
  verification in GitHub Actions run `30507639987`.
- macOS Sparkle replaces the app bundle, not the separately installed Rust
  node and executor.
- Windows WinSparkle source integration still needs a Windows CI run,
  clean-install evidence, busy-node deferral evidence, and rollback evidence.
- Atomic full-node update restoration has no release evidence.
- Signed clean-install and physical-printer matrices remain open.
- The published macOS DMG is labelled Preview. No platform or automatic-update
  feature should be advertised as Supported until
  [`release/support-matrix.yaml`](../../release/support-matrix.yaml) records
  the required installation, physical-printer, busy-queue, upgrade, health,
  and rollback evidence.

See [Node updates](../nodes/updates.md) and
[Release checks](../contributing/releases.md).
