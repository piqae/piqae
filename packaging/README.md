# Native source-bundle packaging

These files are operator-reviewed templates, not signed installers. Native
artifacts are source-usable release foundations with the following support
tiers:

- Linux agent, CUPS executor, and tray shell: **Preview**.
- macOS agent, CUPS executor, and menu shell: **Preview**.
- Windows agent and interactive tray shell: **Disabled** for production, with
  an unsigned per-user development installer for driver/profile integration
  tests.

Preview means the binaries and service template are testable by an operator; it
does not mean unattended upgrades, code signing, notarisation, or distribution
package lifecycle are complete. Disabled means no production use or service
installation is claimed.

The platform directories contain:

- `linux/`: systemd, environment, tmpfiles, and sysusers templates.
- `macos/`: a LaunchDaemon plist template that must be rendered and installed
  by an administrator.
- `windows/`: per-user installer scripts plus explicit production limitations;
  it does not register the console agent as a Windows service.

Read `docs/operations/agent-service-installation.md` before installing a source
bundle.
