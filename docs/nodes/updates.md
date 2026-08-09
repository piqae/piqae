# Node updates

**Status:** update policy, signed-metadata foundations, and a protected release
origin exist. Automated signed full-node updating is not a Supported release
feature. [`release/support-matrix.yaml`](../../release/support-matrix.yaml) is
authoritative.

## Stable public locations

Piqae reserves these stable, unauthenticated HTTPS locations:

```text
https://downloads.piqae.com/releases/stable/appcast-macos.xml
https://downloads.piqae.com/releases/stable/appcast-windows.xml
https://downloads.piqae.com/releases/stable/<signed-artifact>
```

Preview candidates use the same layout below `/releases/preview/`. The web
service validates the requested channel and filename, then redirects to a
short-lived URL for the separate Cloudflare R2 release bucket. Print documents never
use this bucket.

An explicitly unsigned Windows evaluation build is published only from a
protected `v<version>-windows-preview.<number>` tag after checksum, SBOM,
provenance, and unsigned-binary evidence pass. Its preview manifest labels the
package unsigned, and the workflow never publishes a Windows appcast or any
stable-channel object. Windows may display Unknown publisher or SmartScreen
warnings; automatic updates remain disabled.

A reserved URL is not evidence that a release exists. Until the bucket
credentials and a signed artifact are present, the route returns not found and
the Downloads page must not offer a supported download.

## Current platform behaviour

### macOS

Credentialed builds embed Sparkle 2, check the signed feed in the background,
and expose **Check for Piqae Update…**. Sparkle validates the update
archive's Ed25519 signature declared by the appcast in addition to macOS code
signing, and postpones replacement while the local queue, active work, or
profile editor is busy. Download and installation still require operator
confirmation.

Sparkle replaces the signed app bundle, which carries matching signed agent and
executor components. On relaunch Piqae performs an idle-gated, rollback-safe
native component activation. The app and durable node remain separate process
boundaries; durable identity and queue data are not replaced.
Silent installation remains disabled. The per-user installer remains the
recovery path for a missing or damaged installation.

### Windows

The release workflow can sign an installer, sign its bytes for WinSparkle, and
generate `appcast-windows.xml`. The installed tray dynamically loads the
exact pinned WinSparkle runtime only after validating its digest, HTTPS feed,
canonical Ed25519 public key, and trusted configuration tuple. It exposes a
manual update check and honours notify/automatic-check policy.

The install handoff remains fail-closed unless the node is paused, has no
active jobs, and has no profile capture open. The source integration has passed
cross-compilation and Rust tests, but not Windows MSVC/Inno, signed
clean-install, native UI, busy-queue, upgrade, or rollback evidence.

Windows remains Development/Disabled for production until WinSparkle
runtime validation on Windows, signed installation, restart recovery,
rollback, and physical-printer gates pass.

### Linux

Linux remains an operator-managed Preview source/package upgrade. No automatic
native updater is claimed.

## Operator-managed upgrade

Until a platform reaches Supported:

1. Drain or pause new work.
2. Record active and `delivery_uncertain` jobs.
3. Back up the agent data directory and configuration.
4. Verify the archive checksum and platform signature from a trusted channel.
5. Stop the node, replace matching-version components, and preserve
   identity/state.
6. Start the node and verify health, printers, profiles, queue recovery, and
   control-plane reconnection.
7. Submit one controlled profile test.
8. Keep the prior package and evidence until the observation window passes.

Checksums detect transfer corruption only; they do not replace Apple,
Authenticode, or update-metadata signatures. Never run an update command
received through a print job, webhook, support bundle, or log.

## Rollback semantics

The stable appcast is a channel pointer, not a backup system. Versioned
artifacts and their checksums, SBOMs, signatures, notarisation evidence, and
release records must remain immutable.

- Before node pickup, withdraw a bad candidate and restore the last known-good
  stable appcast.
- After installation, use the platform's verified prior package and preserve
  the SQLite queue, identity, and native handoff evidence.
- Never roll back by deleting state, running a down-migration, or automatically
  resubmitting a `delivery_uncertain` job.
- Do not assume Sparkle or WinSparkle will install a lower version through a
  normal feed. A tested downgrade or package-restoration path is separate
  release evidence.

The shared Rust guardian now implements the platform-neutral safety boundary:
checksummed durable command/state journaling, signed-metadata and artifact
validation, paused/idle admission, versioned staging and activation intents,
bounded health checks, restart reconciliation, and automatic rollback
coordination. Its platform runtime interface is intentionally incapable of
choosing an unverified package.

The macOS and Windows packages do not yet activate the full-node guardian
interface. Sparkle/WinSparkle app replacement and the operator-run package
installer remain Preview paths. Whole-node automatic activation becomes
Supported only after each platform wires its signed installer to the guardian
and passes clean-install, busy-queue, interrupted-activation, restart, health,
and rollback evidence.

Server protocol N and N-1 compatibility is the target policy. Upgrade servers
before a broad node rollout. See [native release publishing](../operations/native-release-publishing.md),
[platform upgrades](../operations/upgrades.md), and
[`contributing/releases.md`](../contributing/releases.md).
