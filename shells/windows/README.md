# Piqae Node Windows shell

The Windows V1 shell is a Win32 notification-area process, separate from the
Windows Service or user-mode agent. Its only stable dependency is the local IPC
V1 contract documented in `docs/architecture/local-agent-control.md`.

The shell is a subsystem-Windows Rust binary using `Shell_NotifyIconW` and the
agent's authenticated HTTP loopback API. Its menu reports node status, lists
each discovered Windows queue and immutable profile, opens the dashboard, and
starts create/edit/clone native profile capture. It contains no queue, cloud or
printing code.

`PIQAE_LOCAL_API_URL` defaults to `http://127.0.0.1:39100` and rejects
non-loopback origins. `PIQAE_LOCAL_TOKEN_FILE` can point at the agent's
`local.token`; otherwise the shell uses `PIQAE_DATA_DIR/local.token`, then
`%ProgramData%\Piqae\local.token`. `PIQAE_DASHBOARD_URL` controls the Open Piqae
action.

## Native profile host

Driver configuration is delegated to the separately built
`piqae-profile-host-windows.exe` binary in `crates/executor-windows`. The shell
finds it beside its own executable or at `PIQAE_PROFILE_HOST_PATH`, launches it
in the interactive user's session, writes exactly one JSON request to standard
input, reads exactly one bounded JSON response from standard output, and then
lets the process exit.

The shell and agent integration must:

- obtain a short-lived, single-use capture token from the local agent;
- pass that token in both `PIQAE_PROFILE_CAPTURE_TOKEN` and the request;
- pass the exact installed queue ID rather than a friendly alias;
- optionally pass the tray window handle so the vendor property sheet is modal
  to the tray UI;
- never log the request or response because they contain the opaque native
  `DEVMODE`;
- enforce a ten-minute execution deadline while still allowing the operator
  time to use a complex vendor dialog;
- treat `cancelled` as a normal operator outcome;
- return the captured envelope directly to the agent for local encrypted-at-rest
  persistence.

The host calls the genuine driver `DocumentPropertiesW` property sheet. It
captures and validates `dmSize + dmDriverExtra`, fingerprints the installed
queue and driver, and can revalidate an existing capture without displaying UI.
It does not print a document or change the queue's global defaults.

The host opens the driver's full `DocumentPropertiesW` UI. This includes
manufacturer PostScript/private controls such as OKI media sensing, feed,
registration, and finishing settings when the installed driver exposes them.
The resulting public and private DEVMODE bytes are stored as an immutable
revision; editing restores the exact prior revision and creates another.

## Update integration boundary

Signed Windows release packaging includes a hash-pinned WinSparkle 0.9.4 x64
runtime and an `update-config.json`. The launcher exports this complete trust
tuple to the tray process:

- `PIQAE_UPDATE_POLICY`
- `PIQAE_UPDATE_FEED_URL`
- `PIQAE_UPDATE_ED25519_PUBLIC_KEY`
- `PIQAE_UPDATE_RUNTIME_VERSION`
- `PIQAE_UPDATE_RUNTIME_SHA256`

The Rust tray loads the DLL only by absolute path from its own installation
directory after verifying its SHA-256 digest. It resolves the required C API,
sets the HTTPS appcast and Ed25519 public key, and initializes WinSparkle only
when the full tuple is valid. `notify` enables the tray's manual **Check for
updates…** action without background checks. `automatic` additionally enables
WinSparkle's periodic checks. Both modes still require operator confirmation;
neither silently installs an update.

Changing policy through the installed **Update policy** action restarts only
the tray, leaving the durable agent and queue running, so the new policy takes
effect immediately. The action fails closed while a native driver profile
dialog is open.

Before handing a downloaded installer to Windows, the tray asks the local
agent for current state. The handoff is allowed only when the node is already
paused, no job is active, and no native profile dialog is open. Queued jobs stay
in the durable agent database. This is a fail-closed release boundary, not the
planned shared update guardian or automatic rollback implementation.

Unsigned preview packages contain no feed, key, or WinSparkle DLL and force the
effective policy to `disabled`. Missing, modified, incomplete, non-HTTPS, or
unsupported updater configuration leaves printing available but does not
initialize WinSparkle. Normal shell shutdown calls `win_sparkle_cleanup()`
before unloading the runtime.
