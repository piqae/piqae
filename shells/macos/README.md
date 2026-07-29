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
