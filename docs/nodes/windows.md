# Windows node

**Release tier:** Disabled for production; preview-only on Windows 10/11
x86-64. The Inno Setup workflow can produce a signed release candidate and uses
a crash-loop-limited, SID-scoped per-user login supervisor, not a Windows
Service. Concurrent sessions share one durable agent while retaining and
independently restoring a tray in each interactive session. The agent runs only
while that user remains logged in. A signed
candidate remains a Preview until its Windows installation, update, rollback,
and printer evidence gates pass.

**Implemented in source:** printer discovery, authenticated tray status,
queue/profile menus, native create/edit/clone through the driver's
`DocumentPropertiesW` UI, immutable full DEVMODE capture (including private
driver bytes), and PDFium-to-GDI replay foundations.

The standalone tray displays the revisioned node name and optional
site/location and opens an authenticated local editor. Its default is the
Windows computer name; Piqae never substitutes the logged-in username or an
address. These fields are display metadata and changing them does not replace
the durable node, DPAPI/Credential Manager keys, printer routes, profiles, or
connections.

The Windows standalone node is the common user-managed host for zero to many
connections. Embedded .NET applications may prefer or require its
OS-authenticated broker, or explicitly own an isolated app-scoped runtime. A
broker presence probe is not authorization: attach occurs only after the
verified application principal receives capability-scoped approval.

**Not release-tested:** signed install/upgrade/uninstall, Windows Service
lifecycle, clean-login startup, physical PDF/RAW matrices, OKI production
stock, spooler restart, and long-duration reliability. Therefore implementation
does not equal Supported Windows printing.

The host-policy and local identity paths are covered by Rust and simulated
client tests, but Windows hosted-runner validation, Authenticode signing,
signed installer upgrade/uninstall, per-user login/restart, native broker peer
identity, sleep/resume and physical printer certification remain release gates.

Cloud document-decryption keys are stored for the current user in Windows
Credential Manager, whose credential data is protected by Windows. P-256 key
material is read back and verified before a legacy plaintext file is removed.
The agent fails closed if the credential store or migration fails;
it does not generate a replacement that would make queued encrypted jobs
unreadable. This is OS-backed at-rest protection, not evidence of a
non-exportable TPM-backed P-256 key.

The tray and profile host must run in the interactive user session. The local
agent API remains loopback-only; a Windows node does not connect directly to a
Mac's `127.0.0.1` service. Both nodes enrol with the same hosted/self-hosted
control plane.

Agent, tray, and launcher logs live under `%LOCALAPPDATA%\Spool\logs`. Agent and
tray files rotate at approximately 5 MiB and retain four prior generations;
launcher failures rotate at 1 MiB and retain two. Upgrades append to the active
generation and preserve the retained history.

For complex PostScript drivers, Add Profile opens the manufacturer's genuine
advanced property sheet. Edit restores the prior DEVMODE before opening it.
Saving creates an immutable revision; it does not change global queue defaults.

See [`architecture/windows-pdf-helper.md`](../architecture/windows-pdf-helper.md),
[complex drivers](../printing/complex-drivers.md), and
[`operations/agent-service-installation.md`](../operations/agent-service-installation.md).
