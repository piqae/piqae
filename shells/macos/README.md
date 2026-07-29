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

The generated app is unsigned and intended for source builds. The shell does
not attempt signing, notarisation, installation, or login-item registration.
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
