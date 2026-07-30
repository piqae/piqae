# Agent service installation from a source bundle

The native archives are unsigned Preview/Disabled foundations, not installers.
Verify the archive SHA-256 sidecar and review its SPDX SBOM before installation.
Pin a release version and test printing, restart, and rollback on the target OS.

## URI source policy

Local and hosted URI jobs use the same agent-side fetch policy. Public HTTP(S)
is allowed by default; redirects, embedded URI credentials, cloud metadata,
unspecified/multicast addresses, and non-public destinations are rejected.

`PIQAE_ALLOW_PRIVATE_URI_SOURCES=false` is the secure default. A trusted
local/self-hosted operator may set it to `true` to reach LAN, loopback, or
link-local document sources. Known cloud-metadata, unspecified, and multicast
destinations remain blocked. Restart the service after changing the setting.

## Linux Preview

The Linux template expects:

- `piqae-agent` and `piqae-executor-cups` in `/usr/libexec/piqae`, owned by
  root and not writable by the `piqae` account;
- the supplied sysusers and tmpfiles entries installed under their conventional
  `/usr/lib` locations and applied by `systemd-sysusers` and
  `systemd-tmpfiles`;
- `piqae-agent.env.example` copied to `/etc/piqae/agent.env`, edited, owned by
  `root:piqae`, and mode `0640` or stricter;
- the `piqae` account granted only the distribution-specific CUPS access it
  needs, commonly membership in `lp`, when required;
- `piqae-agent.service` copied to `/etc/systemd/system`, followed by
  `systemctl daemon-reload`, `systemctl enable --now piqae-agent`, and review of
  `journalctl -u piqae-agent`.

The unit uses a dedicated unprivileged account, a private state directory,
read-only system paths, an empty capability set, and restart-on-failure. Do not
weaken its sandbox globally to solve a printer-specific permission problem.

The Linux tray is a separate user-session process. It reads loopback status and
is not installed by this system service. `PIQAE_DASHBOARD_URL` may point it to
an explicit hosted/self-hosted dashboard; the loopback API is not a web UI.

## macOS Preview

Render all `@...@` values in
`com.piqae.agent.plist.in`; an unrendered template is invalid for
installation. Use an installation root and data directory owned by a dedicated
unprivileged account, with binaries not writable by that account. Validate the
rendered file with `plutil -lint`, then install it as
`/Library/LaunchDaemons/com.piqae.agent.plist`, owned by `root:wheel`
and mode `0644`, and load it with `launchctl bootstrap system`.

The menu-bar shell is a separate user application. It reads the authenticated
loopback API, shows status/printers/profiles, and hosts native profile capture.
It shows a dashboard action only when `PIQAE_DASHBOARD_URL` is an explicit
HTTP(S) URL. Source packaging/signing scripts do not constitute a notarised
release or a Supported signed update channel.

## Windows development installation

The unsigned Inno Setup package installs a per-user login process under
`%LOCALAPPDATA%\Programs\Piqae`; it does not register a Windows Service because
the agent is not an SCM service. Do not register it with `sc.exe`. See the
bundled `windows/README.md` for setup, one-time enrolment, ACL, uninstall, and
exact replay limitations. A testable installer does not change the Disabled
production support tier.

## Rollback and support data

Stop the service, preserve the configured data directory, restore the prior
versioned binaries/templates, and start it again. Never put device keys, bearer
tokens, URI credentials, document bytes, or the complete data directory into a
support bundle. The current source bundle has no automated updater or
support-bundle exporter.
