# Headless Linux node

**Release tier:** Preview on listed Ubuntu/Debian x86-64 platforms; Linux ARM64
is Preview without low-power hardware gates.

**Implemented in source:** CUPS printer discovery, PDF/RAW submission, durable
local queue, authenticated loopback API, process-supervised executor, and a
small optional status tray where desktop infrastructure exists.

Cloud document-decryption keys remain in an owner-only local file on headless
Linux. Piqae deliberately does not depend on a desktop secret-service session
that may be absent during systemd startup. With the packaged
`PIQAE_DATA_DIR=/var/lib/piqae`, a private-key-free keyring manifest is stored
beside owner-only files named for each `cek_…` generation. Existing
`/var/lib/piqae/content-encryption.key` and connector-local
`device.content-encryption.key` files are migrated to a generation without
rotating the existing key. The manifest binds the keyring to the stable node
identity; if a manifest or any referenced generation is missing or corrupt,
startup fails closed instead of silently generating a replacement.
The device and content-encryption keys are private key material, not
configuration files. They must be regular files owned by `piqae:piqae`, be
readable by that account, have no group or other permission bits (the agent
creates them as mode `0600`), and reside on an encrypted host volume. The
content key contains private P-256 scalar material. TPM-backed non-exportable
key support remains a future hardening gate.

Install CUPS and the distribution's CUPS development/runtime libraries. Before
enrolment, install and apply the supplied sysusers definition so the dedicated
`piqae` service account exists; do not enrol as `root` or a login user. Treat a
failed `getent` check as a blocked installation:

```sh
sudo install -m 0644 packaging/linux/piqae-agent.sysusers /usr/lib/sysusers.d/piqae.conf
sudo systemd-sysusers /usr/lib/sysusers.d/piqae.conf
getent passwd piqae
getent group piqae
```

Distribution packaging may use the equivalent conventional sysusers path. The
provided systemd templates run the dedicated account with hardened service
settings:

```sh
sudo install -m 0755 piqae-agent piqae-executor-cups /usr/libexec/piqae/
sudo install -m 0644 packaging/linux/piqae-agent.service /etc/systemd/system/
sudo install -d -o piqae -g piqae -m 0700 /var/lib/piqae
sudo systemctl daemon-reload
sudo systemctl enable --now piqae-agent
```

The unit runs as `User=piqae`, `Group=piqae`, sets `UMask=0077`, and declares
`StateDirectory=piqae` with mode `0700`. Do not override it to run as a login
user or place `PIQAE_DATA_DIR` in a shared directory. After enrolment and after
every restore or deployment, audit the key files without printing their
contents:

```sh
sudo find /var/lib/piqae \
  \( -name 'device.key' -o -name '*content-encryption*.key' \) ! -type f -print
sudo find /var/lib/piqae -type f \
  \( -name 'device.key' -o -name '*content-encryption*.key' \) \
  -exec stat -c '%U:%G %a %n' {} +
sudo find /var/lib/piqae -type f \
  \( -name 'device.key' -o -name '*content-encryption*.key' \) \
  -perm /077 -print
sudo find /var/lib/piqae -type f \
  \( -name 'device.key' -o -name '*content-encryption*.key' \) \
  ! -exec sudo -u piqae test -r {} \; -print
```

The first, third, and fourth commands must produce no output. Every `stat` row
must report `piqae:piqae` and mode `600`. A key is legitimately absent before
that connector first starts. If an already-enrolled connector's expected key
is missing, not a regular file, unreadable by `piqae`, owned by another
account, or listed by the permissive-mode check, stop the unit and repair or
restore the key from an approved encrypted backup. Do not restart repeatedly
or delete connector state: automatically generating a replacement would make
outstanding encrypted jobs undecryptable or change the node identity. Never
display, copy into logs, or hash private-key contents during a routine check.

Render environment and state paths for the target host first; the templates are
not a signed distribution package. Confirm the service account can talk to CUPS
and read only the required agent configuration.

CUPS options and named instances can represent profiles. Vendor GUI-only
settings may need an administrator-created CUPS queue/instance. RAW jobs bypass
portable rendering options.

See [printers](../printing/printers.md),
[offline queues](../printing/offline-queues.md), and
[`operations/agent-service-installation.md`](../operations/agent-service-installation.md).
