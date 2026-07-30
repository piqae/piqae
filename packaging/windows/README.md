# Piqae Node for Windows — preview installer

The Windows package is a **per-user preview installation**. It installs the
agent, executor, native driver-profile host, CLI, and notification-area shell
under `%LOCALAPPDATA%\Programs\Piqae` for new installations, then starts the
agent and shell at login
through the current user's `Run` registry key.

It does not register `spool-agent.exe` with the Service Control Manager. The
agent is currently a console process and does not implement the SCM lifecycle.
Machine-wide unattended printing and service-session-to-user-session profile UI
are not claimed by this package.

## Install and connect

Run `piqae-windows-x86_64-setup.exe` as the Windows user who will configure the
printer. Setup opens **Configure Piqae Node** after copying the files.

An upgrade detects the existing `%LOCALAPPDATA%\Spool\config.json`, preserves
the node identity and durable queue, skips the first-run configuration wizard,
and starts the existing node after replacement. A fresh interactive install
opens configuration once. Silent fresh installs intentionally remain
unconfigured until the operator runs **Configure Piqae Node**.

The configuration wizard supports:

- **Local mode**, which discovers and prints to drivers installed for that user
  without connecting to a control plane.
- **Connected mode**, which consumes a short-lived one-time enrolment token
  against an HTTPS Piqae control-plane URL. The private Ed25519 device key is
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
The optional dashboard URL only controls the tray's **Open Piqae** action; it
does not connect or enrol the Windows agent.

## Native driver profiles

`spool-profile-host-windows.exe` calls the installed driver's genuine
`DocumentPropertiesW` property sheet and captures the complete
`DEVMODEW`/private driver bytes. This is the path used for advanced PostScript
and vendor options such as the OKI Pro1050's stock, feed, alignment, colour, and
finishing controls.

PDF jobs pinned to a native profile use the packaged PDFium renderer and a GDI
printer device context created from that exact immutable DEVMODE. Before every
handoff Piqae checks the queue/driver fingerprint, asks the installed driver to
revalidate and normalize the captured bytes, and applies only the profile's
explicitly allowed per-job public overrides. Opaque vendor bytes remain under
the driver’s control.

This is the intended exact replay path, but it remains a development support
tier until it passes physical OKI and Windows restart/recovery gates. The
generic SumatraPDF compatibility path cannot apply opaque vendor DEVMODE
settings and is never substituted for a pinned native profile.

## Operations

- Reconfigure: Start menu → **Piqae → Configure Piqae Node**
- Start: Start menu → **Piqae → Start Piqae Node**
- Stop: Start menu → **Piqae → Stop Piqae Node**
- Local API: `http://127.0.0.1:39100`
- Logs: `%LOCALAPPDATA%\Spool\logs`
- Update policy: Start menu → **Piqae → Update policy**
- Manual update check: tray menu → **Check for updates…** (signed packages only)
- Uninstall: Windows Settings → Apps → Installed apps → Piqae Node

Changing update policy restarts only the disposable tray process so the new
policy becomes active immediately. It does not stop the durable agent or its
local queue, and it refuses to restart the tray while native driver settings
are open.

To remove state after uninstall, first confirm no jobs or enrolled identity must
be retained, then manually remove `%LOCALAPPDATA%\Spool`.

## Building

GitHub Actions builds the binaries with the MSVC Rust target and compiles
`Spool.iss` with Inno Setup. Run the **Release** workflow manually for a
versioned dry-run artifact, or download the `piqae-windows-installer` artifact
from a CI run. With GitHub CLI authenticated on a development computer:

```console
gh run list --workflow CI
gh run download RUN_ID --name piqae-windows-installer --dir ~/Downloads/Piqae-Windows
```

The artifact contains the installer and its SHA-256 sidecar. Release archives
are unsigned until the signing gate is supplied.

The package fetches `pdfium-win-x64.tgz` from the open-source
`bblanchon/pdfium-binaries` Chromium 7961 release and accepts it only when its
SHA-256 is
`88276459349b291c41f10422dad0210f007c04d919c8fa56472b6b7c6406adf4`.
The PDFium binary and all licenses from that distribution are included in the
installer.

## Signing and updates

The dedicated `windows-release.yml` workflow has two explicit modes:

- **Signed release** requires an Authenticode PFX/password, RFC 3161 timestamp
  URL, the exact expected certificate subject and SHA-1 certificate
  thumbprint, a WinSparkle Ed25519 private/public key pair, and an HTTPS appcast
  URL. It signs every Piqae Node executable, the Inno-generated uninstaller,
  and the final installer; verifies both Authenticode validity and the expected
  signer identity; signs the final installer bytes with WinSparkle's official
  companion tool; and generates an appcast.
- **Unsigned preview** is produced when all signing credentials are absent. Its
  artifact name includes `unsigned-preview`; its installed update configuration
  contains no feed or public key, and update policy cannot be enabled.

Partially configured signing fails the workflow. The workflow never silently
downgrades an intended signed release to unsigned.

The non-secret GitHub environment variables are:

- `WINDOWS_RFC3161_TIMESTAMP_URL`
- `WINDOWS_EXPECTED_CERTIFICATE_SUBJECT`
- `WINDOWS_EXPECTED_CERTIFICATE_THUMBPRINT`
- `WINSPARKLE_ED25519_PUBLIC_KEY`

The PFX, PFX password, and WinSparkle private key remain environment secrets.
The expected subject must equal the certificate's full X.509 subject string;
the thumbprint is its 40-character SHA-1 certificate identifier. The release
version must exactly match the Cargo versions of the agent, CLI, Windows
executor/profile host, and tray. Version metadata cannot promote binaries that
report a different version.

WinSparkle 0.9.4's binary distribution is verified against SHA-256
`6037df37fc263bd1650a1c4949681a9d40ffe991d01f35892a406cb5d103c976`.
The x64 runtime inside it is independently pinned to SHA-256
`9b43b1c16ee39fb9a91b5bd75138767898779510e0836be2919250607cdbe8ab`.
Signed packages install that DLL and the Rust tray implements the WinSparkle C
API lifecycle. Unsigned packages omit it.

`notify` enables manual checks and `automatic` enables periodic checks. Both
show WinSparkle's native user confirmation. Installer handoff is denied unless
the node is already paused, has no active job, and is not editing a native
profile. This protects the spooler boundary but is not automatic rollback or a
complete shared update guardian; those remain separate release gates.

Every workflow artifact includes SHA-256 checksums and an SPDX SBOM. GitHub
build provenance is requested for both preview and signed artifacts. Unsigned
previews remain workflow artifacts and can never enter the stable release
bucket.

Signed publication is isolated behind the protected `native-release` GitHub
environment. The workflow uploads immutable versioned objects to the dedicated
Railway S3-compatible release bucket, verifies their recorded size and SHA-256,
and only then promotes the installer and signed appcast under
`https://downloads.piqae.com/releases/stable/`. The appcast is promoted after
its referenced installer, and the shared release manifest is promoted last.
