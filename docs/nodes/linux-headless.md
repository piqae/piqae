# Headless Linux node

**Release tier:** Preview on listed Ubuntu/Debian x86-64 platforms; Linux ARM64
is Preview without low-power hardware gates.

**Implemented in source:** CUPS printer discovery, PDF/RAW submission, durable
local queue, authenticated loopback API, process-supervised executor, and a
small optional status tray where desktop infrastructure exists.

Install CUPS and the distribution's CUPS development/runtime libraries. The
provided systemd templates run a dedicated `piqae` user with hardened service
settings:

```sh
sudo install -m 0755 piqae-agent piqae-executor-cups /usr/libexec/piqae/
sudo install -m 0644 packaging/linux/piqae-agent.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now piqae-agent
```

Render environment and state paths for the target host first; the templates are
not a signed distribution package. Confirm the service account can talk to CUPS
and read only the required agent configuration.

CUPS options and named instances can represent profiles. Vendor GUI-only
settings may need an administrator-created CUPS queue/instance. RAW jobs bypass
portable rendering options.

See [printers](../printing/printers.md),
[offline queues](../printing/offline-queues.md), and
[`operations/agent-service-installation.md`](../operations/agent-service-installation.md).
