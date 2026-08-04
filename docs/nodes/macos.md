# macOS node

**Release tier:** Preview on macOS 13+ Apple silicon and macOS 15 Intel.
Local development artifacts are unsigned. Credentialed tagged Preview
artifacts are Developer ID signed, Apple notarised, and stapled; this does not
promote the platform to Supported without the remaining installation, update,
rollback, and printer evidence.

**Implemented in source:** CUPS discovery/submission, durable local queue,
authenticated loopback API, menu-bar status/printers/print presets, native PrintCore
profile capture, immutable edit/clone revisions, headless PrintCore replay, and
the verified `https://app.piqae.com/connect` link shape
invitation/consent flow for adding a tenant connector to an
existing installation. Connector enrollment requires an explicit printer
selection and an installation-key proof; the shell passes the capability to the
agent over bounded standard input rather than command-line arguments.
The legacy `piqae://connect` scheme is deprecated compatibility and is not
emitted for new sessions.

Cloud document-decryption keys are stored in the current user's macOS login
Keychain. P-256 key material is verified before a legacy file is removed. The agent refuses cloud
startup when Keychain access or migration fails; it does not silently replace a
key and strand queued encrypted jobs. This is OS-backed at-rest protection, not
a claim that the current P-256 key is non-exportable or Secure Enclave-backed.

**Tested:** automated Rust and Swift suites run in development. A real HP A4
print has been exercised during development, but that is not the formal
clean-install, packaging, driver-matrix, sleep/wake, and physical release gate.

Build the native menu app:

```sh
shells/macos/build-app.sh
open shells/macos/build/Piqae.app
```

The menu app connects only to the local loopback agent. Add Print Preset opens the
real macOS print panel and saves driver settings without printing. Edit restores
the exact prior PrintCore revision; Duplicate creates a new profile.

Complex vendor panes are captured as opaque native state. Piqae does not
recreate them in the web UI. Test every saved profile on its actual printer and
stock before publishing it.

Service installation remains Preview. Follow
[`operations/agent-service-installation.md`](../operations/agent-service-installation.md)
and [native profiles](../printing/native-profiles.md).

Embedded onboarding is also Preview. It is verified in source and automated
non-physical tests, but is not a Supported distribution path until a signed,
notarised artifact and clean-install/update/rollback evidence are published.
