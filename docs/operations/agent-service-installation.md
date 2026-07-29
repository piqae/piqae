# Agent service installation from a source bundle

The native archives are unsigned Preview/Disabled foundations, not installers.
Verify the archive SHA-256 sidecar and review its SPDX SBOM before installation.
Pin a release version and test printing, restart, and rollback on the target OS.

## URI source policy

Local and hosted URI jobs use the same agent-side fetch policy. Public HTTP(S)
is allowed by default; redirects, embedded URI credentials, cloud metadata,
unspecified/multicast addresses, and non-public destinations are rejected.

`SPOOL_ALLOW_PRIVATE_URI_SOURCES=false` is the secure default. A trusted
local/self-hosted operator may set it to `true` to reach LAN, loopback, or
link-local document sources. Known cloud-metadata, unspecified, and multicast
destinations remain blocked. Restart the service after changing the setting.

## Linux Preview

The Linux template expects:

- `spool-agent` and `spool-executor-cups` in `/usr/libexec/spool`, owned by
  root and not writable by the `spool` account;
- the supplied sysusers and tmpfiles entries installed under their conventional
  `/usr/lib` locations and applied by `systemd-sysusers` and
  `systemd-tmpfiles`;
- `spool-agent.env.example` copied to `/etc/spool/agent.env`, edited, owned by
  `root:spool`, and mode `0640` or stricter;
- the `spool` account granted only the distribution-specific CUPS access it
  needs, commonly membership in `lp`, when required;
- `spool-agent.service` copied to `/etc/systemd/system`, followed by
  `systemctl daemon-reload`, `systemctl enable --now spool-agent`, and review of
  `journalctl -u spool-agent`.

The unit uses a dedicated unprivileged account, a private state directory,
read-only system paths, an empty capability set, and restart-on-failure. Do not
weaken its sandbox globally to solve a printer-specific permission problem.

The Linux tray is a separate user-session process. It reads loopback status and
is not installed by this system service. `SPOOL_DASHBOARD_URL` may point it to
an explicit hosted/self-hosted dashboard; the loopback API is not a web UI.

## macOS Preview

Render all `@...@` values in
`com.c4coffee.spool.agent.plist.in`; an unrendered template is invalid for
installation. Use an installation root and data directory owned by a dedicated
unprivileged account, with binaries not writable by that account. Validate the
rendered file with `plutil -lint`, then install it as
`/Library/LaunchDaemons/com.c4coffee.spool.agent.plist`, owned by `root:wheel`
and mode `0644`, and load it with `launchctl bootstrap system`.

The menu-bar shell is a separate user application and currently cannot read
agent status. It shows a dashboard action only when `SPOOL_DASHBOARD_URL` is an
explicit HTTP(S) URL. There is no notarised package or signed update channel.

## Windows Disabled

There is no Windows Service template because the agent is not an SCM service.
Do not register it with `sc.exe`. See the bundled `windows/README.md` for the
exact missing gates. Interactive developer execution does not change the
Disabled support tier.

## Rollback and support data

Stop the service, preserve the configured data directory, restore the prior
versioned binaries/templates, and start it again. Never put device keys, bearer
tokens, URI credentials, document bytes, or the complete data directory into a
support bundle. The current source bundle has no automated updater or
support-bundle exporter.
