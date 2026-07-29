# Spool macOS menu shell

This is a small AppKit status-menu client for the headless Spool agent. It
contains no print driver, queue, database, device credential, or cloud client.
Operational actions use the authenticated loopback API and time out quickly
when the agent is unavailable.

Configuration:

- `SPOOL_LOCAL_API_URL` defaults to `http://127.0.0.1:39100` and must remain an
  HTTP loopback URL.
- `SPOOL_LOCAL_TOKEN_FILE` selects the agent's `local.token`.
- Otherwise `SPOOL_DATA_DIR/local.token` is used when `SPOOL_DATA_DIR` is set.
- The macOS fallback is
  `~/Library/Application Support/Spool/local.token`.
- `SPOOL_DASHBOARD_URL` enables **Manage Printers** and **Open Dashboard**.
- `SPOOL_AGENT_LOG_FILE` overrides `/var/log/spool-agent.log`.

Build and test:

```sh
swift test --package-path shells/macos
shells/macos/build-app.sh
open shells/macos/build/Spool.app
```

The default generated app is an unsigned Preview build. Its bundle metadata
explicitly disables Sparkle, and the menu reports that updates are unavailable.
It does not bypass Gatekeeper.

`build-app.sh` accepts `SPOOL_VERSION`, `SPOOL_BUILD_NUMBER`, and
`SPOOL_APP_BUNDLE`. A release build may additionally provide all three of
`SPOOL_CODE_SIGN_IDENTITY`, `SPOOL_SPARKLE_FEED_URL`, and
`SPOOL_SPARKLE_PUBLIC_ED_KEY`. The identity must be an available Developer ID
Application identity and the feed must use HTTPS. Partial update configuration
fails closed. Release builds embed Sparkle 2.9.2 and sign its nested code before
signing the app with the hardened runtime.

The dedicated `macos-release.yml` workflow builds arm64/x86_64 binaries, runs
the Swift suite, creates an SPDX SBOM and checksums, and records provenance.
With no credentials, a manually dispatched run produces only artifacts whose
names and evidence say `unsigned-preview`; update checks remain disabled. A tag
fails unless the complete Developer ID, Apple notarisation, and Sparkle Ed25519
secret set is present. Credentialed runs verify Developer ID signing, notarise
and staple the app, generate an appcast, and verify the update archive's
Ed25519 signature with the public key. The workflow uploads CI artifacts but
does not publish a GitHub release or deploy the appcast.

## Per-user package

`packaging/macos/build-user-package.sh` combines an app, `spool-agent`, and
`spool-executor-cups` into a per-user ZIP. Its installer uses:

- `~/Applications/Spool.app`;
- `~/Library/Application Support/Spool/bin` for the agent and executor;
- `~/Library/Application Support/Spool` for durable identity, queue, and local
  API token;
- `~/Library/LaunchAgents` for separate agent and menu launch agents; and
- `~/Library/Logs/Spool/agent.log`.

The installer must run as the desktop user without `sudo`. Before replacing a
loaded node it authenticates to the existing loopback status endpoint and
refuses the handoff while jobs are queued or active, or when idle state cannot
be verified. It retains the previous app and preserves data on uninstall.
Unsigned packages are labelled Preview and do not remove quarantine or apply a
Gatekeeper workaround.

Sparkle currently replaces only `Spool.app`, including the menu and
`SpoolPrintCoreReplay`. It does not replace or restart the separately installed
Rust agent/executor. A full-node update therefore still uses the per-user
package's idle-checked installer. Do not represent the Sparkle foundation as an
atomic full-node updater.

Automatic Sparkle checks and silent updates are disabled. A credentialed build
adds **Check for Updates…**. If installation reaches the relaunch boundary
while the local API reports queued/active jobs or a profile panel is open, the
menu postpones replacement and polls the authenticated local status until the
node is idle. An unavailable agent is not assumed idle.

The **Local driver test…** action requires an exposed logical printer, a named
print profile, and explicit confirmation. It never falls back to unprofiled
job submission.

## Native print profiles

Every printer menu contains its saved profiles and **Add Profile…**. A profile
can be edited or cloned from its submenu. These actions open the real macOS
`NSPrintPanel` for that destination with **Save Profile** as the confirmation
button. The profile host does not create an `NSPrintOperation`, load a customer
document, or submit anything to the spooler during capture.

The host stores three complementary representations: the complete
property-list-safe `NSPrintInfo.printSettings` dictionary, PrintCore's
`PMPrintSettings` data representation, and its `PMPageFormat` representation.
The profile name, optional stock assignment, safe API overrides, destination
fingerprint, and a portable page summary accompany the opaque native data.

The shell expects these authenticated loopback routes:

```text
POST   /v1/local/printers/{printer_id}/profile-capture-sessions
POST   /v1/local/profile-capture-sessions/{session_id}/complete
DELETE /v1/local/profile-capture-sessions/{session_id}
```

The first response supplies a short-lived, single-use `capture_token`; complete
and cancel send it in `X-Spool-Capture-Token`. Edit and clone sessions also
return the current `native_configuration`, allowing the panel to begin with
the exact saved native settings. Native configurations are capped at 1 MiB in
the shell and must remain local to the agent.

## Headless PrintCore replay

`SpoolPrintCoreReplay` is the bounded, local-only PDF handoff helper used when
a job pins a `macos_printcore` profile. It restores the captured
`NSPrintInfo.printSettings`, `PMPrintSettings`, and `PMPageFormat`, binds the
exact requested `NSPrinter`, applies only allowlisted stable job overrides, and
runs a PDFKit `NSPrintOperation` with both the print and progress panels hidden.
It works on a private `NSPrintInfo` copy and never changes the driver defaults.

Build it directly with:

```sh
swift build --package-path shells/macos -c release --product SpoolPrintCoreReplay
```

The helper reads exactly one JSON value from stdin (maximum 2 MiB):

```json
{
  "printer_native_id": "HP_LaserJet",
  "pdf_path": "/absolute/spool/content/job.pdf",
  "job_title": "Packing slip 1234",
  "native_profile": {
    "kind": "macos_printcore",
    "schema_version": 1,
    "digest": "sha256:<64 lowercase hex characters>",
    "blob_base64": "<base64 JSON LocalMacNativeConfiguration>"
  },
  "portable_options": {
    "copies": 2,
    "collate": true,
    "duplex": "long-edge",
    "fit_to_page": true,
    "pages": "1-2",
    "paper": "iso-a4",
    "rotate": "0",
    "native_options": {}
  },
  "safe_overrides": [
    "copies",
    "collate",
    "duplex",
    "fit_to_page",
    "pages",
    "paper",
    "rotate"
  ]
}
```

`bin`, `color`, `dpi`, `media`, `nup`, and arbitrary `native_options` are
rejected even if allowlisted: macOS has no stable public AppKit/PrintCore job
override for them, and replacing driver-owned dictionary keys would corrupt
exact-profile semantics. Save those choices into the native profile instead.
Page ranges are one page or one ascending contiguous range.

Stdout is one bounded JSON response. Success means AppKit accepted the
synchronous operation; PrintCore does not expose a reliable native queue job ID
through this API:

```json
{"ok":true,"retryable":false,"handoff_may_have_succeeded":false}
```

Failures include a stable `code`, a bounded `message`, `retryable`, and
`handoff_may_have_succeeded`. A failure after `NSPrintOperation.run()` begins is
marked ambiguous and must not be retried automatically. The process exits zero
on success and one on failure. The app bundle builder embeds the helper at
`Spool.app/Contents/MacOS/SpoolPrintCoreReplay`.
