# Windows node

**Release tier:** Disabled for production; preview-only on Windows 10/11
x86-64. The Inno Setup workflow can produce a signed release candidate and uses
a per-user login launcher, not a Windows Service. A signed candidate remains a
Preview until its Windows installation, update, rollback, and printer evidence
gates pass.

**Implemented in source:** printer discovery, authenticated tray status,
queue/profile menus, native create/edit/clone through the driver's
`DocumentPropertiesW` UI, immutable full DEVMODE capture (including private
driver bytes), and PDFium-to-GDI replay foundations.

**Not release-tested:** signed install/upgrade/uninstall, Windows Service
lifecycle, clean-login startup, physical PDF/RAW matrices, OKI production
stock, spooler restart, and long-duration reliability. Therefore implementation
does not equal Supported Windows printing.

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

For complex PostScript drivers, Add Profile opens the manufacturer's genuine
advanced property sheet. Edit restores the prior DEVMODE before opening it.
Saving creates an immutable revision; it does not change global queue defaults.

See [`architecture/windows-pdf-helper.md`](../architecture/windows-pdf-helper.md),
[complex drivers](../printing/complex-drivers.md), and
[`operations/agent-service-installation.md`](../operations/agent-service-installation.md).
