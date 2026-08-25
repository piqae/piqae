# Piqae macOS menu shell

This is a small AppKit status-menu client for the headless Piqae node. It
contains no print driver, queue, database, device credential, or cloud client.
Operational actions use the authenticated loopback API and time out quickly
when the agent is unavailable.

`PiqaeMenuCore` includes strict parsing for the reserved native connect-link
shape, HTTPS-only return destinations (with loopback HTTP for development),
explicit non-empty printer/permission consent, and capability-hash replay
suppression without retaining the capability. The app understands the verified
`/connect` link shape and the `piqae://` compatibility route. Developer ID
builds do not currently claim Associated Domains because that restricted
entitlement requires an embedded provisioning profile; browser handoff remains
the supported connection path until that profile is provisioned. The app
uses the headless agent's bounded stdin-only preview and acceptance commands;
the capability never enters process arguments, environment variables, files,
or diagnostics. Piqae resolves the requesting workspace before showing local
consent, requires the queue to be idle, starts with every printer unchecked,
persists the isolated connector before restarting the agent, and follows only
a server-validated return URL after success. Windows and Linux application-link
registration remain gated on equivalent signed-shell consent flows.

An unconnected install remains fully local and offers **Connect Piqae
Account…**. Integrators do not use that account action: their backend creates a
short-lived node-connect session through the SDK and opens its `connect_url` in
their own UI. The browser keeps the capability in a fragment and hands it to
the registered app scheme; the end user needs no Piqae account. Each accepted
service receives separate credentials, storage and explicit printer grants.

Configuration:

- `PIQAE_LOCAL_API_URL` defaults to `http://127.0.0.1:39100` and must remain an
  HTTP loopback URL.
- `PIQAE_LOCAL_TOKEN_FILE` selects the agent's `local.token`.
- Otherwise `PIQAE_DATA_DIR/local.token` is used when `PIQAE_DATA_DIR` is set.
- The macOS fallback is
  `~/Library/Application Support/Spool/local.token`. This internal path remains
  stable across the visible Piqae rename so existing identities and queues are
  not stranded.
- **Print History…** opens the searchable, filterable retained history view.
  **Connections → View Connections…** opens the separate connection ownership
  and recovery view. Both request
  a short-lived, single-use browser handoff, so the local API token is never put
  in a browser URL. `PIQAE_DASHBOARD_URL` is only a compatibility fallback for
  older agents that do not yet provide that handoff.
- `PIQAE_CONNECTIONS_URL` may send **Connections → Manage Access &
  Reauthorize…** to a verified service-neutral management page. Without it,
  the action opens connector details in the node dashboard through the same
  authenticated handoff.
- `PIQAE_AGENT_LOG_FILE` overrides `/var/log/piqae-agent.log`.

Build and test:

```sh
swift test --package-path shells/macos
shells/macos/build-app.sh
open shells/macos/build/Piqae.app
```

The default generated app is an unsigned Preview build. Its bundle metadata
explicitly disables Sparkle, and the menu reports that updates are unavailable.
It does not bypass Gatekeeper.

`build-app.sh` accepts `PIQAE_VERSION`, `PIQAE_BUILD_NUMBER`, and
`PIQAE_APP_BUNDLE`. A release build may additionally provide all three of
`PIQAE_CODE_SIGN_IDENTITY`, `PIQAE_SPARKLE_FEED_URL`, and
`PIQAE_SPARKLE_PUBLIC_ED_KEY`. The identity must be an available Developer ID
Application identity and the feed must use HTTPS. Partial update configuration
fails closed. Release builds embed Sparkle 2.9.2 and sign its nested code before
signing the app with the hardened runtime.

The reusable `macos-release.yml` stage builds arm64/x86_64 binaries, runs the
Swift suite, creates an SPDX SBOM and checksums, and records provenance. The
single **Piqae release** orchestrator can request a non-publishing manual
candidate. When credentials are absent, its names and evidence say
`unsigned-preview` and update checks remain disabled. A tag fails unless the
complete Developer ID, Apple notarisation, and Sparkle
Ed25519 secret set is present. Credentialed runs Developer ID-sign the app,
node, executor, and installer; notarise and staple both the app and installer
disk image; generate an appcast; and verify the update archive's Ed25519
signature with the public key. A publication run uploads immutable artifacts
to the release bucket before promoting the appcast and shared manifest.

## Per-user package

`packaging/macos/build-user-package.sh` combines an app, `piqae-agent`, and
`piqae-executor-cups` into a notarised per-user DMG and a diagnostic ZIP. A
normal user opens the DMG and double-clicks **Install Piqae**; no Terminal or
administrator access is required. Its installer uses:

- `~/Applications/Piqae.app`;
- `~/Library/Application Support/Spool/bin` for the agent and executor;
- `~/Library/Application Support/Spool` for durable identity, queue, and local
  API token (a deliberately stable, internal compatibility path);
- `~/Library/LaunchAgents` for separate agent and menu launch agents; and
- `~/Library/Logs/Spool/agent.log`.

The installer runs as the desktop user without `sudo`. Before replacing a
loaded node it authenticates to the existing loopback status endpoint and
refuses the handoff while jobs are queued or active, or when idle state cannot
be verified. Signed packages fail closed unless the app, node, and executor all
retain valid Developer ID signatures. It retains the previous app and
preserves data on uninstall. Unsigned packages are labelled Preview and do not
remove quarantine or apply a Gatekeeper workaround.

Sparkle replaces the signed `Piqae.app`. Release app bundles also carry the
matching signed Rust agent and executor. On relaunch, the menu verifies their
version and Developer ID signatures, requires authenticated idle queue status,
stages both on the destination filesystem, switches them together, restarts the
LaunchAgent, and restores both previous binaries if health validation fails.
Identity, connector credentials, profiles, content, and queue data are never
part of the replacement set. App-bundle rollback remains Sparkle's boundary;
the separately activated node-component transaction is deliberately
backwards-compatible across adjacent releases.

Credentialed builds check the signed Sparkle feed in the background and add
**Check for Piqae Update…**. Sparkle still requires the operator to accept
the download and installation; silent installation is disabled. When a release
is found, the menu names its version and waits for authenticated idle status
before the coordinated app and node-component handoff.
If installation reaches the relaunch boundary while the local API reports
queued/active jobs or a profile panel is open, the menu shows that the app
update is waiting for idle and polls authenticated local status until the node
is idle. An unavailable agent is not assumed idle.

Each preset has its own **Test “Preset”…** action and explicit confirmation. It
does not require a cloud connection and never falls back to unprofiled job
submission.

## Print presets (native profiles)

The menu calls native print profiles **Print Presets** because each one can
include paper, tray, colour, duplex, resolution, and vendor-specific settings.
The API and storage model retain the precise `profile` terminology. Every
printer menu contains its presets and **Add Print Preset…**. A preset can be
edited or duplicated from its submenu. These actions open the real macOS
`NSPrintPanel` for that destination with **Save Preset** as the confirmation
button. The profile host does not create an `NSPrintOperation`, load a customer
document, or submit anything to the spooler during capture.

Printers are locally available to connected services by default. A service's
consent screen remains the authority for whether it receives all printers
(including printers added later) or only selected printers; the menu does not
present a competing global exposure toggle.

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
and cancel send it in `X-Piqae-Capture-Token`. Edit and clone sessions also
return the current `native_configuration`, allowing the panel to begin with
the exact saved native settings. Native configurations are capped at 1 MiB in
the shell and must remain local to the agent.

## Headless PrintCore replay

`PiqaePrintCoreReplay` is the bounded, local-only PDF handoff helper used when
a job pins a `macos_printcore` profile. It restores the captured
`NSPrintInfo.printSettings`, `PMPrintSettings`, and `PMPageFormat`, binds the
exact requested `NSPrinter`, applies only allowlisted stable job overrides, and
runs a PDFKit `NSPrintOperation` with both the print and progress panels hidden.
It works on a private `NSPrintInfo` copy and never changes the driver defaults.

Build it directly with:

```sh
swift build --package-path shells/macos -c release --product PiqaePrintCoreReplay
```

The helper reads exactly one JSON value from stdin (maximum 2 MiB):

```json
{
  "printer_native_id": "HP_LaserJet",
  "pdf_path": "/absolute/piqae/content/job.pdf",
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
`Piqae.app/Contents/MacOS/PiqaePrintCoreReplay`.
