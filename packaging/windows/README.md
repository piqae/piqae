# Spool for Windows — development installer

The Windows package is a **per-user development installation**. It installs the
agent, executor, native driver-profile host, CLI, and notification-area shell
under `%LOCALAPPDATA%\Programs\Spool`, then starts the agent and shell at login
through the current user's `Run` registry key.

It does not register `spool-agent.exe` with the Service Control Manager. The
agent is currently a console process and does not implement the SCM lifecycle.
Machine-wide unattended printing, signed distribution, automatic updates, and
service-session-to-user-session profile UI are not claimed by this package.

## Install and connect

Run `spool-windows-x86_64-setup.exe` as the Windows user who will configure the
printer. Setup opens **Configure Spool** after copying the files.

The configuration wizard supports:

- **Local mode**, which discovers and prints to drivers installed for that user
  without connecting to a control plane.
- **Connected mode**, which consumes a short-lived one-time enrolment token
  against an HTTPS Spool control-plane URL. The private Ed25519 device key is
  generated on the Windows computer and is never sent to the server.

Configuration and durable queue data live in `%LOCALAPPDATA%\Spool`. The wizard
removes inherited ACLs from the device key and grants access only to the current
user. Uninstall deliberately preserves this directory so an accidental
uninstall cannot destroy queued-job evidence or an enrolled identity.

The control-plane URL is the Rust server origin, not the Svelte dashboard URL
and not another computer's loopback agent URL. A server on another computer
must listen on a LAN-reachable interface, have firewall access, and preferably
use HTTPS with a certificate trusted by Windows. `127.0.0.1` and `localhost`
always refer to the Windows computer itself. For a short trusted-LAN development
test the wizard can explicitly permit HTTP after showing a transport warning.
The optional dashboard URL only controls the tray's **Open Spool** action; it
does not connect or enrol the Windows agent.

## Native driver profiles

`spool-profile-host-windows.exe` calls the installed driver's genuine
`DocumentPropertiesW` property sheet and captures the complete
`DEVMODEW`/private driver bytes. This is the path used for advanced PostScript
and vendor options such as the OKI Pro1050's stock, feed, alignment, colour, and
finishing controls.

PDF jobs pinned to a native profile use the packaged PDFium renderer and a GDI
printer device context created from that exact immutable DEVMODE. Before every
handoff Spool checks the queue/driver fingerprint, asks the installed driver to
revalidate and normalize the captured bytes, and applies only the profile's
explicitly allowed per-job public overrides. Opaque vendor bytes remain under
the driver’s control.

This is the intended exact replay path, but it remains a development support
tier until it passes physical OKI and Windows restart/recovery gates. The
generic SumatraPDF compatibility path cannot apply opaque vendor DEVMODE
settings and is never substituted for a pinned native profile.

## Operations

- Reconfigure: Start menu → **Spool → Configure Spool**
- Start: Start menu → **Spool → Start Spool**
- Stop: Start menu → **Spool → Stop Spool**
- Local API: `http://127.0.0.1:39100`
- Logs: `%LOCALAPPDATA%\Spool\logs`
- Uninstall: Windows Settings → Apps → Installed apps → Spool

To remove state after uninstall, first confirm no jobs or enrolled identity must
be retained, then manually remove `%LOCALAPPDATA%\Spool`.

## Building

GitHub Actions builds the binaries with the MSVC Rust target and compiles
`Spool.iss` with Inno Setup. Run the **Release** workflow manually for a
versioned dry-run artifact, or download the `spool-windows-installer` artifact
from a CI run. With GitHub CLI authenticated on a development computer:

```console
gh run list --workflow CI
gh run download RUN_ID --name spool-windows-installer --dir ~/Downloads/Spool-Windows
```

The artifact contains the installer and its SHA-256 sidecar. Release archives
are unsigned until the signing gate is supplied.

The package fetches `pdfium-win-x64.tgz` from the open-source
`bblanchon/pdfium-binaries` Chromium 7961 release and accepts it only when its
SHA-256 is
`88276459349b291c41f10422dad0210f007c04d919c8fa56472b6b7c6406adf4`.
The PDFium binary and all licenses from that distribution are included in the
installer.
