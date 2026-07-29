# Windows node

**Release tier:** Disabled for production; development-only on Windows 10/11
x86-64. The Inno Setup package is unsigned and uses a per-user login launcher,
not a Windows Service.

**Implemented in source:** printer discovery, authenticated tray status,
queue/profile menus, native create/edit/clone through the driver's
`DocumentPropertiesW` UI, immutable full DEVMODE capture (including private
driver bytes), and PDFium-to-GDI replay foundations.

**Not release-tested:** signed install/upgrade/uninstall, Windows Service
lifecycle, clean-login startup, physical PDF/RAW matrices, OKI production
stock, spooler restart, and long-duration reliability. Therefore implementation
does not equal Supported Windows printing.

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
